use crate::api::{
    delete_category, delete_feed, delete_filter, get_article, get_category,
    get_export_starred_articles, get_feed, get_filter, get_opml, get_starred_articles,
    get_summary, health, patch_category, patch_feed, patch_filter, post_category, post_feed,
    post_filter, upload_opml,
};
use crate::database::Db;
use aide::{
    axum::{
        ApiRouter,
        routing::{delete_with, get_with, patch_with, post_with},
    },
    openapi::OpenApi,
    swagger::Swagger,
};
use axum::{Json, extract::Extension, routing::get};
use std::sync::Arc;
use tower_http::{compression::CompressionLayer, cors::CorsLayer};

mod api;
mod backup;
mod config;
mod database;
mod feed;
mod filter;
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

    let backup_enabled = config.backup.enabled;
    let backup_every_n_polls = config.backup.every_n_polls;
    let backup_path = config.backup.path.clone();
    let backup_db = db.clone();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(sync_interval);
        let mut polls_since_backup: u32 = 0;
        loop {
            if let Err(e) = sync::sync_feeds(&sync_db, &sync_http).await {
                tracing::error!(error = %e, "sync_feeds failed");
            }

            if backup_enabled && backup_every_n_polls > 0 {
                polls_since_backup += 1;
                if polls_since_backup >= backup_every_n_polls {
                    polls_since_backup = 0;
                    if let Err(e) = backup::backup_database(&backup_db, &backup_path).await {
                        tracing::error!(error = %e, "backup_database failed");
                    }
                }
            }

            interval.tick().await;
        }
    });

    let mut api = OpenApi::default();

    let documented = ApiRouter::new()
        .api_route("/health", get_with(health, |op| op.summary("Health check")))
        .api_route(
            "/tarang/v1/summary",
            get_with(get_summary, |op| {
                op.summary("Get a summary of categories, feeds, and their recent articles")
            }),
        )
        .api_route(
            "/tarang/v1/feed/{feed_id}",
            get_with(get_feed, |op| op.summary("Get a feed and its articles")),
        )
        .api_route(
            "/tarang/v1/article/{article_id}",
            get_with(get_article, |op| {
                op.summary("Get a single article with full content")
            }),
        )
        .api_route(
            "/tarang/v1/feed",
            post_with(post_feed, |op| op.summary("Add a new feed")),
        )
        .api_route(
            "/tarang/v1/feed/{feed_id}",
            delete_with(delete_feed, |op| op.summary("Delete a feed"))
                .patch_with(patch_feed, |op| op.summary("Update a feed")),
        )
        .api_route(
            "/tarang/v1/category",
            get_with(get_category, |op| op.summary("List categories")),
        )
        .api_route(
            "/tarang/v1/category/{category_id}",
            post_with(post_category, |op| {
                op.summary("Create a category")
                    .description("The path segment is the category's name, not an id")
            }),
        )
        .api_route(
            "/tarang/v1/category/{category_id}",
            delete_with(delete_category, |op| op.summary("Delete a category"))
                .patch_with(patch_category, |op| op.summary("Rename a category")),
        )
        .api_route(
            "/tarang/v1/filter",
            get_with(get_filter, |op| op.summary("List filters"))
                .post_with(post_filter, |op| op.summary("Create a filter")),
        )
        .api_route(
            "/tarang/v1/filter/{id}",
            patch_with(patch_filter, |op| op.summary("Update a filter"))
                .delete_with(delete_filter, |op| op.summary("Delete a filter")),
        )
        .api_route(
            "/tarang/v1/starred",
            get_with(get_starred_articles, |op| {
                op.summary("List starred articles with feed info")
            }),
        )
        .api_route(
            "/tarang/v1/export/opml",
            get_with(get_opml, |op| {
                op.summary("Export feeds as OPML").description(
                    "Returns OPML/XML content; documented as text/plain due to axum's \
                     IntoResponse impl for String",
                )
            }),
        )
        .api_route(
            "/tarang/v1/export/opml",
            post_with(upload_opml, |op| {
                op.summary("Import feeds from an OPML file").description(
                    "Accepts multipart/form-data with a single file field; documented as \
                     generic multipart, not field-specific",
                )
            }),
        )
        .api_route(
            "/tarang/v1/export/starred",
            get_with(get_export_starred_articles, |op| {
                op.summary("Export starred articles' url and content")
            }),
        )
        .route("/api.json", get(serve_api))
        .route("/docs", Swagger::new("/api.json").axum_route())
        .finish_api(&mut api);

    let app = documented
        .nest("/greader", greader::router())
        .layer(Extension(Arc::new(api)))
        .with_state(AppState { db, http })
        .layer(CorsLayer::permissive())
        .layer(CompressionLayer::new());

    let bind_addr = config.server.bind_addr();

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .unwrap_or_else(|_| panic!("failed to bind to {bind_addr}"));

    println!("listening on http://{bind_addr}");
    axum::serve(listener, app).await.expect("server crashed");
}

async fn serve_api(Extension(api): Extension<Arc<OpenApi>>) -> Json<OpenApi> {
    Json((*api).clone())
}
