use axum::extract::State;

use super::resolve_feed_pk;
use crate::AppState;
use crate::database::{
    get_category_by_name, mark_all_read_for_category, mark_all_read_for_feed, mark_all_read_global,
    mark_articles_read, mark_articles_starred,
};
use crate::greader::GReaderError;
use crate::greader::commands::{self, TagEditPlan};
use crate::greader::form::MergedParams;
use crate::greader::ids::StreamId;
use crate::greader::item_id::parse_item_id;

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

    let TagEditPlan { read, starred } =
        commands::resolve_tag_edit(params.get_all("a"), params.get_all("r"))?;

    if let Some(is_read) = read {
        mark_articles_read(&db, &pks, is_read).await?;
    }

    if let Some(is_starred) = starred {
        mark_articles_starred(&db, &pks, is_starred).await?;
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

    let before_ts = commands::parse_mark_all_cutoff(params.get("ts"));

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{Db, ParsedArticle, create_articles, create_feed};
    use chrono::Utc;

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
        }
    }

    #[tokio::test]
    async fn edit_tag_ignores_nonexistent_pk_in_batch() {
        let state = test_state().await;
        let feed = create_feed(
            &state.db,
            "Feed",
            "https://example.com/feed",
            "",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();
        let articles = create_articles(
            &state.db,
            feed.pk,
            &[ParsedArticle {
                url: "https://example.com/article".into(),
                guid: "guid-1".into(),
                title: Some("Title".into()),
                content: "content".into(),
                summary: None,
                published_at: Utc::now(),
            }],
            &[],
        )
        .await
        .unwrap();
        let valid_pk = articles[0].pk;
        let nonexistent_pk = valid_pk + 1000;

        let query = format!("i={valid_pk}&i={nonexistent_pk}&a=user/-/state/com.google/read");
        let params = MergedParams::from_query(&query);

        let result = edit_tag(State(state.clone()), params).await;
        assert!(result.is_ok());

        let stored = sqlx::query_scalar!(
            "SELECT is_read FROM article_state WHERE article = ?",
            valid_pk
        )
        .fetch_one(&state.db.read)
        .await
        .unwrap();
        assert_eq!(stored, 1);
    }
}
