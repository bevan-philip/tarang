use axum::extract::State;

use super::resolve_feed_pk;
use crate::AppState;
use crate::database::{
    get_category_by_name, mark_all_read_for_category, mark_all_read_for_feed, mark_all_read_global,
    mark_articles_read, mark_articles_starred,
};
use crate::greader::GReaderError;
use crate::greader::form::MergedParams;
use crate::greader::ids::StreamId;
use crate::greader::item_id::parse_item_id;

const READ_TAG: &str = "user/-/state/com.google/read";
const STARRED_TAG: &str = "user/-/state/com.google/starred";

pub async fn edit_tag(
    State(AppState { db, .. }): State<AppState>,
    params: MergedParams,
) -> Result<&'static str, GReaderError> {
    let pks: Result<Vec<i64>, GReaderError> = params
        .get_all("i")
        .iter()
        .map(|s| parse_item_id(s))
        .collect();
    let pks = pks?;

    let add = params.get_all("a");
    let remove = params.get_all("r");

    let add_read = add.iter().any(|s| s == READ_TAG);
    let remove_read = remove.iter().any(|s| s == READ_TAG);
    let add_starred = add.iter().any(|s| s == STARRED_TAG);
    let remove_starred = remove.iter().any(|s| s == STARRED_TAG);

    if add_read && remove_read {
        return Err(GReaderError::BadRequest(
            "conflicting edit: both adds and removes the read tag".into(),
        ));
    }
    if add_starred && remove_starred {
        return Err(GReaderError::BadRequest(
            "conflicting edit: both adds and removes the starred tag".into(),
        ));
    }

    if add_read {
        mark_articles_read(&db, &pks, true).await?;
    } else if remove_read {
        mark_articles_read(&db, &pks, false).await?;
    }

    if add_starred {
        mark_articles_starred(&db, &pks, true).await?;
    } else if remove_starred {
        mark_articles_starred(&db, &pks, false).await?;
    }

    Ok("OK")
}

pub async fn mark_all_as_read(
    State(AppState { db, .. }): State<AppState>,
    params: MergedParams,
) -> Result<&'static str, GReaderError> {
    let s = params
        .get("s")
        .ok_or_else(|| GReaderError::BadRequest("missing s".into()))?;
    let stream = StreamId::parse(s)?;

    // `ts` is a microsecond-epoch cutoff; absent or explicit 0 both mean
    // "no cutoff" (mark everything), matching how real clients send it.
    let before_ts = match params.get("ts").and_then(|v| v.parse::<i64>().ok()) {
        Some(0) | None => i64::MAX,
        Some(us) => us / 1_000_000,
    };

    match stream {
        StreamId::ReadingList => mark_all_read_global(&db, before_ts).await?,
        StreamId::Feed(feed_ref) => {
            let pk = resolve_feed_pk(&db, &feed_ref).await?;
            mark_all_read_for_feed(&db, pk, before_ts).await?;
        }
        StreamId::Label(name) => {
            let category = get_category_by_name(&db, &name)
                .await?
                .ok_or_else(|| GReaderError::BadRequest(format!("unknown label: {name}")))?;
            mark_all_read_for_category(&db, category.pk, before_ts).await?;
        }
        _ => {
            return Err(GReaderError::BadRequest(
                "unsupported stream for mark-all-as-read".into(),
            ));
        }
    }

    Ok("OK")
}
