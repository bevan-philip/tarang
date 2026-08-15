use crate::api::{
    get_app_state, get_category, health, post_category, post_category_feed, post_feed,
};
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
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let db = database::config().await.expect("failed to initialise db");

    let sync_db = db.clone();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        loop {
            if let Err(e) = sync::sync_feeds(&sync_db).await {
                tracing::error!(error = %e, "sync_feeds failed");
            }
            interval.tick().await;
        }
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/tarang/v1/app", get(get_app_state))
        .route("/tarang/v1/feed", post(post_feed))
        .route("/tarang/v1/category/{name}", post(post_category))
        .route("/tarang/v1/category", get(get_category))
        .route(
            "/tarang/v1/category/{category_id}/feed/{feed_id}",
            post(post_category_feed),
        )
        .with_state(db);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .expect("failed to bind to 127.0.0.1:3000");

    println!("listening on http://127.0.0.1:3000");
    axum::serve(listener, app)
        .await
        .expect("server crashed");
}
