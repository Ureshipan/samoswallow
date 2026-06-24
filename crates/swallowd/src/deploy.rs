use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;
use sqlx::SqlitePool;
use tracing::{info, warn};

use crate::caddy::CaddyClient;
use crate::config::Config;
use crate::docker::DockerEngine;
use crate::manifest::Manifest;
use crate::models::{set_app_manifest, App, Build, Instance};

/// Everything the deploy pipeline needs to do its job.
#[derive(Clone)]
pub struct Deployer {
    pub db: SqlitePool,
    pub docker: DockerEngine,
    pub caddy: CaddyClient,
    pub config: Config,
}

#[derive(Debug, Serialize)]
pub struct DeployResult {
    pub build_id: i64,
    pub instance_id: i64,
    pub image_tag: String,
    pub host: String,
    pub host_port: u16,
}

impl Deployer {
    /// Full deploy: clone -> read manifest -> build image -> run instance ->
    /// route via Caddy -> retire old instances.
    pub async fn deploy(&self, app_id: i64) -> Result<DeployResult> {
        let app = App::get(&self.db, app_id)
            .await
            .context("loading app")?;

        // 1. Clone the repo at the configured branch.
        let work_dir = self.config.state_dir.join("repos").join(app_id.to_string());
        clone_repo(&app, &work_dir).await?;
        let commit_sha = git_head_sha(&work_dir).await?;
        info!(app = %app.name, %commit_sha, "cloned repo");

        // 2. Read & validate the manifest.
        let manifest_raw = tokio::fs::read_to_string(work_dir.join("swallow.yaml"))
            .await
            .context("reading swallow.yaml from repo")?;
        let manifest = Manifest::parse(&manifest_raw).context("parsing swallow.yaml")?;
        set_app_manifest(&self.db, app_id, &manifest_raw).await?;

        // 3. Build the image, recording a Build row.
        let build_id = Build::create(&self.db, app_id, &commit_sha).await?;
        let image_tag = format!("samoswallow/{}:{}", app.name, &commit_sha[..short_len(&commit_sha)]);

        let build_log = match self
            .docker
            .build_image(&work_dir, &manifest.dockerfile, &image_tag)
            .await
        {
            Ok(log) => {
                Build::mark_success(&self.db, build_id, &image_tag, &log).await?;
                log
            }
            Err(e) => {
                Build::mark_failed(&self.db, build_id, &e.to_string()).await?;
                return Err(e).context("building image");
            }
        };
        info!(app = %app.name, %image_tag, "image built ({} bytes of log)", build_log.len());

        // 4. Run a new instance on a free host port.
        let host_port = pick_free_port().context("allocating host port")?;
        let container_name = format!(
            "sw-{}-{}",
            sanitize(&app.name),
            uuid::Uuid::new_v4().simple()
        );
        let running = self
            .docker
            .run_container(&image_tag, &container_name, &manifest, host_port)
            .await
            .context("running container")?;
        let instance_id = Instance::create(
            &self.db,
            app_id,
            build_id,
            &running.container_id,
            running.host_port as i64,
        )
        .await?;
        info!(app = %app.name, instance_id, host_port, "instance started");

        // 5. Point Caddy at the new instance (best-effort in dev).
        let host = format!("{}.{}", app.domain, self.config.base_domain);
        let upstream = format!("127.0.0.1:{host_port}");
        if let Err(e) = self
            .caddy
            .sync_app_route(app_id, &host, &[upstream])
            .await
        {
            warn!(app = %app.name, error = %e, "could not update Caddy route (is Caddy running?)");
        }

        // 6. Retire previously-running instances (simple stop-old-after-new).
        self.retire_old_instances(app_id, instance_id).await;

        Ok(DeployResult {
            build_id,
            instance_id,
            image_tag,
            host,
            host_port,
        })
    }

    /// Stop and remove every running instance of the app except `keep`.
    async fn retire_old_instances(&self, app_id: i64, keep: i64) {
        let running = match Instance::list_running_for_app(&self.db, app_id).await {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "could not list running instances");
                return;
            }
        };
        for inst in running.into_iter().filter(|i| i.id != keep) {
            if let Some(cid) = &inst.container_id {
                if let Err(e) = self.docker.stop_and_remove(cid).await {
                    warn!(instance = inst.id, error = %e, "failed to stop old instance");
                }
            }
            let _ = Instance::set_status(&self.db, inst.id, "stopped").await;
        }
    }
}

async fn clone_repo(app: &App, work_dir: &Path) -> Result<()> {
    if work_dir.exists() {
        tokio::fs::remove_dir_all(work_dir)
            .await
            .context("clearing previous clone")?;
    }
    if let Some(parent) = work_dir.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }

    let status = tokio::process::Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "--branch",
            &app.default_branch,
            &app.repo_url,
        ])
        .arg(work_dir)
        .status()
        .await
        .context("spawning git clone")?;
    anyhow::ensure!(status.success(), "git clone failed for {}", app.repo_url);
    Ok(())
}

async fn git_head_sha(work_dir: &Path) -> Result<String> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(work_dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .await
        .context("git rev-parse")?;
    anyhow::ensure!(out.status.success(), "git rev-parse failed");
    Ok(String::from_utf8(out.stdout)?.trim().to_string())
}

/// Ask the OS for a free TCP port by binding to :0 and releasing it.
fn pick_free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    Ok(port)
}

fn short_len(sha: &str) -> usize {
    sha.len().min(12)
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}
