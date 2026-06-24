use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::Router;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tracing::{info, warn};

use crate::api::AppState;
use crate::models::App;

type HmacSha256 = Hmac<Sha256>;

/// Webhook routes. These are intentionally outside the auth gate (external
/// services call them) and are protected by per-app HMAC signatures instead.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/hooks/{id}", post(handle))
        .with_state(state)
}

/// Handle an incoming push webhook (GitHub-compatible).
///
/// - Verifies the `X-Hub-Signature-256` HMAC against the app's secret.
/// - Responds to GitHub `ping` events.
/// - On a push to the app's default branch, triggers a deploy in the background
///   and returns 202 immediately (builds outlast GitHub's delivery timeout).
async fn handle(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, &'static str) {
    let app = match App::get(&state.db, id).await {
        Ok(a) => a,
        Err(_) => return (StatusCode::NOT_FOUND, "unknown app"),
    };

    // Verify signature if the app has a secret (it always does after creation).
    if let Some(secret) = app.webhook_secret.as_deref() {
        if !signature_valid(secret, &headers, &body) {
            warn!(app = %app.name, "webhook rejected: bad signature");
            return (StatusCode::UNAUTHORIZED, "bad signature");
        }
    }

    let event = headers
        .get("X-GitHub-Event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if event == "ping" {
        return (StatusCode::OK, "pong");
    }

    // For GitHub push events, only deploy when the pushed ref matches the
    // app's tracked branch. Other providers (no event header) just deploy.
    if event == "push" {
        if let Some(branch) = push_branch(&body) {
            if branch != app.default_branch {
                info!(app = %app.name, %branch, "webhook ignored: other branch");
                return (StatusCode::OK, "ignored: other branch");
            }
        }
    }

    info!(app = %app.name, "webhook accepted — deploying in background");
    let deployer = state.deployer();
    tokio::spawn(async move {
        if let Err(e) = deployer.deploy(id).await {
            warn!(app_id = id, error = %e, "background deploy failed");
        }
    });

    (StatusCode::ACCEPTED, "accepted")
}

/// Constant-time verification of the GitHub `X-Hub-Signature-256` header.
fn signature_valid(secret: &str, headers: &HeaderMap, body: &[u8]) -> bool {
    let header = match headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok())
    {
        Some(h) => h,
        None => return false,
    };
    let hex_sig = match header.strip_prefix("sha256=") {
        Some(s) => s,
        None => return false,
    };
    let provided = match hex::decode(hex_sig) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(body);
    mac.verify_slice(&provided).is_ok()
}

/// Extract the branch name from a GitHub push payload's `ref` field.
fn push_branch(body: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let r = v.get("ref")?.as_str()?;
    r.strip_prefix("refs/heads/").map(|s| s.to_string())
}
