use super::{Db, DbResult, clear_matches_for_articles, record_filter_matches};
use crate::filter::CompiledFilter;
use chrono::Utc;
use schemars::JsonSchema;
use serde::Serialize;
use sqlx::{QueryBuilder, Sqlite};

#[derive(Debug, Clone, sqlx::FromRow, Serialize, JsonSchema)]
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
    filters: &[CompiledFilter],
) -> DbResult<Vec<Article>> {
    if articles.is_empty() {
        return Ok(Vec::new());
    }

    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "INSERT INTO article (feed, url, guid, title, content, summary, published_at) ",
    );

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

    // Recompute (not append): ON CONFLICT means a "new" row may be a
    // re-fetch whose content changed, so a previously-matched article that
    // no longer matches must not stay hidden.
    let pks: Vec<i64> = inserted.iter().map(|a| a.pk).collect();
    clear_matches_for_articles(db, &pks).await?;
    record_filter_matches(db, &inserted, filters).await?;

    Ok(inserted)
}

pub async fn list_all_articles(db: &Db) -> DbResult<Vec<Article>> {
    let articles = sqlx::query_as!(
        Article,
        r#"SELECT pk, feed, url, guid, title, content, summary, published_at, retrieved_at
           FROM article"#,
    )
    .fetch_all(&db.read)
    .await?;

    Ok(articles)
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, JsonSchema)]
pub struct ArticlePreview {
    pub pk: i64,
    pub feed: i64,
    pub url: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub published_at: Option<i64>,
    pub retrieved_at: i64,
    pub is_read: bool,
    pub is_starred: bool,
}

pub async fn list_article_previews_for_feed(db: &Db, feed_pk: i64) -> DbResult<Vec<ArticlePreview>> {
    let articles = sqlx::query_as!(
        ArticlePreview,
        r#"SELECT article.pk, article.feed, article.url, article.title, article.summary,
                  article.published_at, article.retrieved_at,
                  COALESCE(article_state.is_read, 0) as "is_read!: bool",
                  COALESCE(article_state.is_starred, 0) as "is_starred!: bool"
           FROM article
           LEFT JOIN article_state ON article_state.article = article.pk
           WHERE article.feed = ?
             AND NOT EXISTS (
                 SELECT 1 FROM article_filter_match afm
                 JOIN filter f ON f.pk = afm.filter AND f.enabled = 1
                 WHERE afm.article = article.pk
             )
           ORDER BY article.published_at DESC"#,
        feed_pk,
    )
    .fetch_all(&db.read)
    .await?;

    Ok(articles)
}

pub async fn list_article_previews_for_feeds(
    db: &Db,
    limit_per_feed: i64,
) -> DbResult<Vec<ArticlePreview>> {
    let articles = sqlx::query_as!(
        ArticlePreview,
        r#"SELECT pk, feed, url, title, summary, published_at, retrieved_at,
                  is_read as "is_read!: bool", is_starred as "is_starred!: bool"
           FROM (
               SELECT
                   article.pk AS pk,
                   article.feed AS feed,
                   article.url AS url,
                   article.title AS title,
                   article.summary AS summary,
                   article.published_at AS published_at,
                   article.retrieved_at AS retrieved_at,
                   COALESCE(article_state.is_read, 0) AS is_read,
                   COALESCE(article_state.is_starred, 0) AS is_starred,
                   ROW_NUMBER() OVER (
                       PARTITION BY article.feed ORDER BY article.published_at DESC
                   ) AS rn
               FROM article
               LEFT JOIN article_state ON article_state.article = article.pk
               WHERE NOT EXISTS (
                   SELECT 1 FROM article_filter_match afm
                   JOIN filter f ON f.pk = afm.filter AND f.enabled = 1
                   WHERE afm.article = article.pk
               )
           )
           WHERE rn <= ?
           ORDER BY feed, published_at DESC"#,
        limit_per_feed,
    )
    .fetch_all(&db.read)
    .await?;

    Ok(articles)
}

#[derive(Debug, Clone, sqlx::FromRow, Serialize, JsonSchema)]
pub struct ArticleWithState {
    pub pk: i64,
    pub feed: i64,
    pub url: String,
    pub guid: String,
    pub title: Option<String>,
    pub content: String,
    pub summary: Option<String>,
    pub published_at: Option<i64>,
    pub retrieved_at: i64,
    pub is_read: bool,
    pub is_starred: bool,
}

