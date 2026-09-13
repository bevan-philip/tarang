use crate::database::Db;

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

pub type BackupResult<T> = Result<T, BackupError>;

pub async fn backup_database(db: &Db, dest: &str) -> BackupResult<()> {
    let tmp_path = format!("{dest}.tmp");

    match std::fs::remove_file(&tmp_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }

    sqlx::query!("VACUUM INTO ?", tmp_path)
        .execute(&db.write)
        .await?;

    std::fs::rename(&tmp_path, dest)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_db(path: &str) -> Db {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&format!("sqlite://{path}?mode=rwc"))
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        Db {
            read: pool.clone(),
            write: pool,
        }
    }

    #[tokio::test]
    async fn backup_database_writes_destination_file() {
        let pid = std::process::id();
        let src_path = std::env::temp_dir().join(format!("tarang-backup-src-{pid}.db"));
        let dest_path = std::env::temp_dir().join(format!("tarang-backup-dest-{pid}.db"));
        let _ = std::fs::remove_file(&src_path);
        let _ = std::fs::remove_file(&dest_path);

        let db = test_db(src_path.to_str().unwrap()).await;
        backup_database(&db, dest_path.to_str().unwrap())
            .await
            .unwrap();

        assert!(dest_path.exists());

        drop(db);
        let _ = std::fs::remove_file(&src_path);
        let _ = std::fs::remove_file(format!("{}-wal", src_path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", src_path.display()));
        let _ = std::fs::remove_file(&dest_path);
    }
}
