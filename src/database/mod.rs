use sqlx::sqlite::SqliteConnectOptions;
use sqlx::sqlite::SqliteJournalMode;
use sqlx::sqlite::SqlitePool;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::sqlite::SqliteSynchronous;
use std::str::FromStr;
use std::time::Duration;

mod article;
mod category;
mod feed;

pub use article::*;
pub use category::*;
pub use feed::*;

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

#[derive(Clone)]
pub struct Db {
    pub read: SqlitePool,
    pub write: SqlitePool,
}

pub async fn config() -> DbResult<Db> {
    let base_opts = SqliteConnectOptions::from_str("sqlite://app.db")?
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5))
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
