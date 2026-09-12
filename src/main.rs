use tarang::config::Config;
use tarang::{AppState, backup, build_app, database, sync};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::load().expect("failed to load config.toml");

    let db = database::config(&config.database.path, config.database.busy_timeout())
        .await
        .expect("failed to initialise db");

    let http = reqwest::Client::builder()
        .timeout(config.http.timeout())
        .build()
        .expect("failed to build http client");

    let sync_db = db.clone();
    let sync_http = http.clone();
    let sync_interval = config.sync.poll_interval();

    let backup_enabled = config.backup.enabled;
    let backup_every_n_polls = config.backup.every_n_polls;
    let backup_path = config.backup.path.clone();
    let backup_db = db.clone();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(sync_interval);
        let mut polls_since_backup: u32 = 0;
        loop {
            match sync::sync_feeds_now(&sync_db, &sync_http).await {
                Ok(summary) if summary.failed > 0 => {
                    tracing::warn!(
                        succeeded = summary.succeeded,
                        failed = summary.failed,
                        "sync_feeds completed with failures"
                    );
                }
                Ok(_) => {}
                Err(e) => tracing::error!(error = %e, "sync_feeds failed"),
            }

            if backup_enabled && backup_every_n_polls > 0 {
                polls_since_backup += 1;
                if polls_since_backup >= backup_every_n_polls {
                    polls_since_backup = 0;
                    if let Err(e) = backup::backup_database(&backup_db, &backup_path).await {
                        tracing::error!(error = %e, "backup_database failed");
                    }
                }
            }

            interval.tick().await;
        }
    });

    let app = build_app(AppState { db, http });

    let bind_addr = config.server.bind_addr();

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .unwrap_or_else(|_| panic!("failed to bind to {bind_addr}"));

    println!("listening on http://{bind_addr}");
    axum::serve(listener, app).await.expect("server crashed");
}
