use axum::extract::{Path, State};
use axum_jsonschema::Json;

use super::AppError;
use crate::{
    AppState,
    database::{ArticleWithState, DbError, list_articles_by_pks},
};

pub async fn get_article(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<ArticleWithState>, AppError> {
    let article = list_articles_by_pks(&db, &[id])
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| DbError::NotFound(format!("article {id} not found")))?;

    Ok(Json(article))
}
