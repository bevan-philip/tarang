use super::{Category, Db, DbResult};

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

pub async fn list_categories_for_feed(db: &Db, feed_pk: i64) -> DbResult<Vec<Category>> {
    let categories = sqlx::query_as!(
        Category,
        r#"SELECT category.pk, category.name
           FROM category
           INNER JOIN feed_category ON feed_category.category = category.pk
           WHERE feed_category.feed = ?
           ORDER BY category.name"#,
        feed_pk,
    )
    .fetch_all(&db.read)
    .await?;

    Ok(categories)
}
