//! Automations, runs and playbooks; notifications, questionnaires and channel links.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use genie_core::automation;
use serde::Deserialize;
use serde_json::{Value, json};

use super::ctx::Ctx;
use super::{ApiError, ApiResult};
use crate::engine;
use crate::state::App;

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/automations", get(list).post(create))
        .route("/automations/playbooks", get(playbooks))
        .route("/automations/playbooks/{name}", post(install))
        .route("/automations/{id}", get(show).put(update).delete(remove))
        .route("/automations/{id}/enabled", post(enable))
        .route("/automations/{id}/run", post(run_now))
        .route("/runs", get(runs))
        .route("/runs/{id}", get(run))
        .route("/runs/{id}/cancel", post(cancel))
        .route("/hooks/{id}", post(hook))
        .route("/notifications", get(notifications))
        .route("/notifications/read", post(read))
        .route("/questions", get(my_questions))
        .route("/questions/{id}/answer", post(answer_mine))
        .route("/answer/{token}", get(answer_form).post(answer))
        .route("/me/channels", get(channels))
        .route("/me/channels/telegram/code", post(telegram_code))
        .route("/me/channels/{channel}", axum::routing::delete(unlink))
}

async fn list(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let project = access.project.clone();
    let out = app
        .blocking(move |app| {
            app.with_server(|db| {
                let rules = db.automations(Some(&project))?;
                let mut out = Vec::new();
                for a in rules {
                    let last = db.runs(&project, Some(a.id), 1)?.into_iter().next();
                    let mut v = json!(a);
                    v["trigger"] = json!(a.trigger_kind());
                    v["lastRun"] = json!(last);
                    out.push(v);
                }
                Ok(out)
            })
        })
        .await?;
    Ok(Json(json!(out)))
}

async fn owned(app: &Arc<App>, ctx: &Ctx, id: i64) -> ApiResult<genie_core::automation::Automation> {
    let access = ctx.access(app, None).await?;
    let a = app.blocking(move |app| app.with_server(|db| db.automation(id))).await?;
    if a.project != access.project {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "automation not found"));
    }
    Ok(a)
}

#[derive(Deserialize)]
struct SpecBody {
    spec: Value,
}

async fn create(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<SpecBody>) -> ApiResult<impl IntoResponse> {
    let access = ctx.access(&app, None).await?;
    access.admin()?;
    let (project, by) = (access.project.clone(), access.actor.name.clone());
    let a = app.blocking(move |app| app.with_server(|db| db.create_automation(&project, &b.spec, &by))).await?;
    app.wake_engine.notify_one();
    Ok((StatusCode::CREATED, Json(json!(a))))
}

async fn show(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let a = owned(&app, &ctx, id).await?;
    let project = a.project.clone();
    let runs = app.blocking(move |app| app.with_server(|db| db.runs(&project, Some(id), 50))).await?;
    Ok(Json(json!({ "automation": a, "runs": runs })))
}

async fn update(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>, Json(b): Json<SpecBody>) -> ApiResult<Json<Value>> {
    ctx.access(&app, None).await?.admin()?;
    owned(&app, &ctx, id).await?;
    let a = app.blocking(move |app| app.with_server(|db| db.update_automation(id, &b.spec))).await?;
    // A changed schedule or trigger is a new deadline for the engine.
    app.wake_engine.notify_one();
    Ok(Json(json!(a)))
}

async fn remove(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    ctx.access(&app, None).await?.admin()?;
    owned(&app, &ctx, id).await?;
    app.blocking(move |app| app.with_server(|db| db.delete_automation(id))).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct EnabledBody {
    enabled: bool,
}

async fn enable(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>, Json(b): Json<EnabledBody>) -> ApiResult<Json<Value>> {
    ctx.access(&app, None).await?.admin()?;
    owned(&app, &ctx, id).await?;
    let a = app.blocking(move |app| app.with_server(|db| db.set_automation_enabled(id, b.enabled))).await?;
    app.wake_engine.notify_one();
    Ok(Json(json!(a)))
}

async fn run_now(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>, body: Option<Json<Value>>) -> ApiResult<impl IntoResponse> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    let a = owned(&app, &ctx, id).await?;
    let inputs = body.map(|Json(b)| b).unwrap_or(json!({}));
    let task = inputs["task"].as_str().map(str::to_string);
    let by = access.actor.name.clone();
    let run = app
        .blocking(move |app| {
            let task_json = match &task {
                Some(t) => serde_json::to_value(app.with_tracker(&a.project, |tr| tr.get(t))?).unwrap_or(Value::Null),
                None => Value::Null,
            };
            let ctx = json!({ "project": a.project, "inputs": inputs, "task": task_json, "event": { "type": "manual", "actor": by, "task": task_json } });
            let key = format!("manual:{}", genie_core::server_db::new_code());
            engine::start_run(app, &a, &key, ctx, 0, None)
        })
        .await?;
    app.wake_engine.notify_one();
    Ok((StatusCode::CREATED, Json(json!(run))))
}

