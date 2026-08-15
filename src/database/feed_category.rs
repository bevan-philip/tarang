use super::{Category, Db, DbResult};
use std::collections::HashMap;

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

pub async fn list_categories_for_all_feeds(db: &Db) -> DbResult<HashMap<i64, Vec<Category>>> {
    struct FeedCategoryRow {
        feed: i64,
        pk: i64,
        name: String,
    }

    let rows = sqlx::query_as!(
        FeedCategoryRow,
        r#"SELECT feed_category.feed as feed, category.pk, category.name
           FROM category
           INNER JOIN feed_category ON feed_category.category = category.pk
           ORDER BY category.name"#,
    )
    .fetch_all(&db.read)
    .await?;

    let mut categories_by_feed: HashMap<i64, Vec<Category>> = HashMap::new();
    for row in rows {
        categories_by_feed
            .entry(row.feed)
            .or_default()
            .push(Category {
                pk: row.pk,
                name: row.name,
            });
    }

    Ok(categories_by_feed)
}
