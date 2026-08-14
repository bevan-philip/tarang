use std::error::Error;

use axum::{Json, extract::State};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::{
    database::{self, Article, Db, Feed, create_feed, list_articles_for_feed},
    feed::update_feed_articles,
};

pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

#[derive(Serialize)]
pub struct InitialState {
    #[serde(flatten)]
    feed: Feed,
    articles: Vec<Article>,
}

pub async fn get_initial_state(
    State(db): State<Db>,
) -> Result<Json<Vec<InitialState>>, StatusCode> {
    let feeds = database::list_feeds(&db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut feed_with_articles: Vec<InitialState> = Vec::new();
    for feed in feeds {
        let articles = list_articles_for_feed(&db, feed.pk, 10, 0)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        feed_with_articles.push(InitialState { feed, articles });
    }

    Ok(Json(feed_with_articles))
}

#[derive(Deserialize)]
pub struct AddSite {
    name: String,
    url: String,
    metadata: Option<String>,
    refresh_interval: Option<i64>,
}

pub async fn add_feed(
    State(db): State<Db>,
    Json(payload): Json<AddSite>,
) -> Result<StatusCode, StatusCode> {
    let feed = create_feed(
        &db,
        &payload.name,
        &payload.url,
        payload.metadata.as_deref(),
        payload.refresh_interval,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    update_feed_articles(&db, feed.pk, feed.url)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
        .unwrap();

    Ok(StatusCode::CREATED)
}
