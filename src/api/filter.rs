use axum::{
    extract::{Path, State},
    http::StatusCode,
};
use axum_jsonschema::Json;
use schemars::JsonSchema;
use serde::Deserialize;

use super::AppError;
use crate::{
    AppState,
    database::{self, DbError, Filter},
    filter::{self, MatchType},
};

#[derive(Deserialize, JsonSchema)]
pub struct PostFilterReq {
    pub name: String,
    #[serde(default)]
    pub field: Option<String>,
    pub match_type: String,
    pub pattern: String,
}

pub async fn post_filter(
    State(AppState { db, .. }): State<AppState>,
    Json(payload): Json<PostFilterReq>,
) -> Result<Json<Filter>, AppError> {
    let match_type = parse_match_type(&payload.match_type);
    // Validated here so create_filter's compile_filter().expect() cannot
    // observe an invalid pattern.
    filter::validate_pattern(match_type, &payload.pattern)?;

    let field = payload.field.as_deref().unwrap_or("both");
    let created = database::create_filter(
        &db,
        &payload.name,
        field,
        &payload.match_type,
        &payload.pattern,
    )
    .await?;

    Ok(Json(created))
}

pub async fn get_filter(
    State(AppState { db, .. }): State<AppState>,
) -> Result<Json<Vec<Filter>>, AppError> {
    Ok(Json(database::list_filters(&db).await?))
}

#[derive(Deserialize, JsonSchema)]
pub struct PatchFilterReq {
    pub name: Option<String>,
    pub field: Option<String>,
    pub match_type: Option<String>,
    pub pattern: Option<String>,
    pub enabled: Option<bool>,
}

pub async fn patch_filter(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<PatchFilterReq>,
) -> Result<Json<Filter>, AppError> {
    if let Some(pattern) = payload.pattern.as_deref() {
        // A client tweaking the pattern shouldn't have to resend
        // match_type - fall back to the row's existing value.
        let match_type = match &payload.match_type {
            Some(mt) => mt.clone(),
            None => {
                let existing = database::get_filter(&db, id)
                    .await?
                    .ok_or_else(|| DbError::NotFound(format!("filter {id} not found")))?;
                existing.match_type
            }
        };
        filter::validate_pattern(parse_match_type(&match_type), pattern)?;
    }

    let updated = database::update_filter(
        &db,
        id,
        payload.name.as_deref(),
        payload.field.as_deref(),
        payload.match_type.as_deref(),
        payload.pattern.as_deref(),
        payload.enabled,
    )
    .await?;

    Ok(Json(updated))
}

pub async fn delete_filter(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    database::drop_filter(&db, id).await?;
    Ok(StatusCode::OK)
}

/// Mirrors `filter::compile_filter`'s leniency: an unrecognized match_type
/// string falls back to Contains here (so it's always safe to validate),
/// and is rejected by the `filter` table's CHECK constraint at insert time.
fn parse_match_type(raw: &str) -> MatchType {
    match raw {
        "regex" => MatchType::Regex,
        _ => MatchType::Contains,
    }
}
