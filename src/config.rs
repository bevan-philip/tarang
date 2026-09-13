use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse config: {0}")]
    Parse(#[from] toml::de::Error),
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub http: HttpConfig,
    pub sync: SyncConfig,
    pub backup: BackupConfig,
    pub discovery: DiscoveryConfig,
}

impl Config {
    /// Parses TOML text into a Config. Missing fields fall back to defaults.
    pub fn from_toml_str(contents: &str) -> Result<Config, ConfigError> {
        Ok(toml::from_str(contents)?)
    }

    pub fn load_from(path: &Path) -> Result<Config, ConfigError> {
        let contents = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        Config::from_toml_str(&contents)
    }

    /// Loads configuration from `config.toml` in the current working directory.
    /// A missing file is not an error; every setting falls back to its default.
    pub fn load() -> Result<Config, ConfigError> {
        Config::load_from(Path::new(CONFIG_FILE))
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 3000,
        }
    }
}

impl ServerConfig {
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct DatabaseConfig {
    pub path: String,
    pub busy_timeout_secs: u64,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        DatabaseConfig {
            path: "app.db".to_string(),
            busy_timeout_secs: 5,
        }
    }
}

impl DatabaseConfig {
    pub fn busy_timeout(&self) -> Duration {
        Duration::from_secs(self.busy_timeout_secs)
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct HttpConfig {
    pub timeout_secs: u64,
}

impl Default for HttpConfig {
    fn default() -> Self {
        HttpConfig { timeout_secs: 30 }
    }
}

impl HttpConfig {
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct SyncConfig {
    pub poll_interval_secs: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        SyncConfig {
            poll_interval_secs: 300,
        }
    }
}

impl SyncConfig {
    pub fn poll_interval(&self) -> Duration {
        Duration::from_secs(self.poll_interval_secs)
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct BackupConfig {
    /// Whether periodic backups are enabled. Off by default.
    pub enabled: bool,
    /// Number of poll intervals between backups (a backup runs after every
    /// Nth poll completes). 0 disables backups regardless of `enabled`.
    pub every_n_polls: u32,
    /// Destination file for the backup. Overwritten on every run. The
    /// parent directory is NOT created automatically — it must exist.
    pub path: String,
}

impl Default for BackupConfig {
    fn default() -> Self {
        BackupConfig {
            enabled: false,
            every_n_polls: 12,
            path: "backup.db".to_string(),
        }
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct DiscoveryConfig {
    /// When resolving a YouTube channel, point at the uploads playlist that
    /// excludes Shorts (UULF...) instead of the default all-uploads feed.
    pub youtube_exclude_shorts: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_toml_falls_back_to_defaults() {
        let config = Config::from_toml_str("").unwrap();
        assert_eq!(config.server.port, 3000);
        assert_eq!(config.database.path, "app.db");
        assert_eq!(config.http.timeout_secs, 30);
        assert_eq!(config.sync.poll_interval_secs, 300);
        assert!(!config.backup.enabled);
        assert!(!config.discovery.youtube_exclude_shorts);
    }

    #[test]
    fn discovery_partial_override_only_changes_specified_fields() {
        let config = Config::from_toml_str(
            r#"
            [discovery]
            youtube_exclude_shorts = true
            "#,
        )
        .unwrap();
        assert!(config.discovery.youtube_exclude_shorts);
    }

    #[test]
    fn partial_overrides_only_change_specified_fields() {
        let config = Config::from_toml_str(
            r#"
            [server]
            port = 8080

            [backup]
            enabled = true
            "#,
        )
        .unwrap();
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.host, "127.0.0.1");
        assert!(config.backup.enabled);
        assert_eq!(config.backup.every_n_polls, 12);
    }

    #[test]
    fn invalid_toml_is_a_parse_error() {
        let result = Config::from_toml_str("this is not valid toml [[[");
        assert!(matches!(result, Err(ConfigError::Parse(_))));
    }

    #[test]
    fn load_from_missing_path_falls_back_to_defaults() {
        let path = std::env::temp_dir().join(format!(
            "tarang-config-test-missing-{}.toml",
            std::process::id()
        ));
        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.server.port, 3000);
    }

    #[test]
    fn load_from_reads_real_file() {
        let path = std::env::temp_dir().join(format!(
            "tarang-config-test-real-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "[server]\nport = 4242\n").unwrap();

        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.server.port, 4242);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn server_config_bind_addr_formats_host_and_port() {
        let config = ServerConfig {
            host: "0.0.0.0".to_string(),
            port: 8080,
        };
        assert_eq!(config.bind_addr(), "0.0.0.0:8080");
    }

    #[test]
    fn database_config_busy_timeout_converts_secs_to_duration() {
        let config = DatabaseConfig {
            path: "app.db".to_string(),
            busy_timeout_secs: 5,
        };
        assert_eq!(config.busy_timeout(), Duration::from_secs(5));
    }

    #[test]
    fn http_config_timeout_converts_secs_to_duration() {
        let config = HttpConfig { timeout_secs: 30 };
        assert_eq!(config.timeout(), Duration::from_secs(30));
    }

    #[test]
    fn sync_config_poll_interval_converts_secs_to_duration() {
        let config = SyncConfig {
            poll_interval_secs: 300,
        };
        assert_eq!(config.poll_interval(), Duration::from_secs(300));
    }
}
