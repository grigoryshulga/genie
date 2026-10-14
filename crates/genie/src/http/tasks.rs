//! Tracker routes: the TypeScript server's `/api/meta` and `/api/tasks…` with the
//! same JSON, plus the operations agents need (split, block, deps, create in draft).

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use genie_core::*;
use serde::Deserialize;
use serde_json::{Value, json};

use super::Body;
use super::ctx::{Access, Ctx};
use super::images::image_mime;
use super::{ApiError, ApiResult};
use crate::state::App;
use crate::tasks::{CommentBody, CreateBody, StatusBody, UpdateBody};

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/meta", get(meta))
        .route("/tasks", get(list).post(create))
        .route("/tasks/{id}", get(show).patch(update).delete(delete_task))
        .route("/tasks/{id}/status", post(status))
        .route("/tasks/{id}/comments", post(comment))
        .route("/tasks/{id}/acceptance/{n}", post(check))
        .route("/tasks/{id}/artifacts", post(add_artifact))
        .route("/tasks/{id}/artifacts/{n}", get(read_artifact))
        .route("/tasks/{id}/split", post(split))
        .route("/tasks/{id}/block", post(block).delete(unblock))
        .route("/tasks/{id}/docs-impact", get(docs_impact))
        .route("/tasks/{id}/usage", get(usage))
        .route("/journal", get(journal))
}

/// Tracker call in the caller's project, off the async runtime.
pub async fn tracker<T: Send + 'static>(
    app: &Arc<App>,
    access: &Access,
    f: impl FnOnce(&Tracker) -> Result<T> + Send + 'static,
) -> ApiResult<T> {
    let slug = access.project.clone();
    let out = app.blocking(move |app| app.with_tracker(&slug, f)).await?;
    Ok(out)
}

pub use crate::tasks::{Touch, changed};

/// An agent acts within its assignment (see [`crate::tasks::authorize`]).
pub async fn in_scope(app: &Arc<App>, access: &Access, id: &str, touch: Touch) -> ApiResult<()> {
    let (caller, id) = (access.caller(), id.to_string());
    Ok(app.blocking(move |app| crate::tasks::authorize(app, &caller, &id, touch)).await?)
}

fn to_json<T: serde::Serialize>(v: T) -> Json<Value> {
    Json(serde_json::to_value(v).unwrap_or(Value::Null))
}

async fn meta(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let (meta, counts) = tracker(&app, &access, |t| Ok((t.meta()?, t.counts()?))).await?;
    // Team roles of the agent configuration available here, with their default models.
    let agents = app.agents();
    let team_roles: Vec<&crate::agent_config::RoleDef> =
        agents.roles.values().filter(|r| r.class != Role::Orchestrator && r.available_in(&access.project)).collect();
    let roles: Vec<&str> = team_roles.iter().map(|r| r.id.as_str()).collect();
    let mut role_models = serde_json::Map::new();
    for r in agents.roles.values() {
        let by_id = app.cfg.role_models.get(&r.id);
        let by_class = app.cfg.role_models.get(r.class.as_str());
        let model = r.model.clone().or_else(|| by_id.and_then(|d| d.model.clone())).or_else(|| by_class.and_then(|d| d.model.clone()));
        let thinking =
            r.thinking.clone().or_else(|| by_id.and_then(|d| d.thinking.clone())).or_else(|| by_class.and_then(|d| d.thinking.clone()));
        role_models.insert(r.id.clone(), json!({ "model": model, "thinking": thinking }));
    }
    Ok(Json(json!({
        "prefix": meta.prefix,
        "project": meta.project,
        "slug": access.project,
        "created": meta.created,
        "counts": counts,
        "statuses": Status::ALL,
        "roles": roles,
        "roleModels": role_models,
        "types": TaskType::ALL,
        "user": access.actor.name,
        "access": access.role,
        "server": "rust",
    })))
}

#[derive(Debug, Default, Deserialize)]
struct ListQuery {
    status: Option<String>,
    #[serde(rename = "type")]
    task_type: Option<String>,
    epics: Option<String>,
    closed: Option<String>,
    q: Option<String>,
    parent: Option<String>,
    team: Option<String>,
    label: Option<String>,
    ready: Option<String>,
}

/// Parse a comma list, ignoring unknown values (the TypeScript server does the same).
fn parse_list<T: std::str::FromStr>(v: &Option<String>) -> Vec<T> {
    v.as_deref().unwrap_or_default().split(',').filter_map(|s| s.trim().parse().ok()).collect()
}

