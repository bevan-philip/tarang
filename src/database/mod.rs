use sqlx::sqlite::SqliteConnectOptions;
use sqlx::sqlite::SqliteJournalMode;
use sqlx::sqlite::SqlitePool;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::sqlite::SqliteSynchronous;
use std::error::Error;
use std::str::FromStr;
use std::time::Duration;

mod article;
mod category;
mod feed;
mod feed_category;

pub use article::*;
pub use category::*;
pub use feed::*;
pub use feed_category::*;

pub type DbResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

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
