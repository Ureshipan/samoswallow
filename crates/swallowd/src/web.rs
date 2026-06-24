use axum::extract::{Form, Path, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use maud::{html, Markup, DOCTYPE};
use serde::Deserialize;

use crate::api::AppState;
use crate::error::{ApiError, ApiResult};
use crate::models::{App, Build, Instance};

/// Server-rendered web UI. Shares `AppState` with the JSON API.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(dashboard))
        .route("/apps", post(create_app))
        .route("/apps/{id}", get(app_detail))
        .route("/apps/{id}/deploy", post(deploy_app))
        .route("/apps/{id}/delete", post(delete_app))
        .route("/instances/{id}/restart", post(restart_instance))
        .route("/instances/{id}/stop", post(stop_instance))
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
                    form class="inline" method="post" action="/logout" {
                        button type="submit" { "Выйти" }
                    }
                }
                main { (body) }
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

    Ok(layout(
        "dashboard",
        html! {
            div class="row" { h1 { "Приложения" } }

            @if apps.is_empty() {
                div class="card muted" { "Пока нет приложений. Добавь первое ниже." }
            }
            @for app in &apps {
                div class="card" {
                    div class="row" {
                        div {
                            a href={ "/apps/" (app.id) } { h2 style="margin:0" { (app.name) } }
                            div class="muted mono" { (app.domain) "." (base) " ← " (app.repo_url) }
                        }
                        form class="inline" method="post" action={ "/apps/" (app.id) "/deploy" } {
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
                        div { label { "Ветка" } input type="text" name="default_branch" value="master"; }
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
}

async fn create_app(
    State(state): State<AppState>,
    Form(form): Form<CreateAppForm>,
) -> ApiResult<Redirect> {
    let branch = if form.default_branch.trim().is_empty() {
        "master"
    } else {
        form.default_branch.trim()
    };
    if form.name.trim().is_empty() || form.repo_url.trim().is_empty() || form.domain.trim().is_empty()
    {
        return Err(ApiError::BadRequest("all fields are required".into()));
    }
    App::create(
        &state.db,
        state.owner_id,
        form.name.trim(),
        form.repo_url.trim(),
        branch,
        form.domain.trim(),
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

    // Best-effort live stats for running instances.
    let mut stats = std::collections::HashMap::new();
    for inst in instances.iter().filter(|i| i.status == "running") {
        if let Some(cid) = &inst.container_id {
            if let Ok(s) = state.docker.stats_snapshot(cid).await {
                stats.insert(inst.id, s);
            }
        }
    }

    Ok(layout(
        &app.name,
        html! {
            div class="row" {
                div {
                    h1 style="margin-bottom:4px" { (app.name) }
                    div class="muted mono" {
                        a href={ "http://" (app.domain) "." (base) } { (app.domain) "." (base) }
                        " ← " (app.repo_url) " (" (app.default_branch) ")"
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
                h2 { "Инстансы" }
                @if instances.is_empty() {
                    div class="muted" { "Ещё не деплоился." }
                } @else {
                    table {
                        thead { tr {
                            th { "ID" } th { "Статус" } th { "Порт" } th { "CPU" } th { "RAM" }
                            th { "Создан" } th { "" }
                        } }
                        tbody {
                            @for inst in &instances {
                                tr {
                                    td { "#" (inst.id) }
                                    td { (status_tag(&inst.status)) }
                                    td class="mono" { (inst.host_port.map(|p| p.to_string()).unwrap_or_default()) }
                                    @match stats.get(&inst.id) {
                                        Some(s) => {
                                            td class="mono" { (format!("{:.1}%", s.cpu_percent)) }
                                            td class="mono" { (fmt_bytes(s.memory_bytes)) "/" (fmt_bytes(s.memory_limit_bytes)) }
                                        }
                                        None => { td class="muted" { "—" } td class="muted" { "—" } }
                                    }
                                    td class="muted" { (inst.created_at) }
                                    td {
                                        @if inst.status == "running" {
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
                        thead { tr { th { "ID" } th { "Commit" } th { "Статус" } th { "Образ" } th { "Когда" } } }
                        tbody {
                            @for b in &builds {
                                tr {
                                    td { "#" (b.id) }
                                    td class="mono" { (short_sha(&b.commit_sha)) }
                                    td { (status_tag(&b.status)) }
                                    td class="mono" { (b.image_tag.clone().unwrap_or_default()) }
                                    td class="muted" { (b.created_at) }
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
