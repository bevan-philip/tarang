use crate::database::{
    self, Db, DbError, DbResult, Feed, list_feeds_due_for_refresh_at, record_poll,
};
use crate::feed::log_skipped;
use crate::filter::{self, CompiledFilter};
use crate::http;
use crate::poll::{self, Fetch, Validators};
use futures::stream::{self, StreamExt};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct SyncSummary {
    pub succeeded: u32,
    pub failed: u32,
}

pub async fn sync_feeds_at(db: &Db, client: &reqwest::Client, now: i64) -> DbResult<SyncSummary> {
    let feeds = list_feeds_due_for_refresh_at(db, now).await?;
    let filters = Arc::new(filter::load_compiled_filters(db).await?);

    // Feeds on one host are polled one after the other; hosts in parallel.
    let results: Vec<(Feed, DbResult<Option<String>>)> =
        stream::iter(poll::group_by_host(feeds, |feed| &feed.url))
            .map(|group| {
                let filters = Arc::clone(&filters);
                async move {
                    let mut results = Vec::with_capacity(group.len());
                    for feed in group {
                        let result = poll_feed(db, client, &feed, &filters, now).await;
                        results.push((feed, result));
                    }
                    results
                }
            })
            .buffer_unordered(8)
            .flat_map(stream::iter)
            .collect()
            .await;

    let (mut succeeded, mut failed) = (0u32, 0u32);
    for (feed, result) in results {
        match result {
            Ok(None) => succeeded += 1,
            Ok(Some(error)) => {
                tracing::warn!(feed = feed.pk, url = %feed.url, error = %error, "feed poll failed");
                failed += 1;
            }
            Err(e) => {
                tracing::error!(feed = feed.pk, url = %feed.url, error = %e, "feed sync failed");
                failed += 1;
            }
        }
    }

    Ok(SyncSummary { succeeded, failed })
}

