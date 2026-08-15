use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{
    AppState,
    database::{
        self, Article, Category, DbError, Feed, add_feed_to_category, create_category,
        create_feed, list_categories, list_categories_for_all_feeds, list_articles_for_feeds,
    },
    feed::{FeedError, update_feed_articles},
};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    Feed(#[from] FeedError),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::Db(DbError::AlreadyExists(msg)) => {
                tracing::warn!(error = %self, "request rejected");
                (StatusCode::CONFLICT, msg.clone())
            }
            other => {
                tracing::error!(error = %other, "request failed");
                (StatusCode::INTERNAL_SERVER_ERROR, other.to_string())
            }
        };

        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

#[derive(Serialize)]
pub struct InitialState {
    categories: Vec<Category>,
    feeds: Vec<FeedOutline>,
}

#[derive(Serialize)]
pub struct FeedOutline {
    #[serde(flatten)]
    feed: Feed,
    category: Vec<Category>,
    articles: Vec<Article>,
}

pub async fn get_app_state(
    State(AppState { db, .. }): State<AppState>,
) -> Result<Json<InitialState>, AppError> {
    let feeds = database::list_feeds(&db).await?;

    let categories = database::list_categories(&db).await?;

    let mut articles_by_feed: HashMap<i64, Vec<Article>> = HashMap::new();
    for article in list_articles_for_feeds(&db, 10).await? {
        articles_by_feed.entry(article.feed).or_default().push(article);
    }

    let mut categories_by_feed = list_categories_for_all_feeds(&db).await?;

    let feed_with_articles: Vec<FeedOutline> = feeds
        .into_iter()
        .map(|feed| FeedOutline {
            category: categories_by_feed.remove(&feed.pk).unwrap_or_default(),
            articles: articles_by_feed.remove(&feed.pk).unwrap_or_default(),
            feed,
        })
        .collect();

    Ok(Json(InitialState {
        categories,
        feeds: feed_with_articles,
    }))
}

#[derive(Deserialize)]
pub struct AddFeed {
    name: String,
    url: String,
    metadata: Option<String>,
    refresh_interval: Option<i64>,
}

#[derive(Serialize)]
pub struct AddFeedResp {
    name: String,
    id: i64,
}

pub async fn post_feed(
    State(AppState { db, http }): State<AppState>,
    Json(payload): Json<AddFeed>,
) -> Result<Json<AddFeedResp>, AppError> {
    let feed = create_feed(
        &db,
        &payload.name,
        &payload.url,
        payload.metadata.as_deref(),
        payload.refresh_interval,
    )
    .await?;

    update_feed_articles(&db, feed.pk, &feed.url, &http).await?;

    Ok(Json(AddFeedResp {
        name: payload.name,
        id: feed.pk,
    }))
}

pub async fn post_category_feed(
    State(AppState { db, .. }): State<AppState>,
    Path((category_id, feed_id)): Path<(i64, i64)>,
) -> Result<StatusCode, AppError> {
    add_feed_to_category(&db, feed_id, category_id).await?;
    Ok(StatusCode::CREATED)
}

pub async fn post_category(
    State(AppState { db, .. }): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<AddFeedResp>, AppError> {
    let category = create_category(&db, &name).await?;
    Ok(Json(AddFeedResp {
        name,
        id: category.pk,
    }))
}

pub async fn get_category(
    State(AppState { db, .. }): State<AppState>,
) -> Result<Json<Vec<Category>>, AppError> {
    Ok(Json(list_categories(&db).await?))
}
