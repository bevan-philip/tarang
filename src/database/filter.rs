use super::{Article, Db, DbResult};
use crate::filter::{self, CompiledFilter};
use schemars::JsonSchema;
use serde::Serialize;
use sqlx::{QueryBuilder, Sqlite};

#[derive(Debug, Clone, sqlx::FromRow, Serialize, JsonSchema)]
pub struct Filter {
    pub pk: i64,
    pub name: String,
    pub field: String,
    pub match_type: String,
    pub pattern: String,
    pub enabled: bool,
    pub created_at: i64,
}

pub async fn create_filter(
    db: &Db,
    name: &str,
    field: &str,
    match_type: &str,
    pattern: &str,
    feed_pks: &[i64],
) -> DbResult<Filter> {
    let row = sqlx::query_as!(
        Filter,
        r#"INSERT INTO filter (name, field, match_type, pattern)
           VALUES (?, ?, ?, ?)
           RETURNING pk, name, field, match_type, pattern, enabled as "enabled: bool", created_at"#,
        name,
        field,
        match_type,
        pattern,
    )
    .fetch_one(&db.write)
    .await?;

    replace_filter_feeds(db, row.pk, feed_pks).await?;

    let feeds = (!feed_pks.is_empty()).then(|| feed_pks.iter().copied().collect());
    // The API handler is expected to call filter::validate_pattern (or
    // validate_effective_rule) with this same pattern before persisting, so
    // compile_filter failing here should not happen in practice. This `?`
    // is defense-in-depth, not a hard contract - if it ever does fail, the
    // caller gets an ordinary error instead of a panic.
    let compiled = filter::compile_filter(&row, feeds)?;
    let articles = super::list_all_articles(db).await?;
    record_filter_matches(db, &articles, std::slice::from_ref(&compiled)).await?;

    Ok(row)
}

pub async fn list_filters(db: &Db) -> DbResult<Vec<Filter>> {
    let rows = sqlx::query_as!(
        Filter,
        r#"SELECT pk, name, field, match_type, pattern, enabled as "enabled: bool", created_at FROM filter ORDER BY pk"#
    )
    .fetch_all(&db.read)
    .await?;

    Ok(rows)
}

pub async fn get_filter(db: &Db, pk: i64) -> DbResult<Option<Filter>> {
    let row = sqlx::query_as!(
        Filter,
        r#"SELECT pk, name, field, match_type, pattern, enabled as "enabled: bool", created_at FROM filter WHERE pk = ?"#,
        pk,
    )
    .fetch_optional(&db.read)
    .await?;

    Ok(row)
}

#[derive(Debug, Default)]
pub struct UpdateFilterFields {
    pub name: Option<String>,
    pub field: Option<String>,
    pub match_type: Option<String>,
    pub pattern: Option<String>,
    pub enabled: Option<bool>,
    pub feed_pks: Option<Vec<i64>>,
}

pub async fn update_filter(db: &Db, pk: i64, fields: UpdateFilterFields) -> DbResult<Filter> {
    let row = sqlx::query_as!(
        Filter,
        r#"UPDATE filter
           SET name       = COALESCE(?, name),
               field      = COALESCE(?, field),
               match_type = COALESCE(?, match_type),
               pattern    = COALESCE(?, pattern),
               enabled    = COALESCE(?, enabled)
           WHERE pk = ?
           RETURNING pk, name, field, match_type, pattern, enabled as "enabled: bool", created_at"#,
        fields.name.as_deref(),
        fields.field.as_deref(),
        fields.match_type.as_deref(),
        fields.pattern.as_deref(),
        fields.enabled,
        pk,
    )
    .fetch_one(&db.write)
    .await?;

    if let Some(feed_pks) = &fields.feed_pks {
        replace_filter_feeds(db, pk, feed_pks).await?;
    }

    if fields.field.is_some()
        || fields.match_type.is_some()
        || fields.pattern.is_some()
        || fields.feed_pks.is_some()
    {
        // Rule content or feed scope changed - resweep against every
        // article using the filter's current (possibly just-updated) feed
        // set. Same `?` rationale as create_filter: the API handler
        // validates the effective pattern before calling update_filter, so
        // this is defense-in-depth rather than a hard contract.
        let feeds = match &fields.feed_pks {
            Some(feed_pks) => (!feed_pks.is_empty()).then(|| feed_pks.iter().copied().collect()),
            None => {
                let current = list_filter_feed_pks(db, pk).await?;
                (!current.is_empty()).then(|| current.into_iter().collect())
            }
        };
        let compiled = filter::compile_filter(&row, feeds)?;
        clear_matches_for_filter(db, pk).await?;
        let articles = super::list_all_articles(db).await?;
        record_filter_matches(db, &articles, std::slice::from_ref(&compiled)).await?;
    }

    Ok(row)
}

