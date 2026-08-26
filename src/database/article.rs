use super::{Db, DbResult};
use chrono::Utc;
use serde::Serialize;
use sqlx::{QueryBuilder, Sqlite};

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct Article {
    pub pk: i64,
    pub feed: i64,
    pub url: String,
    pub guid: String,
    pub title: Option<String>,
    pub content: String,
    pub summary: Option<String>,
    pub published_at: Option<i64>,
    pub retrieved_at: i64,
}

pub struct ParsedArticle {
    pub url: String,
    pub guid: String,
    pub title: Option<String>,
    pub content: String,
    pub summary: Option<String>,
    pub published_at: chrono::DateTime<Utc>,
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
        QueryBuilder::new("INSERT INTO article (feed, url, guid, title, content, summary, published_at) ");

    qb.push_values(articles, |mut b, article| {
        b.push_bind(feed_pk)
            .push_bind(&article.url)
            .push_bind(&article.guid)
            .push_bind(&article.title)
            .push_bind(&article.content)
            .push_bind(&article.summary)
            .push_bind(article.published_at.timestamp());
    });

    qb.push(
        r#" ON CONFLICT(guid) DO UPDATE SET
                url          = excluded.url,
                title        = excluded.title,
                content      = excluded.content,
                summary      = excluded.summary,
                published_at = excluded.published_at,
                retrieved_at = unixepoch()
            RETURNING pk, feed, url, guid, title, content, summary, published_at, retrieved_at"#,
    );

    let inserted = qb.build_query_as::<Article>().fetch_all(&db.write).await?;

    Ok(inserted)
}

pub async fn list_articles_for_feed(db: &Db, feed_pk: i64) -> DbResult<Vec<Article>> {
    let articles = sqlx::query_as!(
        Article,
        r#"SELECT pk, feed, url, guid, title, content, summary, published_at, retrieved_at
           FROM article WHERE feed = ? ORDER BY published_at DESC"#,
        feed_pk,
    )
    .fetch_all(&db.read)
    .await?;

    Ok(articles)
}

pub async fn list_articles_for_feeds(db: &Db, limit_per_feed: i64) -> DbResult<Vec<Article>> {
    let articles = sqlx::query_as!(
        Article,
        r#"SELECT pk, feed, url, guid, title, content, summary, published_at, retrieved_at
           FROM (
               SELECT pk, feed, url, guid, title, content, summary, published_at, retrieved_at,
                      ROW_NUMBER() OVER (PARTITION BY feed ORDER BY published_at DESC) AS rn
               FROM article
           )
           WHERE rn <= ?
           ORDER BY feed, published_at DESC"#,
        limit_per_feed,
    )
    .fetch_all(&db.read)
    .await?;

    Ok(articles)
}
