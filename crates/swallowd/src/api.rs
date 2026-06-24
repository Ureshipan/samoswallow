use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::caddy::CaddyClient;
use crate::config::Config;
use crate::deploy::{DeployResult, Deployer};
use crate::docker::{DockerEngine, StatsSnapshot};
use crate::error::{ApiError, ApiResult};
use crate::models::{App, Build, Instance};

/// Shared application state handed to every request handler.
#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub config: Config,
    pub docker: DockerEngine,
    pub caddy: CaddyClient,
    pub sessions: crate::auth::SessionStore,
    /// Single-user mode: every app belongs to this user.
    pub owner_id: i64,
}

impl AppState {
    pub fn deployer(&self) -> Deployer {
        Deployer {
            db: self.db.clone(),
            docker: self.docker.clone(),
            caddy: self.caddy.clone(),
            config: self.config.clone(),
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/version", get(version))
        .route("/api/apps", get(list_apps).post(create_app))
        .route("/api/apps/{id}", get(get_app).delete(delete_app))
        .route("/api/apps/{id}/deploy", post(deploy_app))
        .route("/api/apps/{id}/builds", get(list_builds))
        .route("/api/builds/{id}/rollback", post(rollback_build))
        .route("/api/apps/{id}/instances", get(list_instances))
        .route("/api/instances/{id}/restart", post(restart_instance))
        .route("/api/instances/{id}/stop", post(stop_instance))
        .route("/api/instances/{id}/logs", get(instance_logs))
        .route("/api/instances/{id}/stats", get(instance_stats))
        .with_state(state)
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    db: &'static str,
}

/// Liveness + DB readiness probe.
async fn healthz(State(state): State<AppState>) -> Json<Health> {
    let db = match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => "ok",
        Err(_) => "down",
    };
    Json(Health { status: "ok", db })
}

#[derive(Serialize)]
struct Version {
    name: &'static str,
    version: &'static str,
    base_domain: String,
}

async fn version(State(state): State<AppState>) -> Json<Version> {
    Json(Version {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        base_domain: state.config.base_domain.clone(),
    })
}

// --- Apps ------------------------------------------------------------------

async fn list_apps(State(state): State<AppState>) -> ApiResult<Json<Vec<App>>> {
    Ok(Json(App::list(&state.db, state.owner_id).await?))
}

async fn get_app(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<App>> {
    Ok(Json(App::get(&state.db, id).await?))
}

#[derive(Deserialize)]
struct CreateApp {
    name: String,
    repo_url: String,
    #[serde(default = "default_branch")]
    default_branch: String,
    domain: String,
}

fn default_branch() -> String {
    "main".to_string()
}

async fn create_app(
    State(state): State<AppState>,
    Json(body): Json<CreateApp>,
) -> ApiResult<(StatusCode, Json<App>)> {
    if body.name.trim().is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    if body.repo_url.trim().is_empty() {
        return Err(ApiError::BadRequest("repo_url is required".into()));
    }
    if body.domain.trim().is_empty() {
        return Err(ApiError::BadRequest("domain is required".into()));
    }

    let app = App::create(
        &state.db,
        state.owner_id,
        body.name.trim(),
        body.repo_url.trim(),
        body.default_branch.trim(),
        body.domain.trim(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(app)))
}

async fn delete_app(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    // Stop any running instances and drop the Caddy route before deleting.
    if let Ok(instances) = Instance::list_running_for_app(&state.db, id).await {
        for inst in instances {
            if let Some(cid) = &inst.container_id {
                let _ = state.docker.stop_and_remove(cid).await;
            }
        }
    }
    let _ = state.caddy.remove_app_route(id).await;

    if App::delete(&state.db, id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

// --- Deploy / builds / instances ------------------------------------------

async fn deploy_app(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<DeployResult>> {
    // Ensure the app exists for a clean 404 before the heavy lifting.
    App::get(&state.db, id).await?;
    let result = state
        .deployer()
        .deploy(id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(result))
}

async fn list_builds(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Vec<Build>>> {
    Ok(Json(Build::list(&state.db, id).await?))
}

async fn rollback_build(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<DeployResult>> {
    let result = state
        .deployer()
        .rollback(id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(result))
}

async fn list_instances(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Vec<Instance>>> {
    Ok(Json(Instance::list_for_app(&state.db, id).await?))
}

async fn restart_instance(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let inst = Instance::get(&state.db, id).await?;
    let cid = inst
        .container_id
        .ok_or_else(|| ApiError::BadRequest("instance has no container".into()))?;
    state.docker.restart(&cid).await.map_err(ApiError::Internal)?;
    Instance::set_status(&state.db, id, "running").await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn stop_instance(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let inst = Instance::get(&state.db, id).await?;
    if let Some(cid) = &inst.container_id {
        state
            .docker
            .stop_and_remove(cid)
            .await
            .map_err(ApiError::Internal)?;
    }
    Instance::set_status(&state.db, id, "stopped").await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct LogsQuery {
    #[serde(default = "default_tail")]
    tail: String,
}

fn default_tail() -> String {
    "200".to_string()
}

#[derive(Serialize)]
struct LogsResponse {
    logs: String,
}

async fn instance_logs(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(q): Query<LogsQuery>,
) -> ApiResult<Json<LogsResponse>> {
    let inst = Instance::get(&state.db, id).await?;
    let cid = inst
        .container_id
        .ok_or_else(|| ApiError::BadRequest("instance has no container".into()))?;
    let logs = state
        .docker
        .logs(&cid, &q.tail)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(LogsResponse { logs }))
}

async fn instance_stats(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<StatsSnapshot>> {
    let inst = Instance::get(&state.db, id).await?;
    let cid = inst
        .container_id
        .ok_or_else(|| ApiError::BadRequest("instance has no container".into()))?;
    let stats = state
        .docker
        .stats_snapshot(&cid)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(stats))
}
