use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use bollard::container::{
    Config, CreateContainerOptions, LogsOptions, RemoveContainerOptions, StartContainerOptions,
    StatsOptions, StopContainerOptions,
};
use bollard::image::BuildImageOptions;
use bollard::models::{HostConfig, PortBinding};
use bollard::Docker;
use futures_util::StreamExt;

use crate::manifest::Manifest;

/// Thin wrapper around the Docker daemon used by the control plane.
#[derive(Clone)]
pub struct DockerEngine {
    docker: Docker,
}

/// Result of running a container.
pub struct RunningContainer {
    pub container_id: String,
    pub host_port: u16,
}

impl DockerEngine {
    /// Connect to the local Docker daemon (unix socket / npipe / DOCKER_HOST).
    pub fn connect() -> Result<Self> {
        let docker = Docker::connect_with_local_defaults()
            .context("connecting to Docker daemon")?;
        Ok(Self { docker })
    }

    /// Verify the daemon is reachable; returns the server version string.
    pub async fn ping(&self) -> Result<String> {
        let v = self.docker.version().await.context("docker version")?;
        Ok(v.version.unwrap_or_else(|| "unknown".to_string()))
    }

    /// Build an image from a local build context directory.
    ///
    /// Returns the concatenated build log. `tag` is the resulting image tag.
    pub async fn build_image(
        &self,
        context_dir: &Path,
        dockerfile: &str,
        tag: &str,
    ) -> Result<String> {
        let tar = tar_directory(context_dir).context("creating build context tarball")?;

        // Dockerfile path inside the tar must be relative; strip a leading "./".
        let dockerfile = dockerfile.trim_start_matches("./");

        let options = BuildImageOptions {
            dockerfile: dockerfile.to_string(),
            t: tag.to_string(),
            rm: true,
            forcerm: true,
            ..Default::default()
        };

        let mut stream = self.docker.build_image(options, None, Some(tar.into()));
        let mut log = String::new();
        while let Some(item) = stream.next().await {
            let info = item.context("docker build stream error")?;
            if let Some(s) = info.stream {
                log.push_str(&s);
            }
            if let Some(err) = info.error {
                anyhow::bail!("docker build failed: {err}\n{log}");
            }
        }
        Ok(log)
    }

    /// Create and start a container for the given image, publishing the app's
    /// primary port on `host_port`.
    pub async fn run_container(
        &self,
        image: &str,
        name: &str,
        manifest: &Manifest,
        host_port: u16,
    ) -> Result<RunningContainer> {
        let container_port = manifest.primary_port();
        let port_key = format!("{container_port}/tcp");

        let mut exposed = HashMap::new();
        exposed.insert(port_key.clone(), HashMap::new());

        let mut bindings = HashMap::new();
        bindings.insert(
            port_key,
            Some(vec![PortBinding {
                host_ip: Some("127.0.0.1".to_string()),
                host_port: Some(host_port.to_string()),
            }]),
        );

        let env: Vec<String> = manifest
            .env
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();

        // Cap container logs so they can't fill the disk: 3 rotated 10MB files.
        let mut log_opts = HashMap::new();
        log_opts.insert("max-size".to_string(), "10m".to_string());
        log_opts.insert("max-file".to_string(), "3".to_string());

        let host_config = HostConfig {
            port_bindings: Some(bindings),
            memory: parse_memory(manifest.resources.memory.as_deref()),
            nano_cpus: parse_cpus(manifest.resources.cpu.as_deref()),
            restart_policy: Some(bollard::models::RestartPolicy {
                name: Some(bollard::models::RestartPolicyNameEnum::ON_FAILURE),
                maximum_retry_count: Some(5),
            }),
            log_config: Some(bollard::models::HostConfigLogConfig {
                typ: Some("json-file".to_string()),
                config: Some(log_opts),
            }),
            ..Default::default()
        };

        let config = Config {
            image: Some(image.to_string()),
            env: Some(env),
            exposed_ports: Some(exposed),
            host_config: Some(host_config),
            ..Default::default()
        };

        let created = self
            .docker
            .create_container(
                Some(CreateContainerOptions {
                    name: name.to_string(),
                    platform: None,
                }),
                config,
            )
            .await
            .context("creating container")?;

        self.docker
            .start_container(&created.id, None::<StartContainerOptions<String>>)
            .await
            .context("starting container")?;

        Ok(RunningContainer {
            container_id: created.id,
            host_port,
        })
    }

