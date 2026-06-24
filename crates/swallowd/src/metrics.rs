use std::time::Duration;

use sqlx::SqlitePool;
use tracing::warn;

use crate::docker::DockerEngine;
use crate::models::{Instance, Metric};

/// How often resource usage is sampled for each running instance.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(15);

/// Spawn the background sampler that records CPU/memory time-series for every
/// running instance. Samples are pruned per-instance (see [`Metric::RETENTION`]).
pub fn spawn(db: SqlitePool, docker: DockerEngine) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SAMPLE_INTERVAL);
        loop {
            tick.tick().await;
            if let Err(e) = sample_once(&db, &docker).await {
                warn!(error = %e, "metrics sampling pass failed");
            }
        }
    });
}

async fn sample_once(db: &SqlitePool, docker: &DockerEngine) -> anyhow::Result<()> {
    let running = Instance::list_all_running(db).await?;
    for inst in running {
        let Some(cid) = inst.container_id.as_deref() else {
            continue;
        };
        match docker.stats_snapshot(cid).await {
            Ok(s) => {
                let _ = Metric::record(
                    db,
                    inst.id,
                    s.cpu_percent,
                    s.memory_bytes as i64,
                    s.memory_limit_bytes as i64,
                )
                .await;
            }
            // Container may have just stopped/restarted; skip quietly.
            Err(_) => continue,
        }
    }
    Ok(())
}
