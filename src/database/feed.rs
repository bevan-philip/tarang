use super::{Db, DbResult};
use serde::Serialize;

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct Feed {
    pub pk: i64,
    pub name: String,
    pub url: String,
    pub metadata: String,
    pub refresh_interval: i64,
    pub last_refresh: Option<i64>,
    pub next_poll_at: Option<i64>,
}

pub async fn create_feed(
    db: &Db,
    name: &str,
    url: &str,
    metadata: Option<&str>,
    refresh_interval: Option<i64>,
) -> DbResult<Feed> {
    let metadata = metadata.unwrap_or("{}");
    let refresh_interval = refresh_interval.unwrap_or(3600);

    let feed = sqlx::query_as!(
        Feed,
        r#"INSERT INTO feed (name, url, metadata, refresh_interval)
           VALUES (?, ?, ?, ?)
           RETURNING pk, name, url, metadata, refresh_interval, last_refresh, next_poll_at"#,
        name,
        url,
        metadata,
        refresh_interval,
    )
    .fetch_one(&db.write)
    .await?;

    Ok(feed)
}

pub async fn list_feeds(db: &Db) -> DbResult<Vec<Feed>> {
    let feeds = sqlx::query_as!(
        Feed,
        r#"SELECT pk, name, url, metadata, refresh_interval, last_refresh, next_poll_at
           FROM feed ORDER BY name"#,
    )
    .fetch_all(&db.read)
    .await?;

    Ok(feeds)
}

pub async fn list_feeds_due_for_refresh(db: &Db) -> DbResult<Vec<Feed>> {
    let feeds = sqlx::query_as!(
        Feed,
        r#"SELECT pk, name, url, metadata, refresh_interval, last_refresh, next_poll_at
           FROM feed
           WHERE next_poll_at IS NULL OR next_poll_at <= unixepoch()"#,
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