async fn list(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<ListQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let ready = q.ready.as_deref() == Some("1");
    let filter = ListFilter {
        status: parse_list(&q.status),
        task_type: parse_list(&q.task_type),
        exclude_epics: q.epics.as_deref() == Some("0"),
        include_closed: q.closed.as_deref() == Some("1"),
        search: q.q.filter(|s| !s.is_empty()),
        parent: q.parent.filter(|s| !s.is_empty()),
        team: q.team.filter(|s| !s.is_empty()),
        label: q.label.filter(|s| !s.is_empty()),
    };
    let rows = tracker(&app, &access, move |t| if ready { t.ready_queue() } else { t.list(&filter) }).await?;
    Ok(to_json(rows))
}

async fn show(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let (task, epic) = tracker(&app, &access, move |t| Ok((t.get(&id)?, t.epic_context(&id)?))).await?;
    let mut v = serde_json::to_value(task).unwrap_or_default();
    if let Some(e) = epic.epic {
        v["epic"] = json!({ "id": e.id, "title": e.title, "description": e.description, "artifacts": e.artifacts });
    }
    Ok(Json(v))
}

fn strings(v: &Value) -> Option<Vec<String>> {
    v.as_array().map(|a| {
        a.iter()
            .filter_map(|x| x.as_str().map(str::to_string).or_else(|| x.as_i64().map(|n| n.to_string())))
            .filter(|s| !s.is_empty())
            .collect()
    })
}

async fn create(State(app): State<Arc<App>>, ctx: Ctx, Body(b): Body<CreateBody>) -> ApiResult<impl IntoResponse> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    let caller = access.caller();
    let task = app.blocking(move |app| crate::tasks::create(app, &caller, b)).await?;
    Ok((StatusCode::CREATED, to_json(task)))
}

async fn update(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Body(b): Body<UpdateBody>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    let caller = access.caller();
    let task = app.blocking(move |app| crate::tasks::update(app, &caller, &id, b)).await?;
    Ok(to_json(task))
}

#[derive(Debug, Default, Deserialize)]
struct DeleteQuery {
    /// Delete the subtasks together with the task (otherwise a task with subtasks is refused).
    cascade: Option<String>,
}

/// Delete a task for good (see [`crate::tasks::delete`]). Project admins only.
async fn delete_task(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path(id): Path<String>,
    Query(q): Query<DeleteQuery>,
) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.admin()?;
    let cascade = q.cascade.as_deref() == Some("1");
    let caller = access.caller();
    let deleted = app.blocking(move |app| crate::tasks::delete(app, &caller, &id, cascade)).await?;
    Ok(Json(json!({ "ok": true, "deleted": deleted.ids, "report": deleted.report })))
}

async fn status(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Body(b): Body<StatusBody>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    let caller = access.caller();
    let task = app.blocking(move |app| crate::tasks::set_status(app, &caller, &id, b)).await?;
    Ok(to_json(task))
}

async fn comment(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path(id): Path<String>,
    Body(b): Body<CommentBody>,
) -> ApiResult<impl IntoResponse> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    let caller = access.caller();
    let task = app.blocking(move |app| crate::tasks::comment(app, &caller, &id, b)).await?;
    Ok((StatusCode::CREATED, to_json(task)))
}

async fn check(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path((id, n)): Path<(String, i64)>,
    body: Option<Json<Value>>,
) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    in_scope(&app, &access, &id, Touch::Edit).await?;
    let done = body.map(|Json(b)| b["done"] != json!(false)).unwrap_or(true);
    let actor = access.actor.clone();
    let task = tracker(&app, &access, move |t| t.check(&actor, &id, n, done)).await?;
    changed(&app);
    Ok(to_json(task))
}

async fn add_artifact(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Json(b): Json<Value>) -> ApiResult<impl IntoResponse> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    in_scope(&app, &access, &id, Touch::Note).await?;
    let kind = match b["kind"].as_str().filter(|k| !k.is_empty()) {
        Some(k) => Some(k.parse::<ArtifactKind>().map_err(|e| ApiError::bad(e.to_string()))?),
        None => Some(ArtifactKind::Doc),
    };
    let content = match b["contentBase64"].as_str() {
        Some(b64) => base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| ApiError::bad(format!("contentBase64: {e}")))?,
        None => b["text"].as_str().unwrap_or_default().as_bytes().to_vec(),
    };
    let input = ArtifactInput {
        kind,
        source: ArtifactSource::Content(content),
        name: b["name"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string),
        note: b["note"].as_str().filter(|s| !s.is_empty()).map(str::to_string),
    };
    let actor = access.actor.clone();
    let task = tracker(&app, &access, move |t| t.add_artifact(&actor, &id, input)).await?;
    changed(&app);
    Ok((StatusCode::CREATED, to_json(task)))
}

#[derive(Deserialize, Default)]
struct ArtifactQuery {
    download: Option<String>,
    raw: Option<String>,
    base64: Option<String>,
}

