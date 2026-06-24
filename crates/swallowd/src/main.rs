mod api;
mod caddy;
mod config;
mod db;
mod deploy;
mod docker;
mod error;
mod manifest;
mod models;
mod web;

use anyhow::Context;
use tracing::{info, warn};
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

    let owner_id = models::ensure_default_user(&db)
        .await
        .context("ensuring default user")?;

    let docker = docker::DockerEngine::connect().context("connecting to Docker")?;
    match docker.ping().await {
        Ok(v) => info!(docker_version = %v, "connected to Docker"),
        Err(e) => warn!(error = %e, "Docker not reachable yet — deploys will fail until it is"),
    }

    let caddy = caddy::CaddyClient::new(config.caddy_admin_url.clone());

    let listener = tokio::net::TcpListener::bind(config.listen_addr)
        .await
        .with_context(|| format!("binding {}", config.listen_addr))?;

    let state = AppState {
        db,
        config,
        docker,
        caddy,
        owner_id,
    };
    let app = web::router(state.clone()).merge(api::router(state));

    info!("swallowd is listening");
    axum::serve(listener, app)
        .await
        .context("running http server")?;

    Ok(())
}
