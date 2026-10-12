use super::{Db, DbResult, FeedScope};
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Clone, sqlx::FromRow, Serialize, JsonSchema)]
pub struct Feed {
    pub pk: i64,
    pub name: String,
    pub url: String,
    pub display_url: String,
    #[serde(rename = "category_id")]
    #[schemars(rename = "category_id")]
    pub category: Option<i64>,
    pub metadata: String,
    pub refresh_interval: i64,
    pub last_refresh: Option<i64>,
    pub next_poll_at: Option<i64>,
    pub greader_hidden: bool,
    #[serde(skip)]
    pub etag: Option<String>,
    #[serde(skip)]
    pub last_modified: Option<String>,
    pub failure_count: i64,
    pub disabled: bool,
    pub last_error: Option<String>,
    pub moved_from: Option<String>,
    pub redirected_to: Option<String>,
    #[serde(skip)]
    pub ttl_secs: Option<i64>,
    #[serde(skip)]
    pub skip_hours: String,
    #[serde(skip)]
    pub skip_days: String,
}

impl Feed {
    /// The URL to show as the feed's human-facing site link, falling back
    /// to the feed's own URL when no display URL is on file (e.g. the feed
    /// had no `<link>` at parse time).
    pub fn html_url(&self) -> &str {
        if self.display_url.is_empty() {
            &self.url
        } else {
            &self.display_url
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn create_feed(
    db: &Db,
    name: &str,
    url: &str,
    display_url: &str,
    category: Option<i64>,
    metadata: Option<String>,
    refresh_interval: Option<i64>,
    greader_hidden: bool,
) -> DbResult<Feed> {
    let metadata = metadata.unwrap_or_else(|| "{}".to_string());
    let refresh_interval = refresh_interval.unwrap_or(3600);

    let feed = sqlx::query_as!(
        Feed,
        r#"INSERT INTO feed (name, url, display_url, category, metadata, refresh_interval, greader_hidden)
           VALUES (?, ?, ?, ?, ?, ?, ?)
           RETURNING pk, name, url, display_url, category, metadata, refresh_interval, last_refresh, next_poll_at, greader_hidden as "greader_hidden: bool", etag, last_modified, failure_count, disabled as "disabled: bool", last_error, moved_from, redirected_to, ttl_secs, skip_hours, skip_days"#,
        name,
        url,
        display_url,
        category,
        metadata,
        refresh_interval,
        greader_hidden,
    )
    .fetch_one(&db.write)
    .await?;

    Ok(feed)
}

pub async fn list_feed(db: &Db, pk: i64) -> DbResult<Option<Feed>> {
    let feed = sqlx::query_as!(
        Feed,
        r#"SELECT pk, name, url, display_url, category, metadata, refresh_interval, last_refresh, next_poll_at, greader_hidden as "greader_hidden: bool", etag, last_modified, failure_count, disabled as "disabled: bool", last_error, moved_from, redirected_to, ttl_secs, skip_hours, skip_days
           FROM feed WHERE pk = ?"#,
        pk,
    )
    .fetch_optional(&db.read)
    .await?;

    Ok(feed)
}

pub async fn get_feed_by_url(db: &Db, url: &str) -> DbResult<Option<Feed>> {
    let feed = sqlx::query_as!(
        Feed,
        r#"SELECT pk, name, url, display_url, category, metadata, refresh_interval, last_refresh, next_poll_at, greader_hidden as "greader_hidden: bool", etag, last_modified, failure_count, disabled as "disabled: bool", last_error, moved_from, redirected_to, ttl_secs, skip_hours, skip_days
           FROM feed WHERE url = ?"#,
        url,
    )
    .fetch_optional(&db.read)
    .await?;

    Ok(feed)
}

pub async fn list_feeds(db: &Db, scope: FeedScope) -> DbResult<Vec<Feed>> {
    let visible_only = scope.visible_only();
    let feeds = sqlx::query_as!(
        Feed,
        r#"SELECT pk, name, url, display_url, category, metadata, refresh_interval, last_refresh, next_poll_at, greader_hidden as "greader_hidden: bool", etag, last_modified, failure_count, disabled as "disabled: bool", last_error, moved_from, redirected_to, ttl_secs, skip_hours, skip_days
           FROM feed WHERE NOT ? OR greader_hidden = 0 ORDER BY name"#,
        visible_only,
    )
    .fetch_all(&db.read)
    .await?;

    Ok(feeds)
}

pub async fn list_feeds_due_for_refresh_at(db: &Db, now: i64) -> DbResult<Vec<Feed>> {
    let feeds = sqlx::query_as!(
        Feed,
        r#"SELECT pk, name, url, display_url, category, metadata, refresh_interval, last_refresh, next_poll_at, greader_hidden as "greader_hidden: bool", etag, last_modified, failure_count, disabled as "disabled: bool", last_error, moved_from, redirected_to, ttl_secs, skip_hours, skip_days
           FROM feed
           WHERE disabled = 0 AND (next_poll_at IS NULL OR next_poll_at <= ?)"#,
        now,
    )
    .fetch_all(&db.read)
    .await?;

    Ok(feeds)
}

pub async fn update_feed_last_refresh(
    db: &Db,
    pk: i64,
    last_refresh: i64,
    next_poll_at: i64,
) -> DbResult<()> {
    sqlx::query!(
        "UPDATE feed SET last_refresh = ?, next_poll_at = ? WHERE pk = ?",
        last_refresh,
        next_poll_at,
        pk
    )
    .execute(&db.write)
    .await?;

    Ok(())
}

pub async fn record_poll(db: &Db, pk: i64, update: &crate::poll::PollUpdate) -> DbResult<()> {
    let validators_provided = update.validators.is_some();
    let validators = update.validators.clone().unwrap_or_default();
    let hints_provided = update.hints.is_some();
    let hints = update.hints.clone().unwrap_or_default();
    let skip_hours = hints.skip_hours_text();
    let skip_days = hints.skip_days_text();
    sqlx::query!(
        r#"UPDATE feed
           SET last_refresh = COALESCE(?, last_refresh),
               next_poll_at = ?,
               failure_count = ?,
               disabled = ?,
               last_error = ?,
               etag = CASE WHEN ? THEN ? ELSE etag END,
               last_modified = CASE WHEN ? THEN ? ELSE last_modified END,
               moved_from = CASE WHEN ? IS NOT NULL THEN url ELSE moved_from END,
               url = COALESCE(?, url),
               redirected_to = ?,
               ttl_secs = CASE WHEN ? THEN ? ELSE ttl_secs END,
               skip_hours = CASE WHEN ? THEN ? ELSE skip_hours END,
               skip_days = CASE WHEN ? THEN ? ELSE skip_days END
           WHERE pk = ?"#,
        update.last_refresh,
        update.next_poll_at,
        update.failure_count,
        update.disabled,
        update.last_error,
        validators_provided,
        validators.etag,
        validators_provided,
        validators.last_modified,
        update.moved_to,
        update.moved_to,
        update.redirected_to,
        hints_provided,
        hints.ttl_secs,
        hints_provided,
        skip_hours,
        hints_provided,
        skip_days,
        pk,
    )
    .execute(&db.write)
    .await?;

    Ok(())
}

pub async fn set_feed_disabled(db: &Db, pk: i64, disabled: bool) -> DbResult<()> {
    sqlx::query!(
        r#"UPDATE feed
           SET disabled = ?,
               failure_count = CASE WHEN ? THEN failure_count ELSE 0 END,
               next_poll_at = CASE WHEN ? THEN next_poll_at ELSE NULL END
           WHERE pk = ?"#,
        disabled,
        disabled,
        disabled,
        pk,
    )
    .execute(&db.write)
    .await?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn update_feed(
    db: &Db,
    pk: i64,
    name: Option<&str>,
    display_url: Option<&str>,
    metadata: Option<&str>,
    refresh_interval: Option<i64>,
    category: Option<Option<i64>>,
    greader_hidden: Option<bool>,
) -> DbResult<Feed> {
    let category_provided = category.is_some();
    let category = category.flatten();

    let feed = sqlx::query_as!(
        Feed,
        r#"UPDATE feed
           SET name = COALESCE(?, name),
               display_url = COALESCE(?, display_url),
               metadata = COALESCE(?, metadata),
               refresh_interval = COALESCE(?, refresh_interval),
               category = CASE WHEN ? THEN ? ELSE category END,
               greader_hidden = COALESCE(?, greader_hidden)
           WHERE pk = ?
           RETURNING pk, name, url, display_url, category as "category: Option<i64>", metadata, refresh_interval, last_refresh, next_poll_at, greader_hidden as "greader_hidden: bool", etag, last_modified, failure_count, disabled as "disabled: bool", last_error, moved_from, redirected_to, ttl_secs, skip_hours, skip_days"#,
        name,
        display_url,
        metadata,
        refresh_interval,
        category_provided,
        category,
        greader_hidden,
        pk,
    )
    .fetch_one(&db.write)
    .await?;

    Ok(feed)
}

pub async fn drop_feed(db: &Db, pk: i64) -> DbResult<()> {
    sqlx::query!("DELETE FROM feed WHERE pk = ?", pk)
        .execute(&db.write)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn make_feed(url: &str, display_url: &str) -> Feed {
        Feed {
            pk: 1,
            name: "Feed".to_string(),
            url: url.to_string(),
            display_url: display_url.to_string(),
            category: None,
            metadata: "{}".to_string(),
            refresh_interval: 3600,
            last_refresh: None,
            next_poll_at: None,
            greader_hidden: false,
            etag: None,
            last_modified: None,
            failure_count: 0,
            disabled: false,
            last_error: None,
            moved_from: None,
            redirected_to: None,
            ttl_secs: None,
            skip_hours: String::new(),
            skip_days: String::new(),
        }
    }

    #[test]
    fn html_url_falls_back_to_url_when_display_url_empty() {
        let feed = make_feed("https://example.com/feed.xml", "");
        assert_eq!(feed.html_url(), "https://example.com/feed.xml");
    }

    #[test]
    fn html_url_prefers_display_url_when_present() {
        let feed = make_feed("https://example.com/feed.xml", "https://example.com/site");
        assert_eq!(feed.html_url(), "https://example.com/site");
    }

    #[tokio::test]
    async fn null_next_poll_at_is_always_due() {
        let db = test_db().await;
        create_feed(
            &db,
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

        let due = list_feeds_due_for_refresh_at(&db, 1000).await.unwrap();
        assert_eq!(due.len(), 1);
    }

    #[tokio::test]
    async fn next_poll_at_relative_to_now() {
        let db = test_db().await;
        let past = create_feed(
            &db,
            "Past",
            "https://example.com/past",
            "",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();
        let equal = create_feed(
            &db,
            "Equal",
            "https://example.com/equal",
            "",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();
        let future = create_feed(
            &db,
            "Future",
            "https://example.com/future",
            "",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();

        update_feed_last_refresh(&db, past.pk, 500, 500)
            .await
            .unwrap();
        update_feed_last_refresh(&db, equal.pk, 500, 1000)
            .await
            .unwrap();
        update_feed_last_refresh(&db, future.pk, 500, 1500)
            .await
            .unwrap();

        let due = list_feeds_due_for_refresh_at(&db, 1000).await.unwrap();
        let due_pks: std::collections::HashSet<i64> = due.iter().map(|f| f.pk).collect();

        assert!(due_pks.contains(&past.pk));
        assert!(
            due_pks.contains(&equal.pk),
            "next_poll_at == now should be due"
        );
        assert!(!due_pks.contains(&future.pk));
    }

    #[tokio::test]
    async fn mixed_set_returns_exactly_the_due_ones() {
        let db = test_db().await;
        let due_null = create_feed(
            &db,
            "Null",
            "https://example.com/null",
            "",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();
        let due_past = create_feed(
            &db,
            "Past",
            "https://example.com/past2",
            "",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();
        let not_due = create_feed(
            &db,
            "Future",
            "https://example.com/future2",
            "",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();

        update_feed_last_refresh(&db, due_past.pk, 500, 900)
            .await
            .unwrap();
        update_feed_last_refresh(&db, not_due.pk, 500, 2000)
            .await
            .unwrap();

        let due = list_feeds_due_for_refresh_at(&db, 1000).await.unwrap();
        let due_pks: std::collections::HashSet<i64> = due.iter().map(|f| f.pk).collect();

        assert_eq!(
            due_pks,
            std::collections::HashSet::from([due_null.pk, due_past.pk])
        );
    }
}
