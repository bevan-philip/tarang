use sqlx::sqlite::SqliteConnectOptions;
use sqlx::sqlite::SqliteJournalMode;
use sqlx::sqlite::SqlitePool;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::sqlite::SqliteSynchronous;
use std::str::FromStr;
use std::time::Duration;

mod article;
mod article_state;
mod category;
mod feed;
mod filter;

pub use article::*;
pub use article_state::*;
pub use category::*;
pub use feed::*;
pub use filter::*;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error("doesn't exist: {0}")]
    NotFound(String),
    #[error(transparent)]
    Sqlx(sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("invalid filter pattern: {0}")]
    InvalidPattern(#[from] regex::Error),
}

impl From<sqlx::Error> for DbError {
    fn from(err: sqlx::Error) -> Self {
        match err.as_database_error() {
            Some(db_err) if db_err.is_unique_violation() => {
                DbError::AlreadyExists(db_err.message().to_string())
            }
            _ => DbError::Sqlx(err),
        }
    }
}

pub type DbResult<T> = Result<T, DbError>;

#[derive(Debug, Clone, Copy)]
pub enum FeedScope {
    All,
    GReaderVisible,
}

impl FeedScope {
    fn visible_only(self) -> bool {
        matches!(self, Self::GReaderVisible)
    }
}

#[derive(Clone)]
pub struct Db {
    pub read: SqlitePool,
    pub write: SqlitePool,
}

pub async fn config(db_path: &str, busy_timeout: Duration) -> DbResult<Db> {
    let base_opts = SqliteConnectOptions::from_str(&format!("sqlite://{db_path}"))?
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(busy_timeout)
        .pragma("cache_size", "-20000")
        .pragma("temp_store", "MEMORY");

    let write_opts = base_opts
        .clone()
        .create_if_missing(true)
        .optimize_on_close(true, None);

    let write = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(write_opts)
        .await?;

    sqlx::migrate!().run(&write).await?;

    let read_opts = base_opts.read_only(true);

    let read = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(read_opts)
        .await?;

    Ok(Db { read, write })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn config_runs_migrations_and_read_pool_is_read_only() {
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("tarang-database-config-{pid}.db"));
        let _ = std::fs::remove_file(&path);

        let db = config(path.to_str().unwrap(), Duration::from_secs(5))
            .await
            .unwrap();

        // A known table from migrations must be queryable.
        sqlx::query("SELECT * FROM feed")
            .fetch_all(&db.read)
            .await
            .unwrap();

        // The read pool must reject writes.
        let write_result = sqlx::query("INSERT INTO category (name) VALUES ('nope')")
            .execute(&db.read)
            .await;
        assert!(write_result.is_err());

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }
}
