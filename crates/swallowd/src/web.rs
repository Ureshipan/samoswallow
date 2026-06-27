use axum::extract::{Form, Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use maud::{html, Markup, DOCTYPE};
use serde::Deserialize;

use crate::api::AppState;
use crate::error::{ApiError, ApiResult};
use crate::models::{App, Build, Instance, Metric};

/// Server-rendered web UI. Shares `AppState` with the JSON API.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(dashboard))
        .route("/apps", post(create_app))
        .route("/apps/{id}", get(app_detail))
        .route("/apps/{id}/deploy", post(deploy_app))
        .route("/apps/{id}/settings", post(update_app_settings))
        .route("/apps/{id}/delete", post(delete_app))
        .route("/instances/{id}/restart", post(restart_instance))
        .route("/instances/{id}/stop", post(stop_instance))
        .route("/builds/{id}/rollback", post(rollback_build))
        .route("/instances/{id}/logs", get(instance_logs_page))
        .with_state(state)
}

const STYLE: &str = r#"
:root { color-scheme: dark; }
* { box-sizing: border-box; }
body { font-family: ui-sans-serif, system-ui, sans-serif; margin: 0; background: #14161a; color: #e6e6e6; }
header { background: #1d2026; padding: 14px 24px; border-bottom: 1px solid #2a2e36; }
header a { color: #f5c542; text-decoration: none; font-weight: 700; font-size: 18px; }
main { max-width: 920px; margin: 0 auto; padding: 24px; }
h1, h2 { font-weight: 650; }
a { color: #6fb3ff; }
.card { background: #1d2026; border: 1px solid #2a2e36; border-radius: 10px; padding: 16px 18px; margin-bottom: 14px; }
.row { display: flex; justify-content: space-between; align-items: center; gap: 12px; }
table { width: 100%; border-collapse: collapse; }
th, td { text-align: left; padding: 8px 10px; border-bottom: 1px solid #2a2e36; font-size: 14px; }
th { color: #9aa3af; font-weight: 600; }
.tag { display: inline-block; padding: 2px 8px; border-radius: 999px; font-size: 12px; font-weight: 600; }
.tag.running, .tag.success { background: #163b22; color: #6ee787; }
.tag.stopped, .tag.failed { background: #3b1616; color: #ff8b8b; }
.tag.building, .tag.pending { background: #3b3416; color: #f5d76e; }
button, input[type=submit] { font: inherit; cursor: pointer; border: 1px solid #3a3f49; background: #262a32; color: #e6e6e6; padding: 6px 12px; border-radius: 7px; }
button.primary { background: #f5c542; color: #1a1a1a; border-color: #f5c542; font-weight: 600; }
button.danger { color: #ff8b8b; border-color: #5a2a2a; }
input[type=text] { font: inherit; background: #14161a; border: 1px solid #3a3f49; color: #e6e6e6; padding: 7px 10px; border-radius: 7px; width: 100%; }
form.inline { display: inline; }
.grid { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
label { display: block; font-size: 13px; color: #9aa3af; margin-bottom: 4px; }
.muted { color: #9aa3af; font-size: 13px; }
.mono { font-family: ui-monospace, monospace; font-size: 12px; }
pre { background: #0f1115; border: 1px solid #2a2e36; border-radius: 8px; padding: 12px; overflow: auto; max-height: 360px; font-size: 12px; }
.badge { font-size: 12px; padding: 3px 9px; border-radius: 999px; border: 1px solid #3a3f49; color: #9aa3af; }
.badge.online { background: #163b22; color: #6ee787; border-color: #245a35; }
.badge.offline { background: #3b1616; color: #ff8b8b; border-color: #5a2a2a; }
"#;

/// Polls the Caddy status endpoint and updates the header badge.
const CADDY_BADGE_JS: &str = r#"
(function () {
  const el = document.getElementById('caddy-badge');
  if (!el) return;
  async function tick() {
    try {
      const r = await fetch('/api/caddy/status');
      const d = await r.json();
      el.textContent = 'Caddy: ' + (d.online ? 'онлайн' : 'офлайн');
      el.className = 'badge ' + (d.online ? 'online' : 'offline');
    } catch (e) {
      el.textContent = 'Caddy: ?';
      el.className = 'badge';
    }
  }
  tick();
  setInterval(tick, 10000);
})();
"#;

fn layout(title: &str, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "samoswallow — " (title) }
                style { (maud::PreEscaped(STYLE)) }
            }
            body {
                header class="row" {
                    a href="/" { "🚛 samoswallow" }
                    div class="row" style="gap:12px" {
                        span id="caddy-badge" class="badge" title="Статус reverse-proxy Caddy" { "Caddy: …" }
                        form class="inline" method="post" action="/logout" {
                            button type="submit" { "Выйти" }
                        }
                    }
                }
                main { (body) }
                script { (maud::PreEscaped(CADDY_BADGE_JS)) }
            }
        }
    }
}

fn status_tag(status: &str) -> Markup {
    html! { span class={ "tag " (status) } { (status) } }
}

// --- Dashboard -------------------------------------------------------------

async fn dashboard(State(state): State<AppState>) -> ApiResult<Markup> {
    let apps = App::list(&state.db, state.owner_id).await?;
    let base = state.config.base_domain.clone();

    // Gather a small summary per app: instance counts + a direct link to a
    // running instance (the newest one), if any.
    struct Summary {
        app: App,
        running: i64,
        total: i64,
        open_port: Option<i64>,
    }
    let mut summaries = Vec::with_capacity(apps.len());
    for app in apps {
        let (running, total) = Instance::counts_for_app(&state.db, app.id).await?;
        let open_port = Instance::list_running_for_app(&state.db, app.id)
            .await
            .ok()
            .and_then(|v| v.into_iter().next())
            .and_then(|i| i.host_port);
        summaries.push(Summary { app, running, total, open_port });
    }

    Ok(layout(
        "dashboard",
        html! {
            div class="row" { h1 { "Приложения" } }

            @if summaries.is_empty() {
                div class="card muted" { "Пока нет приложений. Добавь первое ниже." }
            }
            @for s in &summaries {
                div class="card" {
                    div class="row" {
                        div {
                            a href={ "/apps/" (s.app.id) } { h2 style="margin:0" { (s.app.name) } }
                            div class="muted mono" { (s.app.domain) "." (base) " ← " (s.app.repo_url) }
                            div style="margin-top:6px" {
                                @if s.running > 0 {
                                    span class="tag running" { "● " (s.running) " запущено" }
                                } @else {
                                    span class="tag stopped" { "нет запущенных" }
                                }
                                span class="muted" { " · всего инстансов: " (s.total) }
                                @if let Some(port) = s.open_port {
                                    " "
                                    a href={ "http://127.0.0.1:" (port) "/" } target="_blank" { "открыть ↗" }
                                }
                            }
                        }
                        form class="inline" method="post" action={ "/apps/" (s.app.id) "/deploy" } {
                            button class="primary" type="submit" { "Deploy" }
                        }
                    }
                }
            }

            div class="card" {
                h2 { "Новое приложение" }
                form method="post" action="/apps" {
                    div class="grid" {
                        div { label { "Имя" } input type="text" name="name" placeholder="my-app" required; }
                        div { label { "Поддомен" } input type="text" name="domain" placeholder="my-app" required; }
                        div { label { "Git репозиторий (URL)" } input type="text" name="repo_url" placeholder="https://github.com/you/my-app" required; }
                        div { label { "Ветка" } input type="text" name="default_branch" value="main"; }
                        div { label { "Папка для данных на хосте (опц.)" } input type="text" name="data_dir" placeholder="/srv/my-app/data"; }
                        div { label { "Путь монтирования в контейнере" } input type="text" name="mount_path" placeholder="/data"; }
                    }
                    p class="muted" style="margin-top:8px; margin-bottom:0" {
                        "Папка с данными монтируется в контейнер при каждом деплое — для баз данных и файлов, "
                        "которые должны переживать пересборки и перезагрузки. Оставь пусто, чтобы не монтировать."
                    }
                    div style="margin-top:12px" { button class="primary" type="submit" { "Создать" } }
                }
            }
        },
    ))
}

#[derive(Deserialize)]
struct CreateAppForm {
    name: String,
    repo_url: String,
    #[serde(default)]
    default_branch: String,
    domain: String,
    #[serde(default)]
    data_dir: String,
    #[serde(default)]
    mount_path: String,
}

async fn create_app(
    State(state): State<AppState>,
    Form(form): Form<CreateAppForm>,
) -> ApiResult<Redirect> {
    let branch = if form.default_branch.trim().is_empty() {
        "main"
    } else {
        form.default_branch.trim()
    };
    if form.name.trim().is_empty() || form.repo_url.trim().is_empty() || form.domain.trim().is_empty()
    {
        return Err(ApiError::BadRequest("all fields are required".into()));
    }
    let (data_dir, mount_path) =
        crate::api::validate_data_mount(Some(&form.data_dir), Some(&form.mount_path))?;
    App::create(
        &state.db,
        state.owner_id,
        form.name.trim(),
        form.repo_url.trim(),
        branch,
        form.domain.trim(),
        data_dir.as_deref(),
        mount_path.as_deref(),
    )
    .await?;
    Ok(Redirect::to("/"))
}

// --- App detail ------------------------------------------------------------

async fn app_detail(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Markup> {
    let app = App::get(&state.db, id).await?;
    let builds = Build::list(&state.db, id).await?;
    let instances = Instance::list_for_app(&state.db, id).await?;
    let base = state.config.base_domain.clone();

    // Best-effort live stats + recorded history for running instances.
    let mut stats = std::collections::HashMap::new();
    let mut history = std::collections::HashMap::new();
    for inst in instances.iter().filter(|i| i.status == "running") {
        if let Some(cid) = &inst.container_id {
            if let Ok(s) = state.docker.stats_snapshot(cid).await {
                stats.insert(inst.id, s);
            }
        }
        if let Ok(m) = Metric::recent(&state.db, inst.id).await {
            history.insert(inst.id, m);
        }
    }

    // A direct, always-working link to the newest running instance.
    let open_port = instances
        .iter()
        .find(|i| i.status == "running")
        .and_then(|i| i.host_port);

    Ok(layout(
        &app.name,
        html! {
            div class="row" {
                div {
                    h1 style="margin-bottom:4px" { (app.name) }
                    div class="muted mono" { (app.repo_url) " (" (app.default_branch) ")" }
                    div style="margin-top:6px" {
                        @if let Some(port) = open_port {
                            a href={ "http://127.0.0.1:" (port) "/" } target="_blank" { "открыть ↗ http://127.0.0.1:" (port) }
                        } @else {
                            span class="muted" { "нет запущенных инстансов" }
                        }
                    }
                    div class="muted" style="margin-top:4px; font-size:12px" {
                        "Публичный адрес: " code { (app.domain) "." (base) }
                        " (работает, когда поднят Caddy и домен указывает на сервер)"
                    }
                    @if let Some(port) = app.external_port {
                        div class="muted" style="margin-top:4px; font-size:12px" {
                            "Внешний порт: " code { "0.0.0.0:" (port) }
                            " (прямой доступ снаружи, помимо Caddy)"
                        }
                    }
                    @if let Some((host, target)) = app.data_mount() {
                        div class="muted" style="margin-top:4px; font-size:12px" {
                            "Данные: " code { (host) } " → " code { (target) }
                            " (монтируется в контейнер)"
                        }
                    }
                }
                div {
                    form class="inline" method="post" action={ "/apps/" (app.id) "/deploy" } {
                        button class="primary" type="submit" { "Deploy" }
                    }
                    " "
                    form class="inline" method="post" action={ "/apps/" (app.id) "/delete" }
                        onsubmit="return confirm('Удалить приложение и его инстансы?')" {
                        button class="danger" type="submit" { "Delete" }
                    }
                }
            }

            div class="card" {
                h2 { "Webhook (автодеплой по push)" }
                p class="muted" {
                    "Добавь webhook в настройках GitHub-репозитория. При push в ветку "
                    code { (app.default_branch) } " самосвал пересоберёт и передеплоит приложение."
                }
                table {
                    tr { th { "Payload URL" } td class="mono" { "http://<хост-самосвала>/hooks/" (app.id) } }
                    tr { th { "Content type" } td class="mono" { "application/json" } }
                    tr { th { "Secret" } td class="mono" { (app.webhook_secret.clone().unwrap_or_default()) } }
                }
            }

            div class="card" {
                h2 { "Настройки" }
                form method="post" action={ "/apps/" (app.id) "/settings" } {
                    label for="external_port" { "Внешний порт" }
                    input type="text" inputmode="numeric" name="external_port"
                        id="external_port" placeholder="напр. 8081"
                        value=(app.external_port.map(|p| p.to_string()).unwrap_or_default())
                        style="max-width:160px";
                    p class="muted" style="margin-top:8px" {
                        "Если задан, инстансы публикуются напрямую на "
                        code { "0.0.0.0:<порт>" }
                        " и доступны снаружи (помимо поддомена Caddy). "
                        "При деплое старый инстанс гасится до запуска нового — короткий простой. "
                        "Пусто — случайный порт на " code { "127.0.0.1" } ", только через Caddy."
                    }

                    div class="grid" style="margin-top:14px" {
                        div {
                            label for="data_dir" { "Папка для данных на хосте" }
                            input type="text" name="data_dir" id="data_dir"
                                placeholder="/srv/my-app/data"
                                value=(app.data_dir.clone().unwrap_or_default());
                        }
                        div {
                            label for="mount_path" { "Путь монтирования в контейнере" }
                            input type="text" name="mount_path" id="mount_path"
                                placeholder="/data"
                                value=(app.mount_path.clone().unwrap_or_default());
                        }
                    }
                    p class="muted" style="margin-top:8px" {
                        "Папка с хоста монтируется в контейнер (по умолчанию в " code { "/data" } ") — "
                        "для постоянных баз данных и файлов, доступных и с хоста напрямую. "
                        "Применяется при следующем деплое. Пусто — без монтирования."
                    }

                    div style="margin-top:12px" { button class="primary" type="submit" { "Сохранить" } }
                }
            }

            div class="card" {
                h2 { "Инстансы" }
                @if instances.is_empty() {
                    div class="muted" { "Ещё не деплоился." }
                } @else {
                    table {
                        thead { tr {
                            th { "ID" } th { "Статус" } th { "Адрес" } th { "CPU" } th { "RAM" }
                            th { "" }
                        } }
                        tbody {
                            @for inst in &instances {
                                @let hist = history.get(&inst.id);
                                tr {
                                    td { "#" (inst.id) }
                                    td { (status_tag(&inst.status)) }
                                    td class="mono" {
                                        @match (inst.status.as_str(), inst.host_port) {
                                            ("running", Some(port)) => {
                                                a href={ "http://127.0.0.1:" (port) "/" } target="_blank" { "127.0.0.1:" (port) " ↗" }
                                            }
                                            (_, Some(port)) => { span class="muted" { (port) } }
                                            _ => { span class="muted" { "—" } }
                                        }
                                    }
                                    td {
                                        @match stats.get(&inst.id) {
                                            Some(s) => { div class="mono" { (format!("{:.1}%", s.cpu_percent)) } }
                                            None => { div class="muted" { "—" } }
                                        }
                                        @if let Some(h) = hist {
                                            @let vals: Vec<f64> = h.iter().map(|m| m.cpu_percent).collect();
                                            @let mx = vals.iter().cloned().fold(1.0_f64, f64::max);
                                            (sparkline(&vals, mx, "#6ee787"))
                                        }
                                    }
                                    td {
                                        @match stats.get(&inst.id) {
                                            Some(s) => { div class="mono" { (fmt_bytes(s.memory_bytes)) "/" (fmt_bytes(s.memory_limit_bytes)) } }
                                            None => { div class="muted" { "—" } }
                                        }
                                        @if let Some(h) = hist {
                                            @let vals: Vec<f64> = h.iter().map(|m| m.memory_bytes as f64).collect();
                                            @let lim = h.last().map(|m| m.memory_limit_bytes as f64).filter(|v| *v > 0.0);
                                            @let mx = lim.unwrap_or_else(|| vals.iter().cloned().fold(1.0_f64, f64::max));
                                            (sparkline(&vals, mx, "#6fb3ff"))
                                        }
                                    }
                                    td {
                                        a href={ "/instances/" (inst.id) "/logs" } { "логи" }
                                        @if inst.status == "running" {
                                            " "
                                            form class="inline" method="post" action={ "/instances/" (inst.id) "/restart" } {
                                                button type="submit" { "Restart" }
                                            }
                                            " "
                                            form class="inline" method="post" action={ "/instances/" (inst.id) "/stop" } {
                                                button class="danger" type="submit" { "Stop" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            div class="card" {
                h2 { "Сборки" }
                @if builds.is_empty() {
                    div class="muted" { "Сборок ещё нет." }
                } @else {
                    table {
                        thead { tr { th { "ID" } th { "Commit" } th { "Статус" } th { "Образ" } th { "Когда" } th { "" } } }
                        tbody {
                            @for b in &builds {
                                tr {
                                    td { "#" (b.id) }
                                    td class="mono" { (short_sha(&b.commit_sha)) }
                                    td { (status_tag(&b.status)) }
                                    td class="mono" { (b.image_tag.clone().unwrap_or_default()) }
                                    td class="muted" { (b.created_at) }
                                    td {
                                        @if b.status == "success" {
                                            form class="inline" method="post" action={ "/builds/" (b.id) "/rollback" } {
                                                button type="submit" { "Откатить сюда" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    @if let Some(last) = builds.first() {
                        @if let Some(log) = &last.logs {
                            h2 style="margin-top:18px" { "Лог последней сборки" }
                            pre { (log) }
                        }
                    }
                }
            }

            p { a href="/" { "← ко всем приложениям" } }
        },
    ))
}

async fn deploy_app(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let deployer = state.deployer();
    match deployer.deploy(id).await {
        Ok(_) => Redirect::to(&format!("/apps/{id}")).into_response(),
        Err(e) => ApiError::Internal(e).into_response(),
    }
}

#[derive(Deserialize)]
struct SettingsForm {
    #[serde(default)]
    external_port: String,
    #[serde(default)]
    data_dir: String,
    #[serde(default)]
    mount_path: String,
}

async fn update_app_settings(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Form(form): Form<SettingsForm>,
) -> ApiResult<Redirect> {
    App::get(&state.db, id).await?;
    // Empty field clears the port; otherwise it must parse as a number.
    let port = match form.external_port.trim() {
        "" => None,
        s => Some(
            s.parse::<i64>()
                .map_err(|_| ApiError::BadRequest("external port must be a number".into()))?,
        ),
    };
    let port = crate::api::validate_external_port(port)?;
    let (data_dir, mount_path) =
        crate::api::validate_data_mount(Some(&form.data_dir), Some(&form.mount_path))?;
    App::set_external_port(&state.db, id, port).await?;
    App::set_data_mount(&state.db, id, data_dir.as_deref(), mount_path.as_deref()).await?;
    Ok(Redirect::to(&format!("/apps/{id}")))
}

async fn delete_app(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Redirect> {
    if let Ok(instances) = Instance::list_running_for_app(&state.db, id).await {
        for inst in instances {
            if let Some(cid) = &inst.container_id {
                let _ = state.docker.stop_and_remove(cid).await;
            }
        }
    }
    let _ = state.caddy.remove_app_route(id).await;
    App::delete(&state.db, id).await?;
    Ok(Redirect::to("/"))
}

async fn instance_logs_page(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Markup> {
    let inst = Instance::get(&state.db, id).await?;
    let logs = match &inst.container_id {
        Some(cid) => state
            .docker
            .logs(cid, "500")
            .await
            .unwrap_or_else(|e| format!("(не удалось получить логи: {e})")),
        None => "(у инстанса нет контейнера)".to_string(),
    };
    let body = if logs.trim().is_empty() {
        "(пусто — приложение ничего не вывело в stdout/stderr)".to_string()
    } else {
        logs
    };

    Ok(layout(
        &format!("логи инстанса #{id}"),
        html! {
            div class="row" {
                h1 { "Логи инстанса #" (id) }
                a href={ "/apps/" (inst.app_id) } { "← к приложению" }
            }
            p class="muted" { "Последние 500 строк. Размер логов контейнера ограничен (10 МБ × 3)." }
            pre { (body) }
        },
    ))
}

async fn rollback_build(State(state): State<AppState>, Path(id): Path<i64>) -> Response {
    let app_id = match Build::get(&state.db, id).await {
        Ok(b) => b.app_id,
        Err(e) => return ApiError::from(e).into_response(),
    };
    match state.deployer().rollback(id).await {
        Ok(_) => Redirect::to(&format!("/apps/{app_id}")).into_response(),
        Err(e) => ApiError::Internal(e).into_response(),
    }
}

async fn restart_instance(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Redirect> {
    let inst = Instance::get(&state.db, id).await?;
    let app_id = inst.app_id;
    if let Some(cid) = &inst.container_id {
        state.docker.restart(cid).await.map_err(ApiError::Internal)?;
        Instance::set_status(&state.db, id, "running").await?;
    }
    Ok(Redirect::to(&format!("/apps/{app_id}")))
}

async fn stop_instance(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Redirect> {
    let inst = Instance::get(&state.db, id).await?;
    let app_id = inst.app_id;
    if let Some(cid) = &inst.container_id {
        state
            .docker
            .stop_and_remove(cid)
            .await
            .map_err(ApiError::Internal)?;
    }
    Instance::set_status(&state.db, id, "stopped").await?;
    Ok(Redirect::to(&format!("/apps/{app_id}")))
}

/// Render a tiny inline SVG sparkline from a series of values, scaled to `max`.
fn sparkline(values: &[f64], max: f64, color: &str) -> Markup {
    const W: f64 = 110.0;
    const H: f64 = 26.0;
    if values.len() < 2 {
        return html! { span class="muted" { "—" } };
    }
    let max = max.max(1e-9);
    let n = values.len() as f64;
    let pts: String = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = (i as f64) / (n - 1.0) * W;
            let y = H - (v / max).clamp(0.0, 1.0) * H;
            format!("{x:.1},{y:.1}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    let svg = format!(
        "<svg width=\"{W}\" height=\"{H}\" viewBox=\"0 0 {W} {H}\" preserveAspectRatio=\"none\" \
         style=\"vertical-align:middle\"><polyline fill=\"none\" stroke=\"{color}\" \
         stroke-width=\"1.5\" points=\"{pts}\"/></svg>"
    );
    html! { (maud::PreEscaped(svg)) }
}

fn fmt_bytes(b: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.0}{}", U[i])
}

fn short_sha(sha: &str) -> &str {
    &sha[..sha.len().min(8)]
}
