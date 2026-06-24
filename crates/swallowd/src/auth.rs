use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::{Form, State};
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use maud::{html, Markup, DOCTYPE};
use rand::Rng;
use serde::Deserialize;
use sqlx::SqlitePool;
use tracing::{info, warn};

use crate::api::AppState;

/// Routes that handle authentication itself (not behind the auth gate).
pub fn router(state: AppState) -> axum::Router {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/login", get(login_page).post(login_submit))
        .route("/logout", post(logout))
        .with_state(state)
}

const COOKIE: &str = "swallow_session";
const SESSION_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

/// In-memory session store: token -> expiry. Sessions are lost on restart,
/// which for a single-user tool just means logging in again.
#[derive(Clone, Default)]
pub struct SessionStore {
    inner: Arc<RwLock<HashMap<String, Instant>>>,
}

impl SessionStore {
    pub fn create(&self) -> String {
        let token: String = {
            let mut rng = rand::thread_rng();
            (0..32)
                .map(|_| rng.sample(rand::distributions::Alphanumeric) as char)
                .collect()
        };
        self.inner
            .write()
            .unwrap()
            .insert(token.clone(), Instant::now() + SESSION_TTL);
        token
    }

    pub fn is_valid(&self, token: &str) -> bool {
        let mut guard = self.inner.write().unwrap();
        match guard.get(token) {
            Some(exp) if *exp > Instant::now() => true,
            Some(_) => {
                guard.remove(token);
                false
            }
            None => false,
        }
    }

    pub fn remove(&self, token: &str) {
        self.inner.write().unwrap().remove(token);
    }
}

// --- password hashing ------------------------------------------------------

fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hashing password: {e}"))?
        .to_string();
    Ok(hash)
}

fn verify_password(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// Ensure the admin account has a usable password.
///
/// - If `SWALLOW_ADMIN_PASSWORD` is set, it (re)sets the password on boot.
/// - Otherwise, if the password is still unset, a random one is generated and
///   printed to the log once so the operator can sign in.
pub async fn ensure_admin_password(db: &SqlitePool, user_id: i64) -> Result<()> {
    let current = crate::models::user_password_hash(db, user_id)
        .await
        .context("reading admin password hash")?;

    if let Ok(pw) = std::env::var("SWALLOW_ADMIN_PASSWORD") {
        if !pw.is_empty() {
            let hash = hash_password(&pw)?;
            crate::models::set_user_password(db, user_id, &hash).await?;
            info!("admin password set from SWALLOW_ADMIN_PASSWORD");
            return Ok(());
        }
    }

    if current == "!unset" {
        let generated: String = {
            let mut rng = rand::thread_rng();
            (0..16)
                .map(|_| rng.sample(rand::distributions::Alphanumeric) as char)
                .collect()
        };
        let hash = hash_password(&generated)?;
        crate::models::set_user_password(db, user_id, &hash).await?;
        warn!(
            "no admin password set — generated one (login user: admin):\n\n    {generated}\n\n\
             set SWALLOW_ADMIN_PASSWORD to choose your own."
        );
    }
    Ok(())
}

// --- middleware ------------------------------------------------------------

/// Gate every route behind a valid session, except the login page and health
/// probe. Unauthenticated API calls get 401; browser routes redirect to /login.
pub async fn require_auth(
    State(state): State<AppState>,
    jar: CookieJar,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let path = req.uri().path();
    // Public routes: health probe, login, and webhook deliveries (the latter
    // authenticate via per-app HMAC signatures, not the session cookie).
    if path == "/healthz" || path == "/login" || path.starts_with("/hooks/") {
        return next.run(req).await;
    }

    let authed = jar
        .get(COOKIE)
        .map(|c| state.sessions.is_valid(c.value()))
        .unwrap_or(false);

    if authed {
        next.run(req).await
    } else if path.starts_with("/api/") {
        (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
    } else {
        Redirect::to("/login").into_response()
    }
}

// --- handlers --------------------------------------------------------------

pub async fn login_page() -> Markup {
    login_markup(None)
}

#[derive(Deserialize)]
pub struct LoginForm {
    password: String,
}

pub async fn login_submit(
    State(state): State<AppState>,
    jar: CookieJar,
    Form(form): Form<LoginForm>,
) -> Response {
    let hash = match crate::models::user_password_hash(&state.db, state.owner_id).await {
        Ok(h) => h,
        Err(_) => return login_markup(Some("internal error")).into_response(),
    };

    if !verify_password(&form.password, &hash) {
        return (StatusCode::UNAUTHORIZED, login_markup(Some("Неверный пароль"))).into_response();
    }

    let token = state.sessions.create();
    let cookie = Cookie::build((COOKIE, token))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .build();
    (jar.add(cookie), Redirect::to("/")).into_response()
}

pub async fn logout(State(state): State<AppState>, jar: CookieJar) -> Response {
    if let Some(c) = jar.get(COOKIE) {
        state.sessions.remove(c.value());
    }
    (jar.remove(Cookie::from(COOKIE)), Redirect::to("/login")).into_response()
}

fn login_markup(error: Option<&str>) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "samoswallow — вход" }
                style { (maud::PreEscaped(LOGIN_STYLE)) }
            }
            body {
                div class="box" {
                    h1 { "🚛 samoswallow" }
                    @if let Some(e) = error {
                        div class="err" { (e) }
                    }
                    form method="post" action="/login" {
                        label { "Пароль" }
                        input type="password" name="password" autofocus required;
                        button class="primary" type="submit" { "Войти" }
                    }
                    p class="muted" { "Пользователь: admin" }
                }
            }
        }
    }
}

const LOGIN_STYLE: &str = r#"
:root { color-scheme: dark; }
body { font-family: ui-sans-serif, system-ui, sans-serif; background: #14161a; color: #e6e6e6;
       display: flex; min-height: 100vh; align-items: center; justify-content: center; margin: 0; }
.box { background: #1d2026; border: 1px solid #2a2e36; border-radius: 12px; padding: 28px 32px; width: 320px; }
h1 { font-size: 20px; margin: 0 0 18px; }
label { display: block; font-size: 13px; color: #9aa3af; margin-bottom: 6px; }
input { width: 100%; box-sizing: border-box; background: #14161a; border: 1px solid #3a3f49;
        color: #e6e6e6; padding: 9px 11px; border-radius: 8px; margin-bottom: 16px; font: inherit; }
button.primary { width: 100%; background: #f5c542; color: #1a1a1a; border: none; font-weight: 600;
        padding: 10px; border-radius: 8px; cursor: pointer; font: inherit; }
.err { background: #3b1616; color: #ff8b8b; padding: 8px 11px; border-radius: 8px; font-size: 14px; margin-bottom: 14px; }
.muted { color: #9aa3af; font-size: 12px; text-align: center; margin: 16px 0 0; }
"#;
