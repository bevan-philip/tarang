use axum::{
    extract::{Multipart, State},
    http::StatusCode,
};
use axum_jsonschema::Json;

use super::AppError;
use crate::{
    AppState,
    database::{FeedScope, StarredArticles, list_categories, list_feeds, list_starred_articles},
    feed,
    opml::{self, export_opml},
};

pub async fn upload_opml(
    State(AppState { db, http }): State<AppState>,
    mut multipart: Multipart,
) -> Result<StatusCode, AppError> {
    while let Some(field) = multipart.next_field().await? {
        let opml_bytes = field.bytes().await?;
        let opml_str = str::from_utf8(&opml_bytes)?;
        let feeds = opml::parse_opml(opml_str)?;
        feed::import_opml_feeds(&db, &http, feeds).await?;
    }

    Ok(StatusCode::CREATED)
}

pub async fn get_opml(State(AppState { db, .. }): State<AppState>) -> Result<String, AppError> {
    let feeds = list_feeds(&db, FeedScope::All).await?;
    let categories = list_categories(&db, FeedScope::All).await?;

    Ok(export_opml(feeds, categories)?)
}

pub async fn get_export_starred_articles(
    State(AppState { db, .. }): State<AppState>,
) -> Result<Json<Vec<StarredArticles>>, AppError> {
    let articles = list_starred_articles(&db).await?;

    Ok(Json(articles))
}