#[derive(Deserialize, Default)]
struct RunsQuery {
    automation: Option<i64>,
    limit: Option<i64>,
}

async fn runs(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<RunsQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let project = access.project.clone();
    let out = app.blocking(move |app| app.with_server(|db| db.runs(&project, q.automation, q.limit.unwrap_or(100).min(500)))).await?;
    Ok(Json(json!(out)))
}

async fn run(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let (run, steps) = app.blocking(move |app| app.with_server(|db| Ok((db.run(id)?, db.steps(id)?)))).await?;
    if run.project != access.project {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "run not found"));
    }
    let mut v = json!(run);
    v["steps"] = json!(steps);
    Ok(Json(v))
}

async fn cancel(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    let project = access.project.clone();
    app.blocking(move |app| {
        let run = app.with_server(|db| db.run(id))?;
        if run.project != project {
            return Err(genie_core::GenieError::not_found("run not found").into());
        }
        app.with_server(|db| {
            for s in db.steps(id)? {
                if let Some(job) = s.wait.as_ref().and_then(|w| w["job"].as_i64()) {
                    db.cancel_job(job)?;
                }
            }
            db.set_run_status(id, "cancelled", Some("cancelled by a person"))
        })
    })
    .await?;
    // A cancelled run frees a slot of its rule's concurrency.
    app.wake_engine.notify_one();
    Ok(Json(json!({ "ok": true })))
}

async fn playbooks(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    ctx.access(&app, None).await?;
    Ok(Json(json!(
        engine::playbooks().into_iter().map(|(id, title, spec)| json!({ "id": id, "title": title, "spec": spec })).collect::<Vec<_>>()
    )))
}

async fn install(State(app): State<Arc<App>>, ctx: Ctx, Path(name): Path<String>) -> ApiResult<impl IntoResponse> {
    let access = ctx.access(&app, None).await?;
    access.admin()?;
    let spec = engine::playbooks()
        .into_iter()
        .find(|(id, ..)| *id == name)
        .map(|(_, _, s)| s)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "no such playbook"))?;
    let (project, by) = (access.project.clone(), access.actor.name.clone());
    let a = app.blocking(move |app| app.with_server(|db| db.create_automation(&project, &spec, &by))).await?;
    app.wake_engine.notify_one();
    Ok((StatusCode::CREATED, Json(json!(a))))
}

#[derive(Deserialize, Default)]
struct HookQuery {
    token: Option<String>,
}

/// Incoming webhook trigger: `POST /api/hooks/<automation>?token=<secret from the spec>`.
async fn hook(
    State(app): State<Arc<App>>,
    Path(id): Path<i64>,
    Query(q): Query<HookQuery>,
    body: Option<Json<Value>>,
) -> ApiResult<impl IntoResponse> {
    let a = app
        .blocking(move |app| app.with_server(|db| db.automation(id)))
        .await
        .map_err(|_| ApiError::new(StatusCode::NOT_FOUND, "not found"))?;
    let secret = a.spec["on"]["webhook"]["secret"].as_str().unwrap_or_default();
    let given = q.token.unwrap_or_default();
    if a.trigger_kind() != "webhook" || secret.len() < 16 || !constant_eq(secret.as_bytes(), given.as_bytes()) || !a.enabled {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not found"));
    }
    let payload = body.map(|Json(b)| b).unwrap_or(Value::Null);
    if !automation::matches(&a.spec["on"]["where"], &payload) {
        return Ok((StatusCode::ACCEPTED, Json(json!({ "ok": true, "started": false }))));
    }
    let run = app
        .blocking(move |app| {
            let key = format!("webhook:{}", genie_core::server_db::new_code());
            let ctx =
                json!({ "project": a.project, "payload": payload, "event": { "type": "webhook", "actor": "webhook", "payload": payload } });
            engine::start_run(app, &a, &key, ctx, 0, None)
        })
        .await?;
    app.wake_engine.notify_one();
    Ok((StatusCode::ACCEPTED, Json(json!({ "ok": true, "started": true, "run": run.map(|r| r.id) }))))
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[derive(Deserialize, Default)]
struct NotifQuery {
    unread: Option<String>,
}

async fn notifications(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<NotifQuery>) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.clone();
    if user.id == 0 {
        return Ok(Json(json!({ "items": [], "unread": 0 })));
    }
    let unread_only = q.unread.as_deref() == Some("1");
    let (items, unread) = app
        .blocking(move |app| app.with_server(|db| Ok((db.notifications(user.id, unread_only, 100)?, db.unread_count(user.id)?))))
        .await?;
    Ok(Json(json!({ "items": items, "unread": unread })))
}

#[derive(Deserialize, Default)]
struct ReadBody {
    id: Option<i64>,
}

