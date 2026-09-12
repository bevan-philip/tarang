use crate::database::{Db, DbResult, list_feeds_due_for_refresh_at, update_feed_last_refresh};
use crate::feed::{FeedResult, update_feed_articles};
use crate::filter;
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

    let (succeeded, failed) = stream::iter(feeds)
        .map(|feed| {
            let filters = Arc::clone(&filters);
            async move {
                let result: FeedResult<()> = async {
                    update_feed_articles(db, feed.pk, &feed.url, client, &filters).await?;
                    update_feed_last_refresh(db, feed.pk, now, now + feed.refresh_interval).await?;
                    Ok(())
                }
                .await;
                (feed.pk, feed.url, result)
            }
        })
        .buffer_unordered(8)
        .fold(
            (0u32, 0u32),
            |(succeeded, failed), (pk, url, res)| async move {
                match res {
                    Ok(()) => (succeeded + 1, failed),
                    Err(e) => {
                        tracing::error!(feed = pk, url = %url, error = %e, "feed sync failed");
                        (succeeded, failed + 1)
                    }
                }
            },
        )
        .await;

    Ok(SyncSummary { succeeded, failed })
}

/// Real-clock convenience wrapper for main.rs.
pub async fn sync_feeds_now(db: &Db, client: &reqwest::Client) -> DbResult<SyncSummary> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    sync_feeds_at(db, client, now).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{create_feed, list_feed};

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
        assert_eq!(updated.next_poll_at, Some(1000 + updated.refresh_interval));

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
        assert_eq!(updated_bad.next_poll_at, None);

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
}
