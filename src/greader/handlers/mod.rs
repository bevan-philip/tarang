pub mod edit;
pub mod login;
pub mod streams;
pub mod subscriptions;

use axum::Json;

use super::GReaderError;
use super::ids::FeedRef;
use crate::database::{Db, get_feed_by_url};

pub async fn catch_all() -> Json<serde_json::Value> {
    Json(serde_json::json!([]))
}

pub(crate) async fn resolve_feed_pk(db: &Db, feed_ref: &FeedRef) -> Result<i64, GReaderError> {
    match feed_ref {
        FeedRef::Pk(pk) => Ok(*pk),
        FeedRef::Url(url) => {
            let feed = get_feed_by_url(db, url)
                .await?
                .ok_or_else(|| GReaderError::BadRequest(format!("unknown feed url: {url}")))?;
            Ok(feed.pk)
        }
    }
}

/// Extracts label names from a set of Google Reader tag values
/// (`user/-/label/<name>`), discarding anything that isn't a label
/// (e.g. the read/starred state tags, which `edit-tag` handles separately).
pub(crate) fn label_names(values: &[String]) -> Vec<String> {
    values
        .iter()
        .filter_map(|v| v.strip_prefix("user/-/label/").map(|s| s.to_string()))
        .collect()
}
