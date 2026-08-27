use serde::Deserialize;
use std::time::Duration;

const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read {CONFIG_FILE}: {0}")]
    Read(#[from] std::io::Error),
    #[error("failed to parse {CONFIG_FILE}: {0}")]
    Parse(#[from] toml::de::Error),
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub http: HttpConfig,
    pub sync: SyncConfig,
}

impl Config {
    /// Loads configuration from `config.toml` in the current working directory.
    /// A missing file is not an error; every setting falls back to its default.
    pub fn load() -> Result<Config, ConfigError> {
        let contents = match std::fs::read_to_string(CONFIG_FILE) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(e.into()),
        };
        Ok(toml::from_str(&contents)?)
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