pub async fn drop_filter(db: &Db, pk: i64) -> DbResult<()> {
    sqlx::query!("DELETE FROM filter WHERE pk = ?", pk)
        .execute(&db.write)
        .await?;

    Ok(())
}

pub async fn list_filter_feed_pks(db: &Db, filter_pk: i64) -> DbResult<Vec<i64>> {
    let rows = sqlx::query_scalar!(
        "SELECT feed FROM filter_feed WHERE filter = ? ORDER BY feed",
        filter_pk
    )
    .fetch_all(&db.read)
    .await?;

    Ok(rows)
}

pub async fn list_all_filter_feed_pairs(db: &Db) -> DbResult<Vec<(i64, i64)>> {
    let rows = sqlx::query!("SELECT filter, feed FROM filter_feed")
        .fetch_all(&db.read)
        .await?;

    Ok(rows.into_iter().map(|r| (r.filter, r.feed)).collect())
}

pub async fn replace_filter_feeds(db: &Db, filter_pk: i64, feed_pks: &[i64]) -> DbResult<()> {
    sqlx::query!("DELETE FROM filter_feed WHERE filter = ?", filter_pk)
        .execute(&db.write)
        .await?;

    if feed_pks.is_empty() {
        return Ok(());
    }

    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new("INSERT INTO filter_feed (filter, feed) ");

    qb.push_values(feed_pks, |mut b, feed_pk| {
        b.push_bind(filter_pk).push_bind(feed_pk);
    });

    qb.build().execute(&db.write).await?;

    Ok(())
}

pub async fn clear_matches_for_filter(db: &Db, filter_pk: i64) -> DbResult<()> {
    sqlx::query!(
        "DELETE FROM article_filter_match WHERE filter = ?",
        filter_pk
    )
    .execute(&db.write)
    .await?;

    Ok(())
}

pub async fn clear_matches_for_articles(db: &Db, article_pks: &[i64]) -> DbResult<()> {
    if article_pks.is_empty() {
        return Ok(());
    }

    let mut qb: QueryBuilder<Sqlite> =
        QueryBuilder::new("DELETE FROM article_filter_match WHERE article IN (");

    let mut separated = qb.separated(", ");
    for pk in article_pks {
        separated.push_bind(pk);
    }
    qb.push(")");

    qb.build().execute(&db.write).await?;

    Ok(())
}

pub async fn list_articles_matched_by_filter(db: &Db, filter_pk: i64) -> DbResult<Vec<Article>> {
    let articles = sqlx::query_as!(
        Article,
        r#"SELECT article.pk, article.feed, article.url, article.guid, article.title,
                  article.content, article.summary, article.published_at, article.retrieved_at
           FROM article
           JOIN article_filter_match afm ON afm.article = article.pk
           WHERE afm.filter = ?
           ORDER BY article.published_at DESC"#,
        filter_pk,
    )
    .fetch_all(&db.read)
    .await?;

    Ok(articles)
}

pub async fn record_filter_matches(
    db: &Db,
    articles: &[Article],
    filters: &[CompiledFilter],
) -> DbResult<()> {
    if articles.is_empty() || filters.is_empty() {
        return Ok(());
    }

    let pairs = filter::compute_filter_matches(articles, filters);

    if pairs.is_empty() {
        return Ok(());
    }

    let mut qb: QueryBuilder<Sqlite> =
        QueryBuilder::new("INSERT OR IGNORE INTO article_filter_match (article, filter) ");

    qb.push_values(pairs, |mut b, (article_pk, filter_pk)| {
        b.push_bind(article_pk).push_bind(filter_pk);
    });

    qb.build().execute(&db.write).await?;

    Ok(())
}
