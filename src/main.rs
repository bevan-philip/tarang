use crate::api::{
    delete_category, delete_feed, get_category, get_feed, get_opml, get_summary, health,
    patch_feed, post_category, post_feed, upload_opml,
};
use crate::database::Db;
use axum::{
    Router,
    routing::{delete, get, post},
};
use tower_http::cors::CorsLayer;

mod api;
mod config;
mod database;
mod feed;
mod greader;
mod opml;
mod sync;

use config::Config;

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

    let config = Config::load().expect("failed to load config.toml");

    let db = database::config(&config.database.path, config.database.busy_timeout())
        .await
        .expect("failed to initialise db");

    let http = reqwest::Client::builder()
        .timeout(config.http.timeout())
        .build()
        .expect("failed to build http client");

    let sync_db = db.clone();
    let sync_http = http.clone();
    let sync_interval = config.sync.poll_interval();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(sync_interval);
        loop {
            if let Err(e) = sync::sync_feeds(&sync_db, &sync_http).await {
                tracing::error!(error = %e, "sync_feeds failed");
            }
            interval.tick().await;
        }
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/tarang/v1/summary", get(get_summary))
        .route("/tarang/v1/feed/{feed_id}", get(get_feed))
        .route("/tarang/v1/feed", post(post_feed))
        .route(
            "/tarang/v1/feed/{feed_id}",
            delete(delete_feed).patch(patch_feed),
        )
        .route("/tarang/v1/category", get(get_category))
        .route("/tarang/v1/category/{category_id}", post(post_category))
        .route("/tarang/v1/category/{category_id}", delete(delete_category))
        .route("/tarang/v1/opml", get(get_opml))
        .route("/tarang/v1/opml", post(upload_opml))
        .nest("/greader", greader::router())
        .with_state(AppState { db, http })
        .layer(CorsLayer::permissive());

    let bind_addr = config.server.bind_addr();

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .unwrap_or_else(|_| panic!("failed to bind to {bind_addr}"));

    println!("listening on http://{bind_addr}");
    axum::serve(listener, app).await.expect("server crashed");
}
