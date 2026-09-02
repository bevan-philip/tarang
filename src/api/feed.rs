use axum::{
    extract::{Path, State},
    http::StatusCode,
};
use axum_jsonschema::Json;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::AppError;
use crate::{
    AppState,
    database::{
        Article, DbError, Feed, StarredArticlesWithFeed, drop_feed, list_articles_for_feed,
        list_feed, list_starred_articles_with_feed, update_feed,
    },
    feed::create_feed_with_articles,
};

#[derive(Serialize, JsonSchema)]
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

#[derive(Deserialize, JsonSchema)]
pub struct PostFeedReq {
    pub name: String,
    pub url: String,
    pub category_id: Option<i64>,
    pub metadata: Option<String>,
    pub refresh_interval: Option<i64>,
}

#[derive(Serialize, JsonSchema)]
pub struct PostFeedResp {
    pub name: String,
    pub id: i64,
}

pub async fn post_feed(
    State(AppState { db, http }): State<AppState>,
    Json(payload): Json<PostFeedReq>,
) -> Result<Json<PostFeedResp>, AppError> {
    let feed = create_feed_with_articles(
        &db,
        &http,
        Some(&payload.name),
        &payload.url,
        payload.category_id,
        payload.metadata.as_deref(),
        payload.refresh_interval,
    )
    .await?;

    Ok(Json(PostFeedResp {
        name: feed.name,
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

#[derive(Deserialize, JsonSchema)]
pub struct PatchFeedReq {
    name: Option<String>,
    metadata: Option<String>,
    refresh_interval: Option<i64>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schemars(with = "Option<i64>")]
    category_id: Option<Option<i64>>,
}

pub async fn patch_feed(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<PatchFeedReq>,
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

pub async fn get_starred_articles(
    State(AppState { db, .. }): State<AppState>,
) -> Result<Json<Vec<StarredArticlesWithFeed>>, AppError> {
    let articles = list_starred_articles_with_feed(&db).await?;
    Ok(Json(articles))
}
