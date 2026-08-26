use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Deserializer, Serialize};

use super::AppError;
use crate::{
    AppState,
    database::{
        self, Article, DbError, Feed, create_feed, drop_feed, list_articles_for_feed, list_feed,
        update_feed,
    },
    feed::get_feed_articles,
};

#[derive(Serialize)]
pub struct GetFeed {
    id: i64,
    feed: Feed,
    articles: Vec<Article>,
}

pub async fn get_feed(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<GetFeed>, AppError> {
    let feed = list_feed(&db, id)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("feed {id} not found")))?;
    let articles = list_articles_for_feed(&db, id).await?;

    Ok(Json(GetFeed { id, feed, articles }))
}

#[derive(Deserialize)]
pub struct AddFeed {
    pub name: String,
    pub url: String,
    pub category_id: Option<i64>,
    pub metadata: Option<String>,
    pub refresh_interval: Option<i64>,
}

#[derive(Serialize)]
pub struct AddFeedResp {
    pub name: String,
    pub id: i64,
}

pub async fn post_feed(
    State(AppState { db, http }): State<AppState>,
    Json(payload): Json<AddFeed>,
) -> Result<Json<AddFeedResp>, AppError> {
    // Retrieve the articles first to see if the URL is valid.
    let articles = get_feed_articles(&http, &payload.url).await?;

    let feed = create_feed(
        &db,
        &payload.name,
        &payload.url,
        payload.category_id,
        payload.metadata.as_deref(),
        payload.refresh_interval,
    )
    .await?;

    database::create_articles(&db, feed.pk, &articles).await?;

    Ok(Json(AddFeedResp {
        name: payload.name,
        id: feed.pk,
    }))
}

pub async fn delete_feed(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    drop_feed(&db, id).await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
pub struct PatchFeed {
    name: Option<String>,
    metadata: Option<String>,
    refresh_interval: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_some")]
    category_id: Option<Option<i64>>,
}

fn deserialize_some<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}

pub async fn patch_feed(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<PatchFeed>,
) -> Result<Json<Feed>, AppError> {
    let feed = update_feed(
        &db,
        id,
        payload.name.as_deref(),
        payload.metadata.as_deref(),
        payload.refresh_interval,
        payload.category_id,
    )
    .await?;

    Ok(Json(feed))
}
