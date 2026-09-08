use axum::{
    extract::{Path, State},
    http::StatusCode,
};
use axum_jsonschema::Json;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::AppError;
use crate::{
    AppState,
    database::{self, Article, DbError, Filter},
    filter,
};

#[derive(Serialize, JsonSchema)]
pub struct FilterWithMatches {
    #[serde(flatten)]
    #[schemars(flatten)]
    pub filter: Filter,
    pub feeds: Vec<i64>,
    pub matched_articles: Vec<Article>,
}

#[derive(Serialize, JsonSchema)]
pub struct FilterWithFeeds {
    #[serde(flatten)]
    #[schemars(flatten)]
    pub filter: Filter,
    pub feeds: Vec<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct PostFilterReq {
    pub name: String,
    #[serde(default)]
    pub field: Option<String>,
    pub match_type: String,
    pub pattern: String,
    #[serde(default)]
    pub feeds: Option<Vec<i64>>,
}

pub async fn post_filter(
    State(AppState { db, .. }): State<AppState>,
    Json(payload): Json<PostFilterReq>,
) -> Result<Json<FilterWithMatches>, AppError> {
    // Validated here so create_filter's compile_filter() cannot observe an
    // invalid pattern.
    filter::validate_effective_rule(None, Some(&payload.match_type), Some(&payload.pattern))?;

    let field = payload.field.as_deref().unwrap_or("both");
    let feed_pks = payload.feeds.as_deref().unwrap_or(&[]);
    let created = database::create_filter(
        &db,
        &payload.name,
        field,
        &payload.match_type,
        &payload.pattern,
        feed_pks,
    )
    .await?;
    let feeds = database::list_filter_feed_pks(&db, created.pk).await?;
    let matched_articles = database::list_articles_matched_by_filter(&db, created.pk).await?;

    Ok(Json(FilterWithMatches {
        filter: created,
        feeds,
        matched_articles,
    }))
}

pub async fn get_filter(
    State(AppState { db, .. }): State<AppState>,
) -> Result<Json<Vec<FilterWithFeeds>>, AppError> {
    let filters = database::list_filters(&db).await?;
    let pairs = database::list_all_filter_feed_pairs(&db).await?;

    let mut feeds_by_filter: HashMap<i64, Vec<i64>> = HashMap::new();
    for (filter_pk, feed_pk) in pairs {
        feeds_by_filter.entry(filter_pk).or_default().push(feed_pk);
    }

    let result = filters
        .into_iter()
        .map(|filter| {
            let feeds = feeds_by_filter.remove(&filter.pk).unwrap_or_default();
            FilterWithFeeds { filter, feeds }
        })
        .collect();

    Ok(Json(result))
}

#[derive(Deserialize, JsonSchema)]
pub struct PatchFilterReq {
    pub name: Option<String>,
    pub field: Option<String>,
    pub match_type: Option<String>,
    pub pattern: Option<String>,
    pub enabled: Option<bool>,
    #[serde(default)]
    pub feeds: Option<Vec<i64>>,
}

pub async fn patch_filter(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<PatchFilterReq>,
) -> Result<Json<FilterWithMatches>, AppError> {
    if payload.match_type.is_some() || payload.pattern.is_some() {
        // A client tweaking the pattern shouldn't have to resend
        // match_type - fall back to the row's existing value.
        let existing = database::get_filter(&db, id)
            .await?
            .ok_or_else(|| DbError::NotFound(format!("filter {id} not found")))?;
        filter::validate_effective_rule(
            Some(&existing),
            payload.match_type.as_deref(),
            payload.pattern.as_deref(),
        )?;
    }

    let updated = database::update_filter(
        &db,
        id,
        database::UpdateFilterFields {
            name: payload.name,
            field: payload.field,
            match_type: payload.match_type,
            pattern: payload.pattern,
            enabled: payload.enabled,
            feed_pks: payload.feeds,
        },
    )
    .await?;
    let feeds = database::list_filter_feed_pks(&db, updated.pk).await?;
    let matched_articles = database::list_articles_matched_by_filter(&db, updated.pk).await?;

    Ok(Json(FilterWithMatches {
        filter: updated,
        feeds,
        matched_articles,
    }))
}

pub async fn delete_filter(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    database::drop_filter(&db, id).await?;
    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{Db, list_article_previews_for_feed};

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

    async fn seed_feeds_and_matching_articles(state: &AppState) {
        sqlx::raw_sql(
            "INSERT INTO feed (pk, name, url, greader_hidden) VALUES
                (1, 'Feed A', 'https://example.com/a', 0),
                (2, 'Feed B', 'https://example.com/b', 0);",
        )
        .execute(&state.db.write)
        .await
        .unwrap();

        for (pk, feed) in [(100, 1), (101, 2)] {
            sqlx::query(
                "INSERT INTO article
                    (pk, feed, url, guid, title, content, published_at, retrieved_at)
                 VALUES (?, ?, ?, ?, 'Crypto news', '', 1000, 2000)",
            )
            .bind(pk)
            .bind(feed)
            .bind(format!("https://example.com/article/{pk}"))
            .bind(format!("guid-{pk}"))
            .execute(&state.db.write)
            .await
            .unwrap();
        }
    }

    fn post_req(feeds: Option<Vec<i64>>) -> PostFilterReq {
        PostFilterReq {
            name: "Crypto".to_string(),
            field: None,
            match_type: "contains".to_string(),
            pattern: "crypto".to_string(),
            feeds,
        }
    }

    #[tokio::test]
    async fn filter_scoped_to_feed_does_not_hide_matches_on_other_feeds() {
        let state = test_state().await;
        seed_feeds_and_matching_articles(&state).await;

        let Json(created) = post_filter(State(state.clone()), Json(post_req(Some(vec![1]))))
            .await
            .unwrap();
        assert_eq!(created.feeds, vec![1]);
        assert_eq!(
            created
                .matched_articles
                .iter()
                .map(|a| a.pk)
                .collect::<Vec<_>>(),
            vec![100]
        );

        let feed_a = list_article_previews_for_feed(&state.db, 1).await.unwrap();
        assert!(
            feed_a.is_empty(),
            "feed A's matching article should be hidden"
        );

        let feed_b = list_article_previews_for_feed(&state.db, 2).await.unwrap();
        assert_eq!(
            feed_b.len(),
            1,
            "feed B's matching article should not be hidden by a filter scoped to feed A"
        );
    }

    #[tokio::test]
    async fn global_filter_hides_matches_on_every_feed() {
        let state = test_state().await;
        seed_feeds_and_matching_articles(&state).await;

        let Json(created) = post_filter(State(state.clone()), Json(post_req(None)))
            .await
            .unwrap();
        assert!(created.feeds.is_empty());
        assert_eq!(
            created
                .matched_articles
                .iter()
                .map(|a| a.pk)
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from([100, 101])
        );

        for feed in [1, 2] {
            let previews = list_article_previews_for_feed(&state.db, feed)
                .await
                .unwrap();
            assert!(
                previews.is_empty(),
                "feed {feed} should have its match hidden"
            );
        }
    }

    #[tokio::test]
    async fn patching_filter_feeds_resweeps_matches() {
        let state = test_state().await;
        seed_feeds_and_matching_articles(&state).await;

        let Json(created) = post_filter(State(state.clone()), Json(post_req(None)))
            .await
            .unwrap();
        for feed in [1, 2] {
            assert!(
                list_article_previews_for_feed(&state.db, feed)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }

        // Narrow to feed A only - feed B's article should reappear.
        let Json(narrowed) = patch_filter(
            State(state.clone()),
            Path(created.filter.pk),
            Json(PatchFilterReq {
                name: None,
                field: None,
                match_type: None,
                pattern: None,
                enabled: None,
                feeds: Some(vec![1]),
            }),
        )
        .await
        .unwrap();
        assert_eq!(narrowed.feeds, vec![1]);
        assert!(
            list_article_previews_for_feed(&state.db, 1)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            list_article_previews_for_feed(&state.db, 2)
                .await
                .unwrap()
                .len(),
            1
        );

        // Clear back to global - both feeds hidden again.
        let Json(cleared) = patch_filter(
            State(state.clone()),
            Path(created.filter.pk),
            Json(PatchFilterReq {
                name: None,
                field: None,
                match_type: None,
                pattern: None,
                enabled: None,
                feeds: Some(vec![]),
            }),
        )
        .await
        .unwrap();
        assert!(cleared.feeds.is_empty());
        for feed in [1, 2] {
            assert!(
                list_article_previews_for_feed(&state.db, feed)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[tokio::test]
    async fn patching_match_type_alone_against_invalid_pattern_errors_cleanly() {
        let state = test_state().await;

        let Json(created) = post_filter(
            State(state.clone()),
            Json(PostFilterReq {
                name: "Odd pattern".to_string(),
                field: None,
                match_type: "contains".to_string(),
                pattern: "(unclosed".to_string(),
                feeds: None,
            }),
        )
        .await
        .unwrap();

        let result = patch_filter(
            State(state.clone()),
            Path(created.filter.pk),
            Json(PatchFilterReq {
                name: None,
                field: None,
                match_type: Some("regex".to_string()),
                pattern: None,
                enabled: None,
                feeds: None,
            }),
        )
        .await;
        assert!(result.is_err(), "expected a clean error, not a panic");

        let stored = database::get_filter(&state.db, created.filter.pk)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.match_type, "contains");
        assert_eq!(stored.pattern, "(unclosed");
    }
}
