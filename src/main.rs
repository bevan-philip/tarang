use crate::api::{
    delete_category, delete_feed, get_app_state, get_category, health, post_category,
    post_category_feed, post_feed,
};
use crate::database::Db;
use axum::{
    Router,
    routing::{delete, get, post},
};
use std::time::Duration;

mod api;
mod database;
mod feed;
mod sync;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub http: reqwest::Client,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let db = database::config().await.expect("failed to initialise db");

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("failed to build http client");

    let sync_db = db.clone();
    let sync_http = http.clone();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        loop {
            if let Err(e) = sync::sync_feeds(&sync_db, &sync_http).await {
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
        .route("/tarang/v1/feed/{feed_id}", delete(delete_feed))
        .route("/tarang/v1/category/{category_id}", delete(delete_category))
        .with_state(AppState { db, http });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .expect("failed to bind to 127.0.0.1:3000");

    println!("listening on http://127.0.0.1:3000");
    axum::serve(listener, app).await.expect("server crashed");
}