// Returns the poll error, if any; Err is for database failures only.
async fn poll_feed(
    db: &Db,
    client: &reqwest::Client,
    feed: &Feed,
    filters: &[CompiledFilter],
    now: i64,
) -> DbResult<Option<String>> {
    let headers = Validators::of(feed).request_headers();
    let response = http::get(client, &feed.url, headers)
        .await
        .map_err(|e| http::describe_error(&e));
    let outcome = poll::interpret(&response, now);

    let moved_to_taken = match &outcome.moved_to {
        Some(url) => database::get_feed_by_url(db, url)
            .await?
            .is_some_and(|other| other.pk != feed.pk),
        None => false,
    };
    let update = poll::next_state(feed, &outcome, moved_to_taken, now);

    if let Fetch::Updated { feed: parsed, .. } = &outcome.fetch
        && !update.disabled
    {
        log_skipped(&parsed.skipped);
        database::create_articles(db, feed.pk, &parsed.articles, filters).await?;
    }

    // Another feed may have moved to the same URL since the check above.
    let update = match record_poll(db, feed.pk, &update).await {
        Err(DbError::AlreadyExists(_)) if update.moved_to.is_some() => {
            let update = poll::next_state(feed, &outcome, true, now);
            record_poll(db, feed.pk, &update).await?;
            update
        }
        result => {
            result?;
            update
        }
    };
    Ok(update.last_error)
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Real-clock convenience wrapper for main.rs.
pub async fn sync_feeds_now(db: &Db, client: &reqwest::Client) -> DbResult<SyncSummary> {
    sync_feeds_at(db, client, unix_now()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{create_feed, list_feed};
    use axum::{
        Router,
        http::{HeaderMap, StatusCode, header},
        response::IntoResponse,
        routing::get,
    };

    const RSS: &str = r#"<rss version="2.0"><channel>
        <title>Feed</title>
        <link>https://example.com</link>
        <item>
            <title>Item</title>
            <link>https://example.com/item</link>
            <pubDate>Mon, 01 Jan 2026 00:00:00 GMT</pubDate>
        </item>
    </channel></rss>"#;

    async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, server)
    }

    fn client() -> reqwest::Client {
        crate::http::client_builder().no_proxy().build().unwrap()
    }

    async fn add_feed(db: &Db, url: &str) -> Feed {
        create_feed(db, "Feed", url, "", None, None, None, false)
            .await
            .unwrap()
    }

    async fn test_db() -> Db {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        Db {
            read: pool.clone(),
            write: pool,
        }
    }

    async fn spawn_test_server() -> (String, tokio::task::JoinHandle<()>) {
        use axum::{Router, routing::get};

        let app = Router::new().route(
            "/good",
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
    async fn cycle_counts_success_and_advances_next_poll_at() {
        let db = test_db().await;
        let (base, server) = spawn_test_server().await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let feed = create_feed(
            &db,
            "Good",
            &format!("{base}/good"),
            "",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();

        let summary = sync_feeds_at(&db, &client, 1000).await.unwrap();
        assert_eq!(summary.succeeded, 1);
        assert_eq!(summary.failed, 0);

        let updated = list_feed(&db, feed.pk).await.unwrap().unwrap();
        assert_eq!(updated.last_refresh, Some(1000));
        assert_eq!(
            updated.next_poll_at,
            Some(1000 + 3600 + poll::jitter(feed.pk, 3600))
        );

        server.abort();
    }

    #[tokio::test]
    async fn cycle_with_one_good_and_one_unreachable_feed() {
        let db = test_db().await;
        let (base, server) = spawn_test_server().await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let good = create_feed(
            &db,
            "Good",
            &format!("{base}/good"),
            "",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();
        let bad = create_feed(
            &db,
            "Bad",
            "http://127.0.0.1:1/unreachable",
            "",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();

        let summary = sync_feeds_at(&db, &client, 1000).await.unwrap();
        assert_eq!(summary.succeeded, 1);
        assert_eq!(summary.failed, 1);

        let updated_good = list_feed(&db, good.pk).await.unwrap().unwrap();
        assert_eq!(updated_good.last_refresh, Some(1000));

        let updated_bad = list_feed(&db, bad.pk).await.unwrap().unwrap();
        assert_eq!(updated_bad.last_refresh, None);
        assert_eq!(
            updated_bad.next_poll_at,
            Some(1000 + 7200 + poll::jitter(bad.pk, 7200))
        );
        assert_eq!(updated_bad.failure_count, 1);
        assert!(updated_bad.last_error.is_some());

        server.abort();
    }

    #[tokio::test]
    async fn feed_not_yet_due_is_neither_fetched_nor_counted() {
        let db = test_db().await;
        let (base, server) = spawn_test_server().await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let not_due = create_feed(
            &db,
            "Future",
            &format!("{base}/good"),
            "",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();
        crate::database::update_feed_last_refresh(&db, not_due.pk, 500, 5000)
            .await
            .unwrap();

        let summary = sync_feeds_at(&db, &client, 1000).await.unwrap();
        assert_eq!(summary.succeeded, 0);
        assert_eq!(summary.failed, 0);

        let unchanged = list_feed(&db, not_due.pk).await.unwrap().unwrap();
        assert_eq!(unchanged.last_refresh, Some(500));
        assert_eq!(unchanged.next_poll_at, Some(5000));

        server.abort();
    }

    #[tokio::test]
    async fn second_poll_is_conditional_and_304_counts_as_success() {
        let db = test_db().await;
        let app = Router::new().route(
            "/feed",
            get(|headers: HeaderMap| async move {
                let etag = headers
                    .get(header::IF_NONE_MATCH)
                    .map(|v| v.to_str().unwrap().to_string());
                let ims = headers
                    .get(header::IF_MODIFIED_SINCE)
                    .map(|v| v.to_str().unwrap().to_string());
                match (etag.as_deref(), ims.as_deref()) {
                    (Some("\"v1\""), Some("Mon, 01 Jan 2026 00:00:00 GMT")) => {
                        StatusCode::NOT_MODIFIED.into_response()
                    }
                    (None, None) => (
                        [
                            (header::ETAG, "\"v1\""),
                            (header::LAST_MODIFIED, "Mon, 01 Jan 2026 00:00:00 GMT"),
                        ],
                        RSS,
                    )
                        .into_response(),
                    _ => StatusCode::BAD_REQUEST.into_response(),
                }
            }),
        );
        let (base, server) = serve(app).await;
        let feed = add_feed(&db, &format!("{base}/feed")).await;

        let first = sync_feeds_at(&db, &client(), 1000).await.unwrap();
        assert_eq!(first.succeeded, 1);
        let stored = list_feed(&db, feed.pk).await.unwrap().unwrap();
        assert_eq!(stored.etag.as_deref(), Some("\"v1\""));
        assert_eq!(
            stored.last_modified.as_deref(),
            Some("Mon, 01 Jan 2026 00:00:00 GMT")
        );

        let second = sync_feeds_at(&db, &client(), 1000 + 7200).await.unwrap();
        assert_eq!(second.succeeded, 1);
        let stored = list_feed(&db, feed.pk).await.unwrap().unwrap();
        assert_eq!(stored.last_refresh, Some(1000 + 7200));
        assert_eq!(stored.etag.as_deref(), Some("\"v1\""));
        assert_eq!(stored.failure_count, 0);

        server.abort();
    }

    #[tokio::test]
    async fn too_many_requests_obeys_retry_after_and_keeps_validators() {
        let db = test_db().await;
        let app = Router::new().route(
            "/feed",
            get(|| async {
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    [(header::RETRY_AFTER, "86400")],
                    "slow down",
                )
            }),
        );
        let (base, server) = serve(app).await;
        let feed = add_feed(&db, &format!("{base}/feed")).await;
        sqlx::query("UPDATE feed SET etag = 'x', last_modified = 'y' WHERE pk = ?")
            .bind(feed.pk)
            .execute(&db.write)
            .await
            .unwrap();

        let summary = sync_feeds_at(&db, &client(), 1000).await.unwrap();
        assert_eq!(summary.failed, 1);
        let stored = list_feed(&db, feed.pk).await.unwrap().unwrap();
        assert_eq!(
            stored.next_poll_at,
            Some(1000 + 86400 + poll::jitter(feed.pk, 86400))
        );
        assert_eq!(stored.etag.as_deref(), Some("x"));
        assert_eq!(stored.last_modified.as_deref(), Some("y"));
        assert_eq!(stored.last_error.as_deref(), Some("HTTP 429: slow down"));

        server.abort();
    }

    #[tokio::test]
    async fn gone_disables_the_feed_and_disabled_feeds_are_not_polled() {
        let db = test_db().await;
        let app = Router::new().route("/feed", get(|| async { StatusCode::GONE }));
        let (base, server) = serve(app).await;
        let feed = add_feed(&db, &format!("{base}/feed")).await;

        sync_feeds_at(&db, &client(), 1000).await.unwrap();
        let stored = list_feed(&db, feed.pk).await.unwrap().unwrap();
        assert!(stored.disabled);

        let later = sync_feeds_at(&db, &client(), i64::MAX / 2).await.unwrap();
        assert_eq!(later.succeeded + later.failed, 0);

        crate::database::set_feed_disabled(&db, feed.pk, false)
            .await
            .unwrap();
        let stored = list_feed(&db, feed.pk).await.unwrap().unwrap();
        assert!(!stored.disabled);
        assert_eq!(stored.failure_count, 0);
        assert_eq!(stored.next_poll_at, None);

        server.abort();
    }

    #[tokio::test]
    async fn permanent_redirect_moves_the_feed_url() {
        let db = test_db().await;
        let app = Router::new()
            .route(
                "/old",
                get(|| async { (StatusCode::MOVED_PERMANENTLY, [(header::LOCATION, "/new")]) }),
            )
            .route(
                "/temp",
                get(|| async { (StatusCode::FOUND, [(header::LOCATION, "/new")]) }),
            )
            .route("/new", get(|| async { RSS }));
        let (base, server) = serve(app).await;
        let moved = add_feed(&db, &format!("{base}/old")).await;
        let temp = add_feed(&db, &format!("{base}/temp")).await;

        let summary = sync_feeds_at(&db, &client(), 1000).await.unwrap();
        assert_eq!(summary.succeeded, 2);

        let moved = list_feed(&db, moved.pk).await.unwrap().unwrap();
        assert_eq!(moved.url, format!("{base}/new"));
        assert_eq!(moved.moved_from, Some(format!("{base}/old")));
        assert_eq!(moved.redirected_to, None);

        let temp = list_feed(&db, temp.pk).await.unwrap().unwrap();
        assert_eq!(temp.url, format!("{base}/temp"));
        assert_eq!(temp.redirected_to, Some(format!("{base}/new")));

        server.abort();
    }

    #[tokio::test]
    async fn permanent_redirect_onto_an_existing_feed_disables_the_duplicate() {
        let db = test_db().await;
        let app = Router::new()
            .route(
                "/old",
                get(|| async { (StatusCode::MOVED_PERMANENTLY, [(header::LOCATION, "/new")]) }),
            )
            .route("/new", get(|| async { RSS }));
        let (base, server) = serve(app).await;
        let old = add_feed(&db, &format!("{base}/old")).await;
        let new = add_feed(&db, &format!("{base}/new")).await;
        crate::database::update_feed_last_refresh(&db, new.pk, 1000, 5000)
            .await
            .unwrap();

        sync_feeds_at(&db, &client(), 1000).await.unwrap();

        let old = list_feed(&db, old.pk).await.unwrap().unwrap();
        assert!(old.disabled);
        assert_eq!(old.url, format!("{base}/old"));
        assert!(old.last_error.unwrap().contains("another feed"));

        let (articles,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM article WHERE feed = ?")
            .bind(old.pk)
            .fetch_one(&db.read)
            .await
            .unwrap();
        assert_eq!(articles, 0);

        server.abort();
    }

    #[tokio::test]
    async fn chain_of_permanent_redirects_moves_the_feed_to_the_end() {
        let db = test_db().await;
        let app = Router::new()
            .route(
                "/a",
                get(|| async { (StatusCode::MOVED_PERMANENTLY, [(header::LOCATION, "/b")]) }),
            )
            .route(
                "/b",
                get(|| async { (StatusCode::PERMANENT_REDIRECT, [(header::LOCATION, "/c")]) }),
            )
            .route("/c", get(|| async { RSS }));
        let (base, server) = serve(app).await;
        let feed = add_feed(&db, &format!("{base}/a")).await;

        let summary = sync_feeds_at(&db, &client(), 1000).await.unwrap();
        assert_eq!(summary.succeeded, 1);

        let feed = list_feed(&db, feed.pk).await.unwrap().unwrap();
        assert_eq!(feed.url, format!("{base}/c"));
        assert_eq!(feed.moved_from, Some(format!("{base}/a")));

        server.abort();
    }
}