async fn read_artifact(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path((id, n)): Path<(String, i64)>,
    Query(q): Query<ArtifactQuery>,
) -> ApiResult<Response> {
    let access = ctx.access(&app, None).await?;
    let a = tracker(&app, &access, move |t| t.read_artifact(&id, n)).await?;
    let safe_name: String = a.name.chars().filter(|c| *c != '"' && !c.is_control()).collect();
    if q.download.as_deref() == Some("1") {
        let mut h = HeaderMap::new();
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
        if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{safe_name}\"")) {
            h.insert(header::CONTENT_DISPOSITION, v);
        }
        return Ok((h, Bytes::from(a.content)).into_response());
    }
    let mime = image_mime(&a.content);
    if q.raw.as_deref() == Some("1") {
        let Some(mime) = mime else {
            return Err(ApiError::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, "artifact is not a supported raster image"));
        };
        let mut h = HeaderMap::new();
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
        h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
        if let Ok(v) = HeaderValue::from_str(&format!("inline; filename=\"{safe_name}\"")) {
            h.insert(header::CONTENT_DISPOSITION, v);
        }
        return Ok((h, Bytes::from(a.content)).into_response());
    }
    let mut v = json!({ "name": a.name, "kind": a.kind, "size": a.content.len(), "text": a.text });
    if let Some(m) = mime {
        v["mime"] = json!(m);
    }
    // A binary artifact for a client that saves it (`genie task artifact-read N --out FILE`).
    if q.base64.as_deref() == Some("1") && v["text"].is_null() {
        v["contentBase64"] = json!(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &a.content));
    }
    Ok(Json(v).into_response())
}

async fn split(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Json(b): Json<Value>) -> ApiResult<impl IntoResponse> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    in_scope(&app, &access, &id, Touch::Edit).await?;
    let children: Vec<CreateInput> = b["children"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|c| match c {
                    Value::String(title) => CreateInput { title: title.clone(), ..Default::default() },
                    c => CreateInput {
                        title: c["title"].as_str().unwrap_or_default().to_string(),
                        description: c["description"].as_str().map(str::to_string),
                        acceptance: strings(&c["acceptance"]).unwrap_or_default(),
                        priority: c["priority"].as_i64(),
                        deps: strings(&c["deps"]).unwrap_or_default(),
                        labels: strings(&c["labels"]),
                        ..Default::default()
                    },
                })
                .collect()
        })
        .unwrap_or_default();
    if children.is_empty() {
        return Err(ApiError::bad("children are required"));
    }
    let actor = access.actor.clone();
    let tasks = tracker(&app, &access, move |t| t.split(&actor, &id, children)).await?;
    changed(&app);
    Ok((StatusCode::CREATED, to_json(tasks)))
}

async fn block(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Json(b): Json<Value>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    in_scope(&app, &access, &id, Touch::Edit).await?;
    let reason = b["reason"].as_str().unwrap_or_default().trim().to_string();
    if reason.is_empty() {
        return Err(ApiError::bad("reason is required"));
    }
    let actor = access.actor.clone();
    let task = tracker(&app, &access, move |t| t.block(&actor, &id, &reason)).await?;
    changed(&app);
    Ok(to_json(task))
}

async fn unblock(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    in_scope(&app, &access, &id, Touch::Edit).await?;
    let actor = access.actor.clone();
    let task = tracker(&app, &access, move |t| t.unblock(&actor, &id)).await?;
    changed(&app);
    Ok(to_json(task))
}

#[derive(Debug, Deserialize)]
struct JournalQuery {
    after: Option<i64>,
    limit: Option<usize>,
}

/// Journal page for debugging and external subscribers: `?after=<id>&limit=<n>`.
async fn journal(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<JournalQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let (after, limit) = (q.after.unwrap_or(0), q.limit.unwrap_or(100).min(1000));
    let (events, last) = tracker(&app, &access, move |t| Ok((t.events_after(after, limit)?, t.last_event_id()?))).await?;
    Ok(Json(json!({ "events": events, "last": last })))
}

/// Pages of the project's knowledge the task may have made stale (a hint for review).
async fn docs_impact(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let project = access.project.clone();
    Ok(Json(json!(app.blocking(move |app| crate::knowledge::docs_impact(app, &project, &id)).await?)))
}

/// What the agents spent on a task and the tasks under it (an epic's tasks,
/// subtasks): in all, by model, and by task.
async fn usage(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let (rows, tasks) = tracker(&app, &access, move |t| {
        t.get(&id)?;
        Ok((t.usage_of_task(&id)?, crate::spend::TaskIndex::load(t.conn())?))
    })
    .await?;
    let prices = app.blocking(|app| app.prices()).await?;
    Ok(Json(json!({
        "spend": crate::spend::Spend::of(&prices, &rows),
        "tasks": crate::spend::by_task(&prices, &rows, &tasks, usize::MAX),
    })))
}
