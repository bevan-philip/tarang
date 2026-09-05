use axum::extract::{Path, State};
use axum_jsonschema::Json;
use schemars::JsonSchema;
use serde::Deserialize;

use super::AppError;
use crate::{
    AppState,
    database::{
        ArticleState, ArticleWithState, DbError, list_articles_by_pks, update_article_state,
    },
};

pub async fn get_article(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<ArticleWithState>, AppError> {
    let article = list_articles_by_pks(&db, &[id], crate::database::FeedScope::All)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| DbError::NotFound(format!("article {id} not found")))?;

    Ok(Json(article))
}

#[derive(Deserialize, JsonSchema)]
pub struct PatchArticleReq {
    /// Omitted or null leaves the current read state unchanged.
    pub is_read: Option<bool>,
    /// Omitted or null leaves the current starred state unchanged.
    pub is_starred: Option<bool>,
}

pub async fn patch_article(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<PatchArticleReq>,
) -> Result<Json<ArticleState>, AppError> {
    Ok(Json(
        update_article_state(&db, id, payload.is_read, payload.is_starred).await?,
    ))
}
