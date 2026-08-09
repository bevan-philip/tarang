use chrono::Utc;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::sqlite::SqliteJournalMode;
use sqlx::sqlite::SqlitePool;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::sqlite::SqliteSynchronous;
use sqlx::{QueryBuilder, Sqlite};
use std::error::Error;
use std::str::FromStr;
use std::time::Duration;
use uuid::Uuid;

pub type DbResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Clone)]
pub struct Db {
    pub read: SqlitePool,
    pub write: SqlitePool,
}

pub async fn config() -> DbResult<Db> {
    let base_opts = SqliteConnectOptions::from_str("sqlite://app.db")?
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5))
        .pragma("cache_size", "-20000")
        .pragma("temp_store", "MEMORY");

    let write_opts = base_opts
        .clone()
        .create_if_missing(true)
        .optimize_on_close(true, None);

    let write = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(write_opts)
        .await?;

    sqlx::migrate!().run(&write).await?;

    let read_opts = base_opts.read_only(true);

    let read = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(read_opts)
        .await?;

    Ok(Db { read, write })
}

// -----------------------------------------------------------------------
// feed
// -----------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Feed {
    pub pk: i64,
    pub id: String,
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
    let id = Uuid::new_v4().to_string();
    let metadata = metadata.unwrap_or("{}");
    let refresh_interval = refresh_interval.unwrap_or(3600);

    let feed = sqlx::query_as!(
        Feed,
        r#"INSERT INTO feed (id, name, url, metadata, refresh_interval)
           VALUES (?, ?, ?, ?, ?)
           RETURNING pk, id, name, url, metadata, refresh_interval, last_refresh, next_poll_at"#,
        id,
        name,
        url,
        metadata,
        refresh_interval,
    )
    .fetch_one(&db.write)
    .await?;

    Ok(feed)
}

pub async fn get_feed_by_id(db: &Db, id: &str) -> DbResult<Option<Feed>> {
    let feed = sqlx::query_as!(
        Feed,
        r#"SELECT pk, id, name, url, metadata, refresh_interval, last_refresh, next_poll_at
           FROM feed WHERE id = ?"#,
        id,
    )
    .fetch_optional(&db.read)
    .await?;

    Ok(feed)
}

pub async fn get_feed_by_pk(db: &Db, pk: i64) -> DbResult<Option<Feed>> {
    let feed = sqlx::query_as!(
        Feed,
        r#"SELECT pk, id, name, url, metadata, refresh_interval, last_refresh, next_poll_at
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
        r#"SELECT pk, id, name, url, metadata, refresh_interval, last_refresh, next_poll_at
           FROM feed WHERE url = ?"#,
        url,
    )
    .fetch_optional(&db.read)
    .await?;

    Ok(feed)
}

pub async fn list_feeds(db: &Db) -> DbResult<Vec<Feed>> {
    let feeds = sqlx::query_as!(
        Feed,
        r#"SELECT pk, id, name, url, metadata, refresh_interval, last_refresh, next_poll_at
           FROM feed ORDER BY name"#,
    )
    .fetch_all(&db.read)
    .await?;

    Ok(feeds)
}

