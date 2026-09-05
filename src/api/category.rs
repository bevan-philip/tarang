use axum::{
    extract::{Path, State},
    http::StatusCode,
};
use axum_jsonschema::Json;
use schemars::JsonSchema;
use serde::Deserialize;

use super::{AppError, feed::PostFeedResp};
use crate::{
    AppState,
    database::{Category, create_category, drop_category, list_categories, rename_category},
};

pub async fn post_category(
    State(AppState { db, .. }): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<PostFeedResp>, AppError> {
    let category = create_category(&db, &name).await?;
    Ok(Json(PostFeedResp {
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

#[derive(Deserialize, JsonSchema)]
pub struct PatchCategoryReq {
    pub name: String,
}

pub async fn patch_category(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<PatchCategoryReq>,
) -> Result<Json<Category>, AppError> {
    let category = rename_category(&db, id, &payload.name).await?;
    Ok(Json(category))
}
