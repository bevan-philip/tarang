use super::{Db, DbError, DbResult, FeedScope};
use schemars::JsonSchema;
use serde::Serialize;
use sqlx::{QueryBuilder, Sqlite};

#[derive(Debug, Clone, sqlx::FromRow, Serialize, JsonSchema)]
pub struct ArticleState {
    pub pk: i64,
    pub is_read: bool,
    pub is_starred: bool,
}

pub async fn update_article_state(
    db: &Db,
    pk: i64,
    is_read: Option<bool>,
    is_starred: Option<bool>,
) -> DbResult<ArticleState> {
    let mut tx = db.write.begin_with("BEGIN IMMEDIATE").await?;
    let state = sqlx::query_as::<_, ArticleState>(
        "SELECT article.pk, COALESCE(article_state.is_read, 0) AS is_read,
                COALESCE(article_state.is_starred, 0) AS is_starred
         FROM article LEFT JOIN article_state ON article_state.article = article.pk
         WHERE article.pk = ?",
    )
    .bind(pk)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| DbError::NotFound(format!("article {pk} not found")))?;

    let state = if is_read.is_some() || is_starred.is_some() {
        sqlx::query_as::<_, ArticleState>(
            "INSERT INTO article_state (article, is_read, is_starred)
             VALUES (?, COALESCE(?, 0), COALESCE(?, 0))
             ON CONFLICT(article) DO UPDATE SET
                 is_read = COALESCE(?, article_state.is_read),
                 is_starred = COALESCE(?, article_state.is_starred)
             RETURNING article AS pk, is_read, is_starred",
        )
        .bind(pk)
        .bind(is_read)
        .bind(is_starred)
        .bind(is_read)
        .bind(is_starred)
        .fetch_one(&mut *tx)
        .await?
    } else {
        state
    };
    tx.commit().await?;
    Ok(state)
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct FeedUnreadCount {
    pub feed: i64,
    pub count: i64,
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct CategoryUnreadCount {
    pub category: i64,
    pub count: i64,
}

pub async fn mark_articles_read(db: &Db, article_pks: &[i64], is_read: bool) -> DbResult<()> {
    if article_pks.is_empty() {
        return Ok(());
    }

    let mut qb: QueryBuilder<Sqlite> =
        QueryBuilder::new("INSERT INTO article_state (article, is_read) ");

    qb.push_values(article_pks, |mut b, pk| {
        b.push_bind(pk).push_bind(is_read);
    });

    qb.push(" ON CONFLICT(article) DO UPDATE SET is_read = excluded.is_read");

    qb.build().execute(&db.write).await?;

    Ok(())
}

pub async fn mark_articles_starred(db: &Db, article_pks: &[i64], is_starred: bool) -> DbResult<()> {
    if article_pks.is_empty() {
        return Ok(());
    }

    let mut qb: QueryBuilder<Sqlite> =
        QueryBuilder::new("INSERT INTO article_state (article, is_starred) ");

    qb.push_values(article_pks, |mut b, pk| {
        b.push_bind(pk).push_bind(is_starred);
    });

    qb.push(" ON CONFLICT(article) DO UPDATE SET is_starred = excluded.is_starred");

    qb.build().execute(&db.write).await?;

    Ok(())
}

pub async fn mark_all_read_global(db: &Db, before_ts: i64) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO article_state (article, is_read)
           SELECT pk, 1 FROM article WHERE published_at <= ?
           ON CONFLICT(article) DO UPDATE SET is_read = 1"#,
        before_ts,
    )
    .execute(&db.write)
    .await?;

    Ok(())
}

pub async fn mark_all_read_for_feed(db: &Db, feed_pk: i64, before_ts: i64) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO article_state (article, is_read)
           SELECT pk, 1 FROM article WHERE feed = ? AND published_at <= ?
           ON CONFLICT(article) DO UPDATE SET is_read = 1"#,
        feed_pk,
        before_ts,
    )
    .execute(&db.write)
    .await?;

    Ok(())
}

pub async fn mark_all_read_for_category(db: &Db, category_pk: i64, before_ts: i64) -> DbResult<()> {
    sqlx::query!(
        r#"INSERT INTO article_state (article, is_read)
           SELECT article.pk, 1 FROM article
           JOIN feed ON feed.pk = article.feed
           WHERE feed.category = ? AND article.published_at <= ?
           ON CONFLICT(article) DO UPDATE SET is_read = 1"#,
        category_pk,
        before_ts,
    )
    .execute(&db.write)
    .await?;

    Ok(())
}