pub async fn list_feeds_due_for_refresh(db: &Db) -> DbResult<Vec<Feed>> {
    let feeds = sqlx::query_as!(
        Feed,
        r#"SELECT pk, id, name, url, metadata, refresh_interval, last_refresh, next_poll_at
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

pub async fn update_feed_metadata(db: &Db, pk: i64, metadata: &str) -> DbResult<()> {
    sqlx::query!("UPDATE feed SET metadata = ? WHERE pk = ?", metadata, pk)
        .execute(&db.write)
        .await?;

    Ok(())
}

pub async fn delete_feed(db: &Db, pk: i64) -> DbResult<()> {
    sqlx::query!("DELETE FROM feed WHERE pk = ?", pk)
        .execute(&db.write)
        .await?;

    Ok(())
}

// -----------------------------------------------------------------------
// category
// -----------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Category {
    pub pk: i64,
    pub id: String,
    pub name: String,
}

pub async fn create_category(db: &Db, name: &str) -> DbResult<Category> {
    let id = Uuid::new_v4().to_string();

    let category = sqlx::query_as!(
        Category,
        r#"INSERT INTO category (id, name) VALUES (?, ?)
           RETURNING pk, id, name"#,
        id,
        name,
    )
    .fetch_one(&db.write)
    .await?;

    Ok(category)
}

pub async fn get_category_by_id(db: &Db, id: &str) -> DbResult<Option<Category>> {
    let category = sqlx::query_as!(
        Category,
        r#"SELECT pk, id, name FROM category WHERE id = ?"#,
        id,
    )
    .fetch_optional(&db.read)
    .await?;

    Ok(category)
}

pub async fn list_categories(db: &Db) -> DbResult<Vec<Category>> {
    let categories = sqlx::query_as!(
        Category,
        r#"SELECT pk, id, name FROM category ORDER BY name"#
    )
    .fetch_all(&db.read)
    .await?;

    Ok(categories)
}

pub async fn delete_category(db: &Db, pk: i64) -> DbResult<()> {
    sqlx::query!("DELETE FROM category WHERE pk = ?", pk)
        .execute(&db.write)
        .await?;

    Ok(())
}

// -----------------------------------------------------------------------
// feed_category
// -----------------------------------------------------------------------

pub async fn add_feed_to_category(db: &Db, feed_pk: i64, category_pk: i64) -> DbResult<()> {
    sqlx::query!(
        "INSERT OR IGNORE INTO feed_category (feed, category) VALUES (?, ?)",
        feed_pk,
        category_pk,
    )
    .execute(&db.write)
    .await?;

    Ok(())
}

pub async fn remove_feed_from_category(db: &Db, feed_pk: i64, category_pk: i64) -> DbResult<()> {
    sqlx::query!(
        "DELETE FROM feed_category WHERE feed = ? AND category = ?",
        feed_pk,
        category_pk,
    )
    .execute(&db.write)
    .await?;

    Ok(())
}

pub async fn list_categories_for_feed(db: &Db, feed_pk: i64) -> DbResult<Vec<Category>> {
    let categories = sqlx::query_as!(
        Category,
        r#"SELECT category.pk, category.id, category.name
           FROM category
           JOIN feed_category ON feed_category.category = category.pk
           WHERE feed_category.feed = ?
           ORDER BY category.name"#,
        feed_pk,
    )
    .fetch_all(&db.read)
    .await?;

    Ok(categories)
}

pub async fn list_feeds_for_category(db: &Db, category_pk: i64) -> DbResult<Vec<Feed>> {
    let feeds = sqlx::query_as!(
        Feed,
        r#"SELECT feed.pk, feed.id, feed.name, feed.url, feed.metadata,
                  feed.refresh_interval, feed.last_refresh, feed.next_poll_at
           FROM feed
           JOIN feed_category ON feed_category.feed = feed.pk
           WHERE feed_category.category = ?
           ORDER BY feed.name"#,
        category_pk,
    )
    .fetch_all(&db.read)
    .await?;

    Ok(feeds)
}

// -----------------------------------------------------------------------
// article
// -----------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Article {
    pub pk: i64,
    pub id: String,
    pub feed: i64,
    pub url: String,
    pub author: String,
    pub content: String,
    pub published_at: Option<i64>,
    pub retrieved_at: i64,
}

pub struct ParsedArticle {
    pub url: String,
    pub content: String,
    pub author: String,
    pub published_at: chrono::DateTime<Utc>,
}

pub async fn create_article(
    db: &Db,
    feed_pk: i64,
    url: &str,
    content: &str,
    author: &str,
    published_at: Option<i64>,
) -> DbResult<Article> {
    let id = Uuid::new_v4().to_string();

    let article = sqlx::query_as!(
        Article,
        r#"INSERT INTO article (id, feed, url, content, author, published_at)
           VALUES (?, ?, ?, ?, ?, ?)
           ON CONFLICT(url) DO UPDATE SET
               content      = excluded.content,
               author       = excluded.author,
               published_at = excluded.published_at,
               retrieved_at = unixepoch()
           RETURNING pk, id, feed, url, content, author, published_at, retrieved_at"#,
        id,
        feed_pk,
        url,
        content,
        author,
        published_at,
    )
    .fetch_one(&db.write)
    .await?;

    Ok(article)
}

