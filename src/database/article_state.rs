use super::{Db, DbResult};
use serde::Serialize;
use sqlx::{QueryBuilder, Sqlite};

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

pub async fn count_unread_total(db: &Db) -> DbResult<i64> {
    let rec = sqlx::query!(
        r#"SELECT COUNT(*) as "count!: i64" FROM article
           LEFT JOIN article_state ON article_state.article = article.pk
           WHERE (article_state.article IS NULL OR article_state.is_read = 0)
             AND NOT EXISTS (
                 SELECT 1 FROM article_filter_match afm
                 JOIN filter f ON f.pk = afm.filter AND f.enabled = 1
                 WHERE afm.article = article.pk
             )"#
    )
    .fetch_one(&db.read)
    .await?;

    Ok(rec.count)
}

pub async fn list_unread_counts_by_feed(db: &Db) -> DbResult<Vec<FeedUnreadCount>> {
    let rows = sqlx::query_as!(
        FeedUnreadCount,
        r#"SELECT article.feed as "feed!: i64", COUNT(*) as "count!: i64"
           FROM article
           LEFT JOIN article_state ON article_state.article = article.pk
           WHERE (article_state.article IS NULL OR article_state.is_read = 0)
             AND NOT EXISTS (
                 SELECT 1 FROM article_filter_match afm
                 JOIN filter f ON f.pk = afm.filter AND f.enabled = 1
                 WHERE afm.article = article.pk
             )
           GROUP BY article.feed"#
    )
    .fetch_all(&db.read)
    .await?;

    Ok(rows)
}

pub async fn list_unread_counts_by_category(db: &Db) -> DbResult<Vec<CategoryUnreadCount>> {
    let rows = sqlx::query_as!(
        CategoryUnreadCount,
        r#"SELECT feed.category as "category!: i64", COUNT(*) as "count!: i64"
           FROM article
           JOIN feed ON feed.pk = article.feed
           LEFT JOIN article_state ON article_state.article = article.pk
           WHERE feed.category IS NOT NULL
             AND (article_state.article IS NULL OR article_state.is_read = 0)
             AND NOT EXISTS (
                 SELECT 1 FROM article_filter_match afm
                 JOIN filter f ON f.pk = afm.filter AND f.enabled = 1
                 WHERE afm.article = article.pk
             )
           GROUP BY feed.category"#
    )
    .fetch_all(&db.read)
    .await?;

    Ok(rows)
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
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

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct StarredArticlesWithFeed {
    pub pk: i64,
    pub url: String,
    pub content: String,
}

pub async fn list_starred_articles_with_feed(db: &Db) -> DbResult<Vec<StarredArticlesWithFeed>> {
    let rows = sqlx::query_as!(
        StarredArticlesWithFeed,
        r#"SELECT feed.pk, article.url, article.content
           FROM article
           JOIN feed ON feed.pk = article.feed
           LEFT JOIN  article_state ON article_state.article = article.pk
           WHERE article_state.is_starred = 1"#
    )
    .fetch_all(&db.read)
    .await?;

    Ok(rows)
}
