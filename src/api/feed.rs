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
    database::{
        ArticlePreview, DbError, Feed, StarredArticlePreview, drop_feed,
        list_article_previews_for_feed, list_feed, list_starred_articles_with_feed, update_feed,
    },
    feed::{FeedOptions, create_feed_with_articles},
};

#[derive(Serialize, JsonSchema)]
pub struct GetFeed {
    id: i64,
    feed: Feed,
    articles: Vec<ArticlePreview>,
}

pub async fn get_feed(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<GetFeed>, AppError> {
    let feed = list_feed(&db, id)
        .await?
        .ok_or_else(|| DbError::NotFound(format!("feed {id} not found")))?;
    let articles = list_article_previews_for_feed(&db, id).await?;

    Ok(Json(GetFeed { id, feed, articles }))
}

#[derive(Deserialize, JsonSchema)]
pub struct PostFeedReq {
    pub name: Option<String>,
    pub url: String,
    pub category_id: Option<i64>,
    pub metadata: Option<String>,
    pub refresh_interval: Option<i64>,
    pub greader_hidden: Option<bool>,
}

#[derive(Serialize, JsonSchema)]
pub struct PostFeedResp {
    pub name: String,
    pub id: i64,
    pub greader_hidden: bool,
}

pub async fn post_feed(
    State(AppState { db, http }): State<AppState>,
    Json(payload): Json<PostFeedReq>,
) -> Result<Json<PostFeedResp>, AppError> {
    let feed = create_feed_with_articles(
        &db,
        &http,
        &payload.url,
        FeedOptions {
            name: payload.name,
            category: payload.category_id,
            metadata: payload.metadata,
            refresh_interval: payload.refresh_interval,
            greader_hidden: payload.greader_hidden.unwrap_or(false),
        },
    )
    .await?;

    Ok(Json(PostFeedResp {
        name: feed.name,
        id: feed.pk,
        greader_hidden: feed.greader_hidden,
    }))
}

pub async fn delete_feed(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, AppError> {
    drop_feed(&db, id).await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize, JsonSchema)]
pub struct PatchFeedReq {
    name: Option<String>,
    metadata: Option<String>,
    refresh_interval: Option<i64>,
    greader_hidden: Option<bool>,
    #[serde(default, with = "::serde_with::rust::double_option")]
    #[schemars(with = "Option<i64>")]
    category_id: Option<Option<i64>>,
}

pub async fn patch_feed(
    State(AppState { db, .. }): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<PatchFeedReq>,
) -> Result<Json<Feed>, AppError> {
    let feed = update_feed(
        &db,
        id,
        payload.name.as_deref(),
        payload.metadata.as_deref(),
        payload.refresh_interval,
        payload.category_id,
        payload.greader_hidden,
    )
    .await?;

    Ok(Json(feed))
}

pub async fn get_starred_articles(
    State(AppState { db, .. }): State<AppState>,
) -> Result<Json<Vec<StarredArticlePreview>>, AppError> {
    let articles = list_starred_articles_with_feed(&db).await?;
    Ok(Json(articles))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{PatchArticleReq, get_article, patch_article};
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
        }
    }

    #[tokio::test]
    async fn feed_name_defaults_and_explicit_overrides() {
        use axum::{
            Router,
            routing::{get, post},
        };

        let state = test_state().await;
        let app = Router::new()
            .route("/feed", post(post_feed))
            .route("/rss", get(|| async {
                r#"<rss version="2.0"><channel><title>RSS title</title><link>https://example.com</link><description>Test</description></channel></rss>"#
            }))
            .route("/atom", get(|| async {
                r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>Atom title</title><id>urn:test:feed</id><updated>2026-01-01T00:00:00Z</updated></feed>"#
            }))
            .route("/untitled", get(|| async {
                r#"<feed xmlns="http://www.w3.org/2005/Atom"><id>urn:test:untitled</id><updated>2026-01-01T00:00:00Z</updated></feed>"#
            }))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        for (path, title) in [
            ("rss", "RSS title"),
            ("atom", "Atom title"),
            ("untitled", ""),
        ] {
            for (index, name) in [
                None,
                Some(serde_json::Value::Null),
                Some(serde_json::json!("Custom")),
                Some(serde_json::json!("")),
                Some(serde_json::json!("  \t")),
            ]
            .into_iter()
            .enumerate()
            {
                let url = format!("{base}/{path}?case={index}");
                let mut payload = serde_json::json!({"url": url});
                if let Some(name) = name {
                    payload["name"] = name;
                }
                let expected = payload
                    .get("name")
                    .and_then(|name| name.as_str())
                    .unwrap_or(if path == "untitled" { &url } else { title });
                let response = client
                    .post(format!("{base}/feed"))
                    .header("content-type", "application/json")
                    .body(payload.to_string())
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK, "{payload}");
                let body: serde_json::Value =
                    serde_json::from_str(&response.text().await.unwrap()).unwrap();
                assert_eq!(body["name"], expected, "{payload}");
                let saved = list_feed(&state.db, body["id"].as_i64().unwrap())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(saved.name, expected);
            }
        }
        server.abort();
    }

    #[test]
    fn feed_name_schema_is_optional_and_nullable() {
        let schema = serde_json::to_value(schemars::schema_for!(PostFeedReq)).unwrap();
        let required = schema["required"].as_array().unwrap();
        assert!(!required.contains(&serde_json::json!("name")));
        assert!(required.contains(&serde_json::json!("url")));
        let types = schema["properties"]["name"]["type"].as_array().unwrap();
        assert!(types.contains(&serde_json::json!("string")));
        assert!(types.contains(&serde_json::json!("null")));
    }

    #[tokio::test]
    async fn empty_starred_list() {
        let Json(articles) = get_starred_articles(State(test_state().await))
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(articles).unwrap(),
            serde_json::json!([])
        );
    }

    #[tokio::test]
    async fn unlimited_starred_previews_support_article_navigation() {
        let state = test_state().await;
        sqlx::raw_sql(
            "INSERT INTO feed (pk, name, url, greader_hidden) VALUES
                (1, 'Visible feed', 'https://example.com/feed', 0),
                (2, 'Hidden feed', 'https://example.com/hidden', 1);
             INSERT INTO filter (pk, name, match_type, pattern)
                VALUES (1, 'Blocked', 'contains', 'Article');",
        )
        .execute(&state.db.write)
        .await
        .unwrap();

        for id in 100..115_i64 {
            sqlx::query(
                "INSERT INTO article
                    (pk, feed, url, guid, title, summary, content, published_at, retrieved_at)
                 VALUES (?, ?, ?, ?, ?, ?, 'Full content', ?, 2000)",
            )
            .bind(id)
            .bind(if id == 112 { 2 } else { 1 })
            .bind(format!("https://example.com/article/{id}"))
            .bind(format!("guid-{id}"))
            .bind((id != 100).then_some("Article title"))
            .bind((id != 100).then_some("Article summary"))
            .bind(match id {
                100 => None,
                111 => Some(500_i64),
                _ => Some(1000_i64),
            })
            .execute(&state.db.write)
            .await
            .unwrap();
            // 113 is explicitly unstarred; 114 has no state record.
            if id != 114 {
                sqlx::query(
                    "INSERT INTO article_state (article, is_read, is_starred) VALUES (?, ?, ?)",
                )
                .bind(id)
                .bind(id % 2 == 0)
                .bind(id != 113)
                .execute(&state.db.write)
                .await
                .unwrap();
            }
        }
        sqlx::query("INSERT INTO article_filter_match (article, filter) VALUES (112, 1)")
            .execute(&state.db.write)
            .await
            .unwrap();

        let Json(articles) = get_starred_articles(State(state.clone())).await.unwrap();
        assert_eq!(articles.len(), 13);
        assert_eq!(articles.iter().filter(|a| a.feed_id == 1).count(), 12);
        assert_eq!(
            articles.iter().map(|a| a.article_id).collect::<Vec<_>>(),
            vec![
                112, 110, 109, 108, 107, 106, 105, 104, 103, 102, 101, 111, 100
            ]
        );
        assert!(articles.iter().all(|a| a.is_starred));
        assert!(articles.iter().any(|a| !a.is_read));
        assert_eq!(
            serde_json::to_value(&articles[0]).unwrap(),
            serde_json::json!({
                "article_id": 112, "feed_id": 2, "feed_name": "Hidden feed",
                "url": "https://example.com/article/112", "title": "Article title",
                "summary": "Article summary", "published_at": 1000,
                "retrieved_at": 2000, "is_read": true, "is_starred": true
            })
        );
        let nullable = serde_json::to_value(articles.last().unwrap()).unwrap();
        assert!(nullable["title"].is_null());
        assert!(nullable["summary"].is_null());
        assert!(nullable["published_at"].is_null());

        let id = articles[0].article_id;
        let Json(article) = get_article(State(state.clone()), Path(id)).await.unwrap();
        assert_eq!(article.pk, id);
        assert_eq!(article.content, "Full content");
        let Json(updated) = patch_article(
            State(state.clone()),
            Path(id),
            Json(PatchArticleReq {
                is_read: Some(false),
                is_starred: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(updated.pk, id);
        assert!(!updated.is_read);
        assert!(updated.is_starred);
        let Json(refreshed) = get_starred_articles(State(state)).await.unwrap();
        assert!(!refreshed[0].is_read);
    }
}
