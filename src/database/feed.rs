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

pub async fn list_feeds_due_for_refresh(db: &Db) -> DbResult<Vec<Feed>> {
    let feeds = sqlx::query_as!(
        Feed,
        r#"SELECT pk, name, url, category, metadata, refresh_interval, last_refresh, next_poll_at, greader_hidden as "greader_hidden: bool"
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