pub async fn create_articles(
    db: &Db,
    feed_pk: i64,
    articles: &[ParsedArticle],
) -> DbResult<Vec<Article>> {
    if articles.is_empty() {
        return Ok(Vec::new());
    }

    let mut qb: QueryBuilder<Sqlite> =
        QueryBuilder::new("INSERT INTO article (id, feed, url, content, author, published_at) ");

    qb.push_values(articles, |mut b, article| {
        b.push_bind(Uuid::new_v4().to_string())
            .push_bind(feed_pk)
            .push_bind(&article.url)
            .push_bind(&article.content)
            .push_bind(&article.author)
            .push_bind(article.published_at.timestamp());
    });

    qb.push(
        r#" ON CONFLICT(url) DO UPDATE SET
                content      = excluded.content,
                author       = excluded.author,
                published_at = excluded.published_at,
                retrieved_at = unixepoch()
            RETURNING pk, id, feed, url, content, author, published_at, retrieved_at"#,
    );

    let inserted = qb.build_query_as::<Article>().fetch_all(&db.write).await?;

    Ok(inserted)
}

pub async fn get_article_by_id(db: &Db, id: &str) -> DbResult<Option<Article>> {
    let article = sqlx::query_as!(
        Article,
        r#"SELECT pk, id, feed, url, content, author, published_at, retrieved_at
           FROM article WHERE id = ?"#,
        id,
    )
    .fetch_optional(&db.read)
    .await?;

    Ok(article)
}

pub async fn list_articles_for_feed(
    db: &Db,
    feed_pk: i64,
    limit: i64,
    offset: i64,
) -> DbResult<Vec<Article>> {
    let articles = sqlx::query_as!(
        Article,
        r#"SELECT pk, id, feed, url, content, author, published_at, retrieved_at
           FROM article
           WHERE feed = ?
           ORDER BY published_at DESC
           LIMIT ? OFFSET ?"#,
        feed_pk,
        limit,
        offset,
    )
    .fetch_all(&db.read)
    .await?;

    Ok(articles)
}

pub async fn list_recent_articles(db: &Db, limit: i64, offset: i64) -> DbResult<Vec<Article>> {
    let articles = sqlx::query_as!(
        Article,
        r#"SELECT pk, id, feed, url, content, author, published_at, retrieved_at
           FROM article
           ORDER BY published_at DESC
           LIMIT ? OFFSET ?"#,
        limit,
        offset,
    )
    .fetch_all(&db.read)
    .await?;

    Ok(articles)
}

pub async fn delete_article(db: &Db, pk: i64) -> DbResult<()> {
    sqlx::query!("DELETE FROM article WHERE pk = ?", pk)
        .execute(&db.write)
        .await?;

    Ok(())
}

// -----------------------------------------------------------------------
// article_state
// -----------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ArticleState {
    pub pk: i64,
    pub article: i64,
    pub is_read: bool,
    pub is_starred: bool,
    pub modified_at: i64,
}

pub async fn get_or_create_article_state(db: &Db, article_pk: i64) -> DbResult<ArticleState> {
    sqlx::query!(
        "INSERT INTO article_state (article) VALUES (?) ON CONFLICT(article) DO NOTHING",
        article_pk,
    )
    .execute(&db.write)
    .await?;

    let state = sqlx::query_as!(
        ArticleState,
        r#"SELECT pk, article, is_read as "is_read: bool", is_starred as "is_starred: bool", modified_at
           FROM article_state WHERE article = ?"#,
        article_pk,
    )
    .fetch_one(&db.write)
    .await?;

    Ok(state)
}

pub async fn set_article_read(db: &Db, article_pk: i64, is_read: bool) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO article_state (article, is_read) VALUES (?, ?)
           ON CONFLICT(article) DO UPDATE SET is_read = excluded.is_read"#,
        article_pk,
        is_read,
    )
    .execute(&db.write)
    .await?;

    Ok(())
}

pub async fn set_article_starred(db: &Db, article_pk: i64, is_starred: bool) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO article_state (article, is_starred) VALUES (?, ?)
           ON CONFLICT(article) DO UPDATE SET is_starred = excluded.is_starred"#,
        article_pk,
        is_starred,
    )
    .execute(&db.write)
    .await?;

    Ok(())
}

