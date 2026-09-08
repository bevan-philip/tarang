use super::{Db, DbResult, FeedScope};
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Clone, sqlx::FromRow, Serialize, JsonSchema)]
pub struct Feed {
    pub pk: i64,
    pub name: String,
    pub url: String,
    #[serde(rename = "category_id")]
    #[schemars(rename = "category_id")]
    pub category: Option<i64>,
    pub metadata: String,
    pub refresh_interval: i64,
    pub last_refresh: Option<i64>,
    pub next_poll_at: Option<i64>,
    pub greader_hidden: bool,
}

pub async fn create_feed(
    db: &Db,
    name: &str,
    url: &str,
    category: Option<i64>,
    metadata: Option<&str>,
    refresh_interval: Option<i64>,
    greader_hidden: bool,
) -> DbResult<Feed> {
    let metadata = metadata.unwrap_or("{}");
    let refresh_interval = refresh_interval.unwrap_or(3600);

    let feed = sqlx::query_as!(
        Feed,
        r#"INSERT INTO feed (name, url, category, metadata, refresh_interval, greader_hidden)
           VALUES (?, ?, ?, ?, ?, ?)
           RETURNING pk, name, url, category, metadata, refresh_interval, last_refresh, next_poll_at, greader_hidden as "greader_hidden: bool""#,
        name,
        url,
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
        r#"SELECT pk, name, url, category, metadata, refresh_interval, last_refresh, next_poll_at, greader_hidden as "greader_hidden: bool"
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
        r#"SELECT pk, name, url, category, metadata, refresh_interval, last_refresh, next_poll_at, greader_hidden as "greader_hidden: bool"
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
        r#"SELECT pk, name, url, category, metadata, refresh_interval, last_refresh, next_poll_at, greader_hidden as "greader_hidden: bool"
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
        r#"SELECT pk, name, url, category, metadata, refresh_interval, last_refresh, next_poll_at, greader_hidden as "greader_hidden: bool"
           FROM feed
           WHERE next_poll_at IS NULL OR next_poll_at <= ?"#,
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

pub async fn update_feed(
    db: &Db,
    pk: i64,
    name: Option<&str>,
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
               metadata = COALESCE(?, metadata),
               refresh_interval = COALESCE(?, refresh_interval),
               category = CASE WHEN ? THEN ? ELSE category END,
               greader_hidden = COALESCE(?, greader_hidden)
           WHERE pk = ?
           RETURNING pk, name, url, category as "category: Option<i64>", metadata, refresh_interval, last_refresh, next_poll_at, greader_hidden as "greader_hidden: bool""#,
        name,
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

    #[tokio::test]
    async fn null_next_poll_at_is_always_due() {
        let db = test_db().await;
        create_feed(
            &db,
            "Feed",
            "https://example.com/feed",
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