pub async fn count_unread_total(db: &Db, scope: FeedScope) -> DbResult<i64> {
    let rec = sqlx::query!(
        r#"SELECT COUNT(*) as "count!: i64" FROM article
           LEFT JOIN article_state ON article_state.article = article.pk
           WHERE (article_state.article IS NULL OR article_state.is_read = 0)
             AND (NOT ? OR EXISTS (SELECT 1 FROM feed WHERE feed.pk = article.feed AND feed.greader_hidden = 0))
             AND NOT EXISTS (
                 SELECT 1 FROM article_filter_match afm
                 JOIN filter f ON f.pk = afm.filter AND f.enabled = 1
                 WHERE afm.article = article.pk
             )"#,
        scope.visible_only(),
    )
    .fetch_one(&db.read)
    .await?;

    Ok(rec.count)
}

pub async fn list_unread_counts_by_feed(
    db: &Db,
    scope: FeedScope,
) -> DbResult<Vec<FeedUnreadCount>> {
    let rows = sqlx::query_as!(
        FeedUnreadCount,
        r#"SELECT article.feed as "feed!: i64", COUNT(*) as "count!: i64"
           FROM article
           LEFT JOIN article_state ON article_state.article = article.pk
           WHERE (article_state.article IS NULL OR article_state.is_read = 0)
             AND (NOT ? OR EXISTS (SELECT 1 FROM feed WHERE feed.pk = article.feed AND feed.greader_hidden = 0))
             AND NOT EXISTS (
                 SELECT 1 FROM article_filter_match afm
                 JOIN filter f ON f.pk = afm.filter AND f.enabled = 1
                 WHERE afm.article = article.pk
             )
           GROUP BY article.feed"#,
        scope.visible_only(),
    )
    .fetch_all(&db.read)
    .await?;

    Ok(rows)
}

pub async fn list_unread_counts_by_category(
    db: &Db,
    scope: FeedScope,
) -> DbResult<Vec<CategoryUnreadCount>> {
    let rows = sqlx::query_as!(
        CategoryUnreadCount,
        r#"SELECT feed.category as "category!: i64", COUNT(*) as "count!: i64"
           FROM article
           JOIN feed ON feed.pk = article.feed
           LEFT JOIN article_state ON article_state.article = article.pk
           WHERE feed.category IS NOT NULL
             AND (article_state.article IS NULL OR article_state.is_read = 0)
             AND (NOT ? OR EXISTS (SELECT 1 FROM feed WHERE feed.pk = article.feed AND feed.greader_hidden = 0))
             AND NOT EXISTS (
                 SELECT 1 FROM article_filter_match afm
                 JOIN filter f ON f.pk = afm.filter AND f.enabled = 1
                 WHERE afm.article = article.pk
             )
           GROUP BY feed.category"#,
        scope.visible_only(),
    )
    .fetch_all(&db.read)
    .await?;

    Ok(rows)
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, JsonSchema)]
pub struct StarredArticles {
    pub url: String,
    pub content: String,
}

pub async fn list_starred_articles(db: &Db) -> DbResult<Vec<StarredArticles>> {
    let rows = sqlx::query_as!(
        StarredArticles,
        r#"SELECT article.url, article.content
           FROM article
           LEFT JOIN  article_state ON article_state.article = article.pk
           WHERE article_state.is_starred = 1"#
    )
    .fetch_all(&db.read)
    .await?;

    Ok(rows)
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, JsonSchema)]
pub struct StarredArticlePreview {
    pub article_id: i64,
    pub feed_id: i64,
    pub feed_name: String,
    pub url: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub published_at: Option<i64>,
    pub retrieved_at: i64,
    pub is_read: bool,
    pub is_starred: bool,
}

pub async fn list_starred_articles_with_feed(db: &Db) -> DbResult<Vec<StarredArticlePreview>> {
    let rows = sqlx::query_as!(
        StarredArticlePreview,
        r#"SELECT article.pk AS article_id, feed.pk AS feed_id, feed.name AS feed_name,
                  article.url, article.title, article.summary,
                  article.published_at, article.retrieved_at,
                  article_state.is_read AS "is_read!: bool",
                  article_state.is_starred AS "is_starred!: bool"
           FROM article
           JOIN feed ON feed.pk = article.feed
           JOIN article_state ON article_state.article = article.pk
           WHERE article_state.is_starred = 1
           ORDER BY article.published_at DESC, article.pk DESC"#
    )
    .fetch_all(&db.read)
    .await?;

    Ok(rows)
}