async fn read(State(app): State<Arc<App>>, ctx: Ctx, body: Option<Json<ReadBody>>) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.clone();
    let id = body.and_then(|Json(b)| b.id);
    app.blocking(move |app| app.with_server(|db| db.mark_read(user.id, id))).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn my_questions(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.clone();
    let list = app.blocking(move |app| app.with_server(|db| db.questionnaires_for(user.id, true))).await?;
    Ok(Json(json!(list)))
}

/// Public: the questionnaire behind an answer link (the token is the credential).
async fn answer_form(State(app): State<Arc<App>>, Path(token): Path<String>) -> ApiResult<Json<Value>> {
    let qn = app
        .blocking(move |app| app.with_server(|db| db.questionnaire_by_secret(&token)))
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "the link is invalid or was replaced by a newer reminder"))?;
    let task_title = qn.task.clone().and_then(|t| app.with_tracker(&qn.project, |tr| Ok(tr.get(&t)?.title)).ok());
    Ok(Json(json!({ "questionnaire": qn, "taskTitle": task_title })))
}

#[derive(Deserialize)]
struct AnswerBody {
    /// `{ "1": "answer", "2": "…" }`
    answers: std::collections::BTreeMap<String, String>,
}

/// Record the answers to questions still open; whether they completed the questionnaire.
fn answer_all(
    app: &App,
    qn: &genie_core::inbox::Questionnaire,
    answers: &std::collections::BTreeMap<String, String>,
    via: &str,
) -> crate::state::AppResult<bool> {
    let mut done = false;
    for (n, text) in answers.iter().filter(|(_, t)| !t.trim().is_empty()) {
        let n: i64 = n.parse().map_err(|_| genie_core::GenieError::invalid(format!("bad question number {n}")))?;
        if qn.questions.iter().any(|q| q.n == n && q.answer.is_none()) {
            done = crate::questions::answer(app, qn.id, n, text, via)?.1;
        }
    }
    Ok(done)
}

async fn answer(State(app): State<Arc<App>>, Path(token): Path<String>, Json(b): Json<AnswerBody>) -> ApiResult<Json<Value>> {
    let result = app
        .blocking(move |app| {
            let Some(qn) = app.with_server(|db| db.questionnaire_by_secret(&token))? else {
                return Ok(None);
            };
            let done = answer_all(app, &qn, &b.answers, "web")?;
            Ok(Some((app.with_server(|db| db.questionnaire(qn.id))?, done)))
        })
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "the link is invalid or was replaced by a newer reminder"))?;
    Ok(Json(json!({ "questionnaire": result.0, "complete": result.1 })))
}

/// The person asked answers with their session or token (`genie me answer`, their MCP client).
async fn answer_mine(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>, Json(b): Json<AnswerBody>) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.clone();
    let result = app
        .blocking(move |app| {
            let qn = match app.with_server(|db| db.questionnaire(id)) {
                Ok(qn) if qn.recipient == user.id => qn,
                Ok(_) | Err(crate::state::AppError::Genie(genie_core::GenieError::NotFound(_))) => return Ok(None),
                Err(e) => return Err(e),
            };
            if qn.status != "open" {
                return Err(genie_core::GenieError::invalid(format!("questionnaire {id} is {}", qn.status)).into());
            }
            let done = answer_all(app, &qn, &b.answers, "api")?;
            Ok(Some((app.with_server(|db| db.questionnaire(qn.id))?, done)))
        })
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "no such questionnaire for you"))?;
    Ok(Json(json!({ "questionnaire": result.0, "complete": result.1 })))
}

async fn channels(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.clone();
    let links = app.blocking(move |app| app.with_server(|db| db.channel_links(user.id))).await?;
    Ok(Json(json!({
        "links": links.into_iter().map(|(c, a)| json!({ "channel": c, "address": a })).collect::<Vec<_>>(),
        "telegram": app.cfg.telegram.is_some(),
        "email": app.cfg.smtp.is_some(),
    })))
}

async fn telegram_code(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.clone();
    if user.id == 0 {
        return Err(ApiError::bad("create a user first: genie user add <login> --admin"));
    }
    if app.cfg.telegram.is_none() {
        return Err(ApiError::bad("Telegram is not configured on this server (telegram.token in config.json)"));
    }
    let code = app.blocking(move |app| app.with_server(|db| db.create_link_code(user.id, "telegram"))).await?;
    let bot = crate::channels::telegram_username(&app).await;
    Ok(Json(
        json!({ "code": code, "bot": bot, "instructions": format!("Send /start {code} to the bot{} within 30 minutes.", bot.as_ref().map(|b| format!(" @{b}")).unwrap_or_default()) }),
    ))
}

async fn unlink(State(app): State<Arc<App>>, ctx: Ctx, Path(channel): Path<String>) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.clone();
    app.blocking(move |app| app.with_server(|db| db.unlink_channel(user.id, &channel))).await?;
    Ok(Json(json!({ "ok": true })))
}
