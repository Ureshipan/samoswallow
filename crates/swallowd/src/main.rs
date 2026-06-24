mod api;
mod config;
mod db;

use anyhow::Context;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::api::AppState;
use crate::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("SWALLOW_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env().context("loading configuration")?;
    info!(?config.listen_addr, base_domain = %config.base_domain, "starting swallowd");

    let db = db::connect(&config.database_url)
        .await
        .context("connecting to database")?;

    let listener = tokio::net::TcpListener::bind(config.listen_addr)
        .await
        .with_context(|| format!("binding {}", config.listen_addr))?;

    let state = AppState { db, config };
    let app = api::router(state);

    info!("swallowd is listening");
    axum::serve(listener, app)
        .await
        .context("running http server")?;

    Ok(())
}
