use axum::{
    Json,
    extract::multipart::MultipartError,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::str::Utf8Error;

use crate::{database::DbError, feed::FeedError, opml::OpmlError};

mod article;
mod category;
mod export;
mod feed;
mod filter;
mod health;
mod summary;

pub use article::*;
pub use category::*;
pub use export::*;
pub use feed::*;
pub use filter::*;
pub use health::*;
pub use summary::*;

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
    #[error(transparent)]
    InvalidFilterPattern(#[from] regex::Error),
}

// aide's blanket `OperationOutput for Result<T, E>` impl requires `E:
// OperationOutput` as well as `T`, even though we don't want AppError
// documented as a response - the default trait methods are no-ops, so this
// satisfies the bound without adding anything to the generated spec.
impl aide::OperationOutput for AppError {
    type Inner = Self;
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::Db(DbError::NotFound(msg)) => {
                tracing::warn!(error = %self, "request rejected");
                (StatusCode::NOT_FOUND, msg.clone())
            }
            AppError::Db(DbError::AlreadyExists(msg)) => {
                tracing::warn!(error = %self, "request rejected");
                (StatusCode::CONFLICT, msg.clone())
            }
            AppError::InvalidFilterPattern(_) => {
                tracing::warn!(error = %self, "request rejected");
                (StatusCode::BAD_REQUEST, self.to_string())
            }
            other => {
                tracing::error!(error = %other, "request failed");
                (StatusCode::INTERNAL_SERVER_ERROR, other.to_string())
            }
        };

        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}
