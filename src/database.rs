use sqlx::sqlite::SqliteConnectOptions;
use sqlx::sqlite::SqliteJournalMode;
use sqlx::sqlite::SqlitePool;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::sqlite::SqliteSynchronous;
use std::error::Error;
use std::str::FromStr;
use std::time::Duration;

async fn config() -> Result<SqlitePool, Box<dyn Error>> {
    let opts = SqliteConnectOptions::from_str("sqlite://app.db")?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5))
        .pragma("cache_size", "-20000")
        .pragma("temp_store", "MEMORY")
        .optimize_on_close(true, None);

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await?;

    sqlx::migrate!().run(&pool).await?;

    Ok(pool)
}
