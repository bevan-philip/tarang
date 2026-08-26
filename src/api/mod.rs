use axum::{
    Json,
    extract::multipart::MultipartError,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::str::Utf8Error;

use crate::{database::DbError, feed::FeedError, opml::OpmlError};

mod app_state;
mod category;
mod feed;
mod health;
mod opml;

pub use app_state::*;
pub use category::*;
pub use feed::*;
pub use health::*;
pub use opml::*;

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
