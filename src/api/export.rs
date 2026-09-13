use axum::{
    extract::{Multipart, State},
    http::StatusCode,
};
use axum_jsonschema::Json;

use super::AppError;
use crate::{
    AppState,
    database::{FeedScope, StarredArticles, list_categories, list_feeds, list_starred_articles},
    feed,
    opml::{self, export_opml},
};

pub async fn upload_opml(
    State(AppState { db, http, .. }): State<AppState>,
    mut multipart: Multipart,
) -> Result<StatusCode, AppError> {
    while let Some(field) = multipart.next_field().await? {
        let opml_bytes = field.bytes().await?;
        let opml_str = str::from_utf8(&opml_bytes)?;
        let feeds = opml::parse_opml(opml_str)?;
        feed::import_opml_feeds(&db, &http, feeds).await?;
    }

    Ok(StatusCode::CREATED)
}

pub async fn get_opml(State(AppState { db, .. }): State<AppState>) -> Result<String, AppError> {
    let feeds = list_feeds(&db, FeedScope::All).await?;
    let categories = list_categories(&db, FeedScope::All).await?;

    Ok(export_opml(feeds, categories)?)
}

pub async fn get_export_starred_articles(
    State(AppState { db, .. }): State<AppState>,
) -> Result<Json<Vec<StarredArticles>>, AppError> {
    let articles = list_starred_articles(&db).await?;

    Ok(Json(articles))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{Db, ParsedArticle, create_articles, create_feed, mark_articles_starred};
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
            discovery: Default::default(),
        }
    }

    #[tokio::test]
    async fn get_opml_round_trips_created_feed() {
        let state = test_state().await;
        create_feed(
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

        let opml = get_opml(State(state.clone())).await.unwrap();
        assert!(opml.contains("https://example.com/feed"));
        assert!(opml.contains("Feed"));
    }

    #[tokio::test]
    async fn get_export_starred_articles_returns_only_starred() {
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
        mark_articles_starred(&state.db, &[articles[0].pk], true)
            .await
            .unwrap();

        let Json(starred) = get_export_starred_articles(State(state.clone()))
            .await
            .unwrap();
        assert_eq!(starred.len(), 1);
        assert_eq!(starred[0].url, "https://example.com/article");
    }

    async fn spawn_test_feed_server() -> (String, tokio::task::JoinHandle<()>) {
        use axum::{Router, routing::get};

        let app = Router::new().route(
            "/feed",
            get(|| async {
                r#"<rss version="2.0"><channel>
                    <title>Feed</title>
                    <link>https://example.com</link>
                    <item>
                        <title>Item</title>
                        <link>https://example.com/item</link>
                        <pubDate>Mon, 01 Jan 2026 00:00:00 GMT</pubDate>
                    </item>
                </channel></rss>"#
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, server)
    }

    #[tokio::test]
    async fn upload_opml_imports_feeds_from_multipart() {
        use axum::body::Body;
        use axum::extract::{FromRequest, Multipart};
        use axum::http::{Request, header};

        let mut state = test_state().await;
        state.http = reqwest::Client::builder().no_proxy().build().unwrap();
        let (base, server) = spawn_test_feed_server().await;

        let opml = format!(
            r#"<?xml version="1.0"?>
<opml version="2.0">
  <body>
    <outline text="Feed" title="Feed" type="rss" xmlUrl="{base}/feed"/>
  </body>
</opml>"#
        );
        let boundary = "boundary123";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"feeds.opml\"\r\nContent-Type: text/xml\r\n\r\n{opml}\r\n--{boundary}--\r\n"
        );
        let request = Request::builder()
            .method("POST")
            .uri("/")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        let multipart = Multipart::from_request(request, &state).await.unwrap();

        let status = upload_opml(State(state.clone()), multipart).await.unwrap();
        assert_eq!(status, StatusCode::CREATED);

        let feeds = crate::database::list_feeds(&state.db, FeedScope::All)
            .await
            .unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].url, format!("{base}/feed"));

        server.abort();
    }
}
