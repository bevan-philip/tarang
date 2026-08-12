use crate::api::{get_initial_state, health};
use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

mod api;
mod database;
mod feed;
mod sync;

#[tokio::main]
async fn main() {
    let db = database::config().await.expect("failed to initialise db");

    let sync_db = db.clone();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        loop {
            if let Err(e) = sync::sync_feeds(&sync_db).await {
                eprintln!("sync_feeds failed: {e}");
            }
            interval.tick().await;
        }
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/tarang/v1/initialState", get(get_initial_state))
        .with_state(db);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();

    println!("listening on http://127.0.0.1:3000");
    axum::serve(listener, app).await.unwrap();
}
