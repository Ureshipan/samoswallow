use anyhow::{Context, Result};
use serde_json::json;

/// Client for Caddy's Admin API. Routes are managed per-app via Caddy `@id`s,
/// so they can be replaced idempotently on every deploy.
#[derive(Clone)]
pub struct CaddyClient {
    admin_url: String,
    http: reqwest::Client,
}

impl CaddyClient {
    pub fn new(admin_url: impl Into<String>) -> Self {
        // Short timeout so a down/unreachable Caddy fails fast instead of
        // stalling deploys or the status check.
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap_or_default();
        Self {
            admin_url: admin_url.into(),
            http,
        }
    }

    /// Whether Caddy's Admin API is reachable (used for the UI status badge).
    pub async fn is_online(&self) -> bool {
        self.http
            .get(format!("{}/config/", self.admin_url))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    fn route_id(app_id: i64) -> String {
        format!("swallow-route-{app_id}")
    }

    /// Ensure the base HTTP server exists (idempotent). Creates an empty
    /// `srv0` listening on :80 and :443 with an `@id`'d routes array.
    async fn ensure_bootstrap(&self) -> Result<()> {
        let url = format!("{}/config/apps/http/servers/srv0", self.admin_url);
        let existing = self.http.get(&url).send().await?;
        if existing.status().is_success() {
            let body = existing.text().await.unwrap_or_default();
            if body.trim() != "null" && !body.trim().is_empty() {
                return Ok(());
            }
        }

        // When Caddy has no config yet its `/config` tree is `null`, so we can't
        // POST/PUT into a sub-path that doesn't exist. The canonical way to set a
        // whole config from scratch is `POST /load`. samoswallow owns this Caddy
        // instance, so replacing the root is fine.
        let config = json!({
            "apps": {
                "http": {
                    "servers": {
                        "srv0": {
                            "listen": [":80", ":443"],
                            "routes": []
                        }
                    }
                }
            }
        });
        let resp = self
            .http
            .post(format!("{}/load", self.admin_url))
            .json(&config)
            .send()
            .await
            .context("bootstrapping caddy config")?;
        anyhow::ensure!(
            resp.status().is_success(),
            "caddy bootstrap failed: {}",
            resp.status()
        );
        Ok(())
    }

    /// Create or replace the route for an app: `host` -> reverse_proxy to the
    /// given upstream `host:port` dials.
    pub async fn sync_app_route(&self, app_id: i64, host: &str, upstreams: &[String]) -> Result<()> {
        self.ensure_bootstrap().await?;

        let id = Self::route_id(app_id);

        // Remove any existing route with this id (ignore 404).
        let _ = self
            .http
            .delete(format!("{}/id/{}", self.admin_url, id))
            .send()
            .await;

        let route = json!({
            "@id": id,
            "match": [{ "host": [host] }],
            "handle": [{
                "handler": "reverse_proxy",
                "upstreams": upstreams.iter().map(|u| json!({ "dial": u })).collect::<Vec<_>>()
            }]
        });

        let resp = self
            .http
            .post(format!(
                "{}/config/apps/http/servers/srv0/routes",
                self.admin_url
            ))
            .json(&route)
            .send()
            .await
            .context("posting caddy route")?;
        anyhow::ensure!(
            resp.status().is_success(),
            "caddy route upsert failed: {}",
            resp.status()
        );
        Ok(())
    }

    /// Remove an app's route (used on app delete). Best-effort.
    pub async fn remove_app_route(&self, app_id: i64) -> Result<()> {
        let _ = self
            .http
            .delete(format!("{}/id/{}", self.admin_url, Self::route_id(app_id)))
            .send()
            .await;
        Ok(())
    }
}
