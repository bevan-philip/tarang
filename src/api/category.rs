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
    database::{Category, create_category, drop_category, list_categories, rename_category},
};

#[derive(Serialize, JsonSchema)]
pub struct PostCategoryResp {
    pub name: String,
    pub id: i64,
}

pub async fn post_category(
    State(AppState { db, .. }): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<PostCategoryResp>, AppError> {
    let category = create_category(&db, &name).await?;
    Ok(Json(PostCategoryResp {
        name,
        id: category.pk,
    }))
}

pub async fn get_category(
    State(AppState { db, .. }): State<AppState>,
) -> Result<Json<Vec<Category>>, AppError> {
    Ok(Json(
        list_categories(&db, crate::database::FeedScope::All).await?,
    ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Db;

    async fn test_state() -> AppState {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        AppState {
            db: Db {
                read: pool.clone(),
                write: pool,
            },
            http: reqwest::Client::new(),
            discovery: Default::default(),
        }
    }

    #[tokio::test]
    async fn post_category_creates_and_returns_it() {
        let state = test_state().await;
        let Json(created) = post_category(State(state.clone()), Path("News".to_string()))
            .await
            .unwrap();
        assert_eq!(created.name, "News");

        let categories =
            crate::database::list_categories(&state.db, crate::database::FeedScope::All)
                .await
                .unwrap();
        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].pk, created.id);
    }

    #[tokio::test]
    async fn get_category_lists_all() {
        let state = test_state().await;
        post_category(State(state.clone()), Path("News".to_string()))
            .await
            .unwrap();

        let Json(categories) = get_category(State(state.clone())).await.unwrap();
        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].name, "News");
    }

    #[tokio::test]
    async fn delete_category_removes_it() {
        let state = test_state().await;
        let Json(created) = post_category(State(state.clone()), Path("News".to_string()))
            .await
            .unwrap();

        let status = delete_category(State(state.clone()), Path(created.id))
            .await
            .unwrap();
        assert_eq!(status, StatusCode::OK);

        let Json(categories) = get_category(State(state.clone())).await.unwrap();
        assert!(categories.is_empty());
    }

    #[tokio::test]
    async fn patch_category_renames_it() {
        let state = test_state().await;
        let Json(created) = post_category(State(state.clone()), Path("News".to_string()))
            .await
            .unwrap();

        let Json(updated) = patch_category(
            State(state.clone()),
            Path(created.id),
            Json(PatchCategoryReq {
                name: "Tech".to_string(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(updated.name, "Tech");
    }
}