    /// Stop and remove a container by id. Missing containers are ignored.
    pub async fn stop_and_remove(&self, container_id: &str) -> Result<()> {
        let _ = self
            .docker
            .stop_container(container_id, Some(StopContainerOptions { t: 10 }))
            .await;
        self.docker
            .remove_container(
                container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
            .context("removing container")?;
        Ok(())
    }

    /// Restart a running container in place.
    pub async fn restart(&self, container_id: &str) -> Result<()> {
        self.docker
            .restart_container(container_id, None)
            .await
            .context("restarting container")?;
        Ok(())
    }

    /// Fetch the last `tail` lines of a container's logs.
    pub async fn logs(&self, container_id: &str, tail: &str) -> Result<String> {
        let mut stream = self.docker.logs(
            container_id,
            Some(LogsOptions::<String> {
                stdout: true,
                stderr: true,
                tail: tail.to_string(),
                ..Default::default()
            }),
        );
        let mut out = String::new();
        while let Some(item) = stream.next().await {
            let chunk = item.context("docker logs stream error")?;
            out.push_str(&chunk.to_string());
        }
        Ok(out)
    }

    /// Take a single CPU/memory stats snapshot for a running container.
    pub async fn stats_snapshot(&self, container_id: &str) -> Result<StatsSnapshot> {
        let mut stream = self.docker.stats(
            container_id,
            Some(StatsOptions {
                stream: false,
                one_shot: true,
            }),
        );
        let s = stream
            .next()
            .await
            .context("no stats returned")?
            .context("docker stats error")?;

        let mem = s.memory_stats.usage.unwrap_or(0);
        let mem_limit = s.memory_stats.limit.unwrap_or(0);

        // CPU percentage relative to the system, mirroring `docker stats`.
        let cpu_pct = compute_cpu_percent(&s);

        Ok(StatsSnapshot {
            cpu_percent: cpu_pct,
            memory_bytes: mem,
            memory_limit_bytes: mem_limit,
        })
    }
}

#[derive(Debug, serde::Serialize)]
pub struct StatsSnapshot {
    pub cpu_percent: f64,
    pub memory_bytes: u64,
    pub memory_limit_bytes: u64,
}

fn compute_cpu_percent(s: &bollard::container::Stats) -> f64 {
    let cpu_delta = s.cpu_stats.cpu_usage.total_usage as f64
        - s.precpu_stats.cpu_usage.total_usage as f64;
    let system_delta = s.cpu_stats.system_cpu_usage.unwrap_or(0) as f64
        - s.precpu_stats.system_cpu_usage.unwrap_or(0) as f64;
    let online = s.cpu_stats.online_cpus.unwrap_or(1).max(1) as f64;
    if system_delta > 0.0 && cpu_delta > 0.0 {
        (cpu_delta / system_delta) * online * 100.0
    } else {
        0.0
    }
}

/// Parse a memory string like "256m", "1g" into bytes.
fn parse_memory(s: Option<&str>) -> Option<i64> {
    let s = s?.trim().to_lowercase();
    if s.is_empty() {
        return None;
    }
    let (num, mult) = if let Some(n) = s.strip_suffix('g') {
        (n, 1024 * 1024 * 1024)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 1024 * 1024)
    } else if let Some(n) = s.strip_suffix('k') {
        (n, 1024)
    } else {
        (s.as_str(), 1)
    };
    num.trim().parse::<i64>().ok().map(|v| v * mult)
}

/// Parse a CPU string like "0.5" (cores) into nano-cpus.
fn parse_cpus(s: Option<&str>) -> Option<i64> {
    let s = s?.trim();
    if s.is_empty() {
        return None;
    }
    s.parse::<f64>().ok().map(|c| (c * 1e9) as i64)
}

/// Pack a directory into an uncompressed tar archive (build context).
fn tar_directory(dir: &Path) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut buf);
        builder.append_dir_all(".", dir)?;
        builder.finish()?;
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_memory_units() {
        assert_eq!(parse_memory(Some("256m")), Some(256 * 1024 * 1024));
        assert_eq!(parse_memory(Some("1g")), Some(1024 * 1024 * 1024));
        assert_eq!(parse_memory(Some("512")), Some(512));
        assert_eq!(parse_memory(None), None);
        assert_eq!(parse_memory(Some("")), None);
    }

    #[test]
    fn parses_cpus() {
        assert_eq!(parse_cpus(Some("0.5")), Some(500_000_000));
        assert_eq!(parse_cpus(Some("2")), Some(2_000_000_000));
        assert_eq!(parse_cpus(None), None);
    }
}
