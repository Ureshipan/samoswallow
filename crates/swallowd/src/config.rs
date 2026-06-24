use std::net::SocketAddr;

/// Runtime configuration for the daemon, sourced from environment variables.
///
/// All variables are prefixed `SWALLOW_`. Sensible defaults let the daemon boot
/// for local development without any configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Address the HTTP API / web UI listens on.
    pub listen_addr: SocketAddr,
    /// SQLite connection string, e.g. `sqlite:///var/lib/samoswallow/state.db`.
    pub database_url: String,
    /// Base domain; app `domain: foo` is served at `foo.<base_domain>`.
    pub base_domain: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let listen_addr = std::env::var("SWALLOW_LISTEN")
            .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
            .parse()?;

        let database_url = std::env::var("SWALLOW_DATABASE_URL")
            .unwrap_or_else(|_| "sqlite://samoswallow.db?mode=rwc".to_string());

        let base_domain =
            std::env::var("SWALLOW_BASE_DOMAIN").unwrap_or_else(|_| "localhost".to_string());

        Ok(Self {
            listen_addr,
            database_url,
            base_domain,
        })
    }
}