pub async fn list_unread_articles(db: &Db, limit: i64, offset: i64) -> DbResult<Vec<Article>> {
    let articles = sqlx::query_as!(
        Article,
        r#"SELECT article.pk, article.id, article.feed, article.url, article.content,
                  article.author, article.published_at, article.retrieved_at
           FROM article
           LEFT JOIN article_state ON article_state.article = article.pk
           WHERE article_state.is_read IS NULL OR article_state.is_read = 0
           ORDER BY article.published_at DESC
           LIMIT ? OFFSET ?"#,
        limit,
        offset,
    )
    .fetch_all(&db.read)
    .await?;

    Ok(articles)
}

pub async fn list_starred_articles(db: &Db, limit: i64, offset: i64) -> DbResult<Vec<Article>> {
    let articles = sqlx::query_as!(
        Article,
        r#"SELECT article.pk, article.id, article.feed, article.url, article.content,
                  article.author, article.published_at, article.retrieved_at
           FROM article
           JOIN article_state ON article_state.article = article.pk
           WHERE article_state.is_starred = 1
           ORDER BY article.published_at DESC
           LIMIT ? OFFSET ?"#,
        limit,
        offset,
    )
    .fetch_all(&db.read)
    .await?;

    Ok(articles)
}

// -----------------------------------------------------------------------
// label
// -----------------------------------------------------------------------

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Label {
    pub pk: i64,
    pub id: String,
    pub name: String,
}

pub async fn create_label(db: &Db, name: &str) -> DbResult<Label> {
    let id = Uuid::new_v4().to_string();

    let label = sqlx::query_as!(
        Label,
        r#"INSERT INTO label (id, name) VALUES (?, ?)
           RETURNING pk, id, name"#,
        id,
        name,
    )
    .fetch_one(&db.write)
    .await?;

    Ok(label)
}

pub async fn get_label_by_id(db: &Db, id: &str) -> DbResult<Option<Label>> {
    let label = sqlx::query_as!(Label, r#"SELECT pk, id, name FROM label WHERE id = ?"#, id)
        .fetch_optional(&db.read)
        .await?;

    Ok(label)
}

pub async fn list_labels(db: &Db) -> DbResult<Vec<Label>> {
    let labels = sqlx::query_as!(Label, r#"SELECT pk, id, name FROM label ORDER BY name"#)
        .fetch_all(&db.read)
        .await?;

    Ok(labels)
}

pub async fn delete_label(db: &Db, pk: i64) -> DbResult<()> {
    sqlx::query!("DELETE FROM label WHERE pk = ?", pk)
        .execute(&db.write)
        .await?;

    Ok(())
}

// -----------------------------------------------------------------------
// article_label
// -----------------------------------------------------------------------

pub async fn add_label_to_article(db: &Db, article_pk: i64, label_pk: i64) -> DbResult<()> {
    sqlx::query!(
        "INSERT OR IGNORE INTO article_label (article, label) VALUES (?, ?)",
        article_pk,
        label_pk,
    )
    .execute(&db.write)
    .await?;

    Ok(())
}

pub async fn remove_label_from_article(db: &Db, article_pk: i64, label_pk: i64) -> DbResult<()> {
    sqlx::query!(
        "DELETE FROM article_label WHERE article = ? AND label = ?",
        article_pk,
        label_pk,
    )
    .execute(&db.write)
    .await?;

    Ok(())
}

pub async fn list_labels_for_article(db: &Db, article_pk: i64) -> DbResult<Vec<Label>> {
    let labels = sqlx::query_as!(
        Label,
        r#"SELECT label.pk, label.id, label.name
           FROM label
           JOIN article_label ON article_label.label = label.pk
           WHERE article_label.article = ?
           ORDER BY label.name"#,
        article_pk,
    )
    .fetch_all(&db.read)
    .await?;

    Ok(labels)
}

pub async fn list_articles_for_label(
    db: &Db,
    label_pk: i64,
    limit: i64,
    offset: i64,
) -> DbResult<Vec<Article>> {
    let articles = sqlx::query_as!(
        Article,
        r#"SELECT article.pk, article.id, article.feed, article.url, article.content,
                  article.author, article.published_at, article.retrieved_at
           FROM article
           JOIN article_label ON article_label.article = article.pk
           WHERE article_label.label = ?
           ORDER BY article.published_at DESC
           LIMIT ? OFFSET ?"#,
        label_pk,
        limit,
        offset,
    )
    .fetch_all(&db.read)
    .await?;

    Ok(articles)
}
