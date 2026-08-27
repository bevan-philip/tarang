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
