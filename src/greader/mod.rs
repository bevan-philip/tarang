use axum::{
    Router,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};

use crate::AppState;
use crate::database::DbError;
use crate::feed::FeedError;

pub mod form;
pub mod handlers;
pub mod ids;
pub mod item_id;
pub mod responses;

#[derive(Debug, thiserror::Error)]
pub enum GReaderError {
    #[error("{0}")]
    BadRequest(String),
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    Feed(#[from] FeedError),
}

impl IntoResponse for GReaderError {
    fn into_response(self) -> Response {
        match self {
            GReaderError::BadRequest(msg) => {
                tracing::warn!(error = %msg, "greader request rejected");
                (StatusCode::BAD_REQUEST, msg).into_response()
            }
            other => {
                tracing::error!(error = %other, "greader request failed");
                (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()).into_response()
            }
        }
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/accounts/ClientLogin", post(handlers::login::client_login))
        .route("/reader/api/0/token", get(handlers::login::token))
        .route("/reader/api/0/user-info", get(handlers::login::user_info))
        .route(
            "/reader/api/0/tag/list",
            get(handlers::subscriptions::tag_list),
        )
        .route(
            "/reader/api/0/subscription/list",
            get(handlers::subscriptions::subscription_list),
        )
        .route(
            "/reader/api/0/subscription/quickadd",
            post(handlers::subscriptions::subscription_quickadd),
        )
        .route(
            "/reader/api/0/subscription/edit",
            post(handlers::subscriptions::subscription_edit),
        )
        .route(
            "/reader/api/0/rename-tag",
            post(handlers::subscriptions::rename_tag),
        )
        .route(
            "/reader/api/0/disable-tag",
            post(handlers::subscriptions::disable_tag),
        )
        .route("/reader/api/0/edit-tag", post(handlers::edit::edit_tag))
        .route(
            "/reader/api/0/mark-all-as-read",
            post(handlers::edit::mark_all_as_read),
        )
        .route(
            "/reader/api/0/stream/items/ids",
            get(handlers::streams::stream_items_ids),
        )
        .route(
            "/reader/api/0/stream/items/contents",
            get(handlers::streams::stream_items_contents)
                .post(handlers::streams::stream_items_contents),
        )
        .route(
            "/reader/api/0/stream/contents/{*stream_id}",
            get(handlers::streams::stream_contents),
        )
        .route(
            "/reader/api/0/unread-count",
            get(handlers::streams::unread_count),
        )
        .fallback(handlers::catch_all)
}
