mod api;
mod auth;
mod caddy;
mod config;
mod db;
mod deploy;
mod docker;
mod error;
mod hooks;
mod manifest;
mod metrics;
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
    auth::ensure_admin_password(&db, owner_id)
        .await
        .context("ensuring admin password")?;

    let docker = docker::DockerEngine::connect().context("connecting to Docker")?;
    match docker.ping().await {
        Ok(v) => info!(docker_version = %v, "connected to Docker"),
        Err(e) => warn!(error = %e, "Docker not reachable yet — deploys will fail until it is"),
    }

    let caddy = caddy::CaddyClient::new(config.caddy_admin_url.clone());
    // Make sure Caddy has the base :80/:443 server up front so the first request
    // to an app's subdomain works even before the first deploy. Best-effort:
    // Caddy may not be running yet, and deploys re-run this anyway.
    match caddy.ensure_bootstrap().await {
        Ok(()) => info!("caddy base server ready"),
        Err(e) => warn!(error = %e, "could not bootstrap Caddy (is it running?)"),
    }

    // Bring deployed apps back after a host reboot: restart surviving
    // containers, clean up vanished ones, and re-apply Caddy routes (Caddy comes
    // up with an empty config). Best-effort — the daemon serves regardless.
    let reconciler = deploy::Deployer {
        db: db.clone(),
        docker: docker.clone(),
        caddy: caddy.clone(),
        config: config.clone(),
    };
    match reconciler.reconcile().await {
        Ok(()) => info!("reconciled instances and routes after startup"),
        Err(e) => warn!(error = %e, "startup reconciliation failed"),
    }

    // Background sampler: records CPU/memory time-series for running instances.
    metrics::spawn(db.clone(), docker.clone());

    let listener = tokio::net::TcpListener::bind(config.listen_addr)
        .await
        .with_context(|| format!("binding {}", config.listen_addr))?;

    let state = AppState {
        db,
        config,
        docker,
        caddy,
        sessions: auth::SessionStore::default(),
        owner_id,
    };
    let app = web::router(state.clone())
        .merge(api::router(state.clone()))
        .merge(auth::router(state.clone()))
        .merge(hooks::router(state.clone()))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));

    info!("swallowd is listening");
    axum::serve(listener, app)
        .await
        .context("running http server")?;

    Ok(())
}
