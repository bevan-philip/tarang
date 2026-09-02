use super::{Article, Db, DbResult};
use crate::filter::{self, CompiledFilter};
use serde::Serialize;
use sqlx::{QueryBuilder, Sqlite};

#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
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

    // compile_filter is expected to succeed here: the API handler is
    // required to call filter::validate_pattern with this same pattern
    // before persisting, so a compile failure at this point would indicate
    // a caller bypassed that check.
    let compiled = filter::compile_filter(&row).expect("pattern already validated by caller");
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

pub async fn update_filter(
    db: &Db,
    pk: i64,
    name: Option<&str>,
    field: Option<&str>,
    match_type: Option<&str>,
    pattern: Option<&str>,
    enabled: Option<bool>,
) -> DbResult<Filter> {
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
        name,
        field,
        match_type,
        pattern,
        enabled,
        pk,
    )
    .fetch_one(&db.write)
    .await?;

    if field.is_some() || match_type.is_some() || pattern.is_some() {
        // Rule content changed - resweep against every article. Same
        // expect() rationale as create_filter: the API handler validates
        // the effective pattern before calling update_filter.
        let compiled = filter::compile_filter(&row).expect("pattern already validated by caller");
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

pub async fn record_filter_matches(
    db: &Db,
    articles: &[Article],
    filters: &[CompiledFilter],
) -> DbResult<()> {
    if articles.is_empty() || filters.is_empty() {
        return Ok(());
    }

    let pairs: Vec<(i64, i64)> = articles
        .iter()
        .flat_map(|article| {
            let title = article.title.as_deref().unwrap_or("");
            let content = article.content.as_str();
            filters
                .iter()
                .filter(move |f| filter::matches(f, title, content))
                .map(|f| (article.pk, f.pk))
        })
        .collect();

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
