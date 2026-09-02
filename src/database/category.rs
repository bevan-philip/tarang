use super::{Db, DbResult};
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

pub async fn list_categories(db: &Db) -> DbResult<Vec<Category>> {
    let categories = sqlx::query_as!(Category, r#"SELECT pk, name FROM category ORDER BY name"#)
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