#[derive(Debug, Default)]
pub struct ArticleQuery {
    pub feed: Option<i64>,
    pub category: Option<i64>,
    pub unread_only: bool,
    pub starred_only: bool,
    pub published_after: Option<i64>,
    pub published_before: Option<i64>,
    pub cursor: Option<i64>,
    pub ascending: bool,
    pub limit: i64,
}

const ARTICLE_WITH_STATE_COLUMNS: &str = r#"
    article.pk, article.feed, article.url, article.guid, article.title,
    article.content, article.summary, article.published_at, article.retrieved_at,
    COALESCE(article_state.is_read, 0) as is_read,
    COALESCE(article_state.is_starred, 0) as is_starred
"#;

/// Strict exclusion: hides any article with an enabled matching filter,
/// with no starred bypass. Used everywhere except star-specific views.
const FILTER_EXCLUSION: &str = r#" AND NOT EXISTS (
        SELECT 1 FROM article_filter_match afm
        JOIN filter f ON f.pk = afm.filter AND f.enabled = 1
        WHERE afm.article = article.pk
    )"#;

pub async fn list_articles_by_query(db: &Db, q: &ArticleQuery) -> DbResult<Vec<ArticleWithState>> {
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT ");
    qb.push(ARTICLE_WITH_STATE_COLUMNS);
    qb.push(" FROM article LEFT JOIN article_state ON article_state.article = article.pk");

    if q.category.is_some() {
        qb.push(" JOIN feed ON feed.pk = article.feed");
    }

    qb.push(" WHERE 1 = 1");

    if let Some(feed) = q.feed {
        qb.push(" AND article.feed = ").push_bind(feed);
    }
    if let Some(category) = q.category {
        qb.push(" AND feed.category = ").push_bind(category);
    }
    if q.unread_only {
        qb.push(" AND (article_state.article IS NULL OR article_state.is_read = 0)");
    }
    if q.starred_only {
        qb.push(" AND article_state.is_starred = 1");
    } else {
        // Starred-only result sets are already all-starred, which is
        // exactly the "starring bypasses the filter" case for this path -
        // so the exclusion only applies when we're not in that mode.
        qb.push(FILTER_EXCLUSION);
    }
    if let Some(after) = q.published_after {
        qb.push(" AND article.published_at > ").push_bind(after);
    }
    if let Some(before) = q.published_before {
        qb.push(" AND article.published_at < ").push_bind(before);
    }
    if let Some(cursor) = q.cursor {
        if q.ascending {
            qb.push(" AND article.pk > ").push_bind(cursor);
        } else {
            qb.push(" AND article.pk < ").push_bind(cursor);
        }
    }

    if q.ascending {
        qb.push(" ORDER BY article.pk ASC");
    } else {
        qb.push(" ORDER BY article.pk DESC");
    }

    qb.push(" LIMIT ").push_bind(q.limit);

    let articles = qb
        .build_query_as::<ArticleWithState>()
        .fetch_all(&db.read)
        .await?;

    Ok(articles)
}

/// Starred-bypass exclusion: hides a filtered article unless it's starred.
/// Used by list_articles_by_pks, which has no caller-supplied "mode" - the
/// bypass has to be decided per-row from the article's own starred state.
const FILTER_EXCLUSION_UNLESS_STARRED: &str = r#" AND (
        COALESCE(article_state.is_starred, 0) = 1
        OR NOT EXISTS (
            SELECT 1 FROM article_filter_match afm
            JOIN filter f ON f.pk = afm.filter AND f.enabled = 1
            WHERE afm.article = article.pk
        )
    )"#;

pub async fn list_articles_by_pks(db: &Db, pks: &[i64]) -> DbResult<Vec<ArticleWithState>> {
    if pks.is_empty() {
        return Ok(Vec::new());
    }

    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT ");
    qb.push(ARTICLE_WITH_STATE_COLUMNS);
    qb.push(
        " FROM article LEFT JOIN article_state ON article_state.article = article.pk WHERE article.pk IN (",
    );

    let mut separated = qb.separated(", ");
    for pk in pks {
        separated.push_bind(pk);
    }
    qb.push(")");

    qb.push(FILTER_EXCLUSION_UNLESS_STARRED);

    let articles = qb
        .build_query_as::<ArticleWithState>()
        .fetch_all(&db.read)
        .await?;

    Ok(articles)
}
