use axum::{
    Json,
    extract::{Multipart, Path, State, multipart::MultipartError},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Deserializer, Serialize};
use std::{collections::HashMap, str::Utf8Error};

use crate::{
    AppState,
    database::{
        self, Article, Category, DbError, Feed, create_category, create_feed, drop_category,
        drop_feed, list_articles_for_feeds, list_categories, update_feed,
    },
    feed::{FeedError, get_feed_articles},
    opml::{self, OpmlError},
};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    Feed(#[from] FeedError),
    #[error(transparent)]
    Upload(#[from] MultipartError),
    #[error(transparent)]
    UploadBytes(#[from] Utf8Error),
    #[error(transparent)]
    ParseOpml(#[from] OpmlError),
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
    category: Option<Category>,
    articles: Vec<Article>,
}

pub async fn get_app_state(
    State(AppState { db, .. }): State<AppState>,
) -> Result<Json<InitialState>, AppError> {
    let feeds = database::list_feeds(&db).await?;

    let categories = database::list_categories(&db).await?;

    let mut articles_by_feed: HashMap<i64, Vec<Article>> = HashMap::new();
    for article in list_articles_for_feeds(&db, 10).await? {
        articles_by_feed
            .entry(article.feed)
            .or_default()
            .push(article);
    }

    let category_by_pk: HashMap<i64, Category> =
        categories.iter().map(|c| (c.pk, c.clone())).collect();

    let feed_with_articles: Vec<FeedOutline> = feeds
        .into_iter()
        .map(|feed| FeedOutline {
            category: feed
                .category
                .and_then(|pk| category_by_pk.get(&pk).cloned()),
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
    category_id: Option<i64>,
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

pub async fn delete_category(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    drop_category(&db, id).await?;

    Ok(StatusCode::OK)
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

pub async fn upload_opml(
    State(AppState { db, .. }): State<AppState>,
    mut multipart: Multipart,
) -> Result<StatusCode, AppError> {
    while let Some(field) = multipart.next_field().await? {
        let opml_bytes = field.bytes().await?;
        let opml = str::from_utf8(&opml_bytes)?;

        let feeds = opml::parse_opml(opml).await?;
        let categories = database::list_categories(&db).await?;

        let mut category_map: HashMap<String, i64> =
            categories.into_iter().map(|c| (c.name, c.pk)).collect();

        for feed in feeds {
            let Some(category) = feed.category.as_deref() else {
                database::create_feed(&db, &feed.name, &feed.url, None, None, None).await?;

                continue;
            };

            if !category_map.contains_key(category) {
                let new_category = database::create_category(&db, category).await?;
                category_map.insert(new_category.name, new_category.pk);
            }
            database::create_feed(
                &db,
                &feed.name,
                &feed.url,
                Some(category_map[category]),
                Some(&String::from("")),
                Some(360),
            )
            .await?;
        }
    }

    Ok(StatusCode::CREATED)
}
