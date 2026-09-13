use super::{Db, DbResult, FeedScope};
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Clone, sqlx::FromRow, Serialize, JsonSchema)]
pub struct Category {
    pub pk: i64,
    pub name: String,
}

pub async fn create_category(db: &Db, name: &str) -> DbResult<Category> {
    let category = sqlx::query_as!(
        Category,
        r#"INSERT INTO category (name) VALUES (?)
           RETURNING pk, name"#,
        name,
    )
    .fetch_one(&db.write)
    .await?;

    Ok(category)
}

pub async fn list_categories(db: &Db, scope: FeedScope) -> DbResult<Vec<Category>> {
    let visible_only = scope.visible_only();
    let categories = sqlx::query_as!(Category, r#"SELECT pk, name FROM category
        WHERE NOT ? OR EXISTS (SELECT 1 FROM feed WHERE feed.category = category.pk AND feed.greader_hidden = 0)
        ORDER BY name"#, visible_only)
        .fetch_all(&db.read)
        .await?;

    Ok(categories)
}

pub async fn get_category_by_name(db: &Db, name: &str) -> DbResult<Option<Category>> {
    let category = sqlx::query_as!(
        Category,
        r#"SELECT pk, name FROM category WHERE name = ?"#,
        name,
    )
    .fetch_optional(&db.read)
    .await?;

    Ok(category)
}

pub async fn rename_category(db: &Db, pk: i64, name: &str) -> DbResult<Category> {
    let category = sqlx::query_as!(
        Category,
        r#"UPDATE category SET name = ? WHERE pk = ? RETURNING pk, name"#,
        name,
        pk,
    )
    .fetch_one(&db.write)
    .await?;

    Ok(category)
}

pub async fn drop_category(db: &Db, pk: i64) -> DbResult<()> {
    sqlx::query!("DELETE FROM category WHERE pk = ?", pk)
        .execute(&db.write)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_db() -> Db {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        Db {
            read: pool.clone(),
            write: pool,
        }
    }

    #[tokio::test]
    async fn get_category_by_name_finds_existing() {
        let db = test_db().await;
        let created = create_category(&db, "News").await.unwrap();

        let found = get_category_by_name(&db, "News").await.unwrap().unwrap();
        assert_eq!(found.pk, created.pk);
    }

    #[tokio::test]
    async fn get_category_by_name_missing_returns_none() {
        let db = test_db().await;
        assert!(
            get_category_by_name(&db, "Missing")
                .await
                .unwrap()
                .is_none()
        );
    }
}
