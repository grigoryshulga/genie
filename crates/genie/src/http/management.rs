//! HTTP entrance for project memory, task managers, clarification and retrospectives.

use super::{ApiError, ApiResult, ctx::Ctx};
use crate::{management, state::App};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use genie_core::Role;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/management/tasks/{id}", get(show))
        .route("/management/tasks/{id}/sources", get(sources))
        .route("/management/tasks/{id}/source/{source}", get(source))
        .route("/management/tasks/{id}/{action}", axum::routing::post(act))
        .route("/management/memory", get(memory).post(remember))
}

async fn show(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let a = ctx.access(&app, None).await?;
    let out = app
        .blocking(move |app| {
            management::authorize_task(app, &a.project, &a.actor, &id)?;
            management::show(app, &a.project, &id)
        })
        .await?;
    Ok(Json(out))
}

async fn sources(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let a = ctx.access(&app, None).await?;
    let out = app
        .blocking(move |app| {
            management::authorize_task(app, &a.project, &a.actor, &id)?;
            management::sources(app, &a.project, &id)
        })
        .await?;
    Ok(Json(out))
}

#[derive(Deserialize, Default)]
struct Offset {
    #[serde(default)]
    offset: usize,
}
async fn source(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path((id, source)): Path<(String, String)>,
    Query(q): Query<Offset>,
) -> ApiResult<Json<Value>> {
    let a = ctx.access(&app, None).await?;
    let out = app
        .blocking(move |app| {
            management::authorize_task(app, &a.project, &a.actor, &id)?;
            management::source(app, &a.project, &id, &source, q.offset)
        })
        .await?;
    Ok(Json(out))
}

#[derive(Deserialize, Default, ts_rs::TS)]
#[ts(export, optional_fields)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagerAction {
    question: Option<String>,
    text: Option<String>,
    source: Option<String>,
    id: Option<i64>,
    #[serde(default)]
    nonblocking: bool,
    #[serde(default)]
    incomplete: bool,
    #[serde(default)]
    accept_missing: bool,
    #[serde(default)]
    reviewed: Vec<String>,
    /// Dispatch: the chat message in which a person asked for it (the project manager in a manual project needs one).
    on_behalf: Option<i64>,
}

async fn act(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path((task, action)): Path<(String, String)>,
    Json(b): Json<ManagerAction>,
) -> ApiResult<Json<Value>> {
    let a = ctx.access(&app, None).await?;
    a.write()?;
    if !matches!(a.actor.role, Role::Human | Role::Orchestrator) {
        return Err(ApiError::bad("only managers and people manage task lifecycles"));
    }
    let out = app
        .blocking(move |app| {
            management::authorize_task(app, &a.project, &a.actor, &task)?;
            let required = |s: Option<String>, key: &str| {
                s.filter(|s| !s.trim().is_empty()).ok_or_else(|| crate::state::AppError::Bad(format!("{key} is required")))
            };
            let v = match action.as_str() {
                "dispatch" => json!(match b.on_behalf {
                    Some(chat) => management::dispatch_on_behalf(app, &a.project, &a.actor, &task, chat)?,
                    None => management::dispatch(app, &a.project, &a.actor, &task)?,
                }),
                "clarify" => json!(app.with_tracker(&a.project, |t| t.management().ask(
                    &a.actor,
                    &task,
                    &b.question.unwrap_or_default(),
                    !b.nonblocking
                ))?),
                "answer" => {
                    management::project_manager(&a.actor)?;
                    let id = b.id.ok_or_else(|| crate::state::AppError::Bad("id is required".into()))?;
                    let text = required(b.text, "text")?;
                    let source = required(
                        b.source.or_else(|| (a.actor.role == Role::Human).then(|| format!("owner:{} clarification:{id}", a.actor.name))),
                        "source",
                    )?;
                    app.with_tracker(&a.project, |t| t.management().answer(&a.actor, &task, id, &text, &source))?;
                    management::show(app, &a.project, &task)?
                }
                "escalate" => management::escalate(
                    app,
                    &a.project,
                    &a.actor,
                    &task,
                    b.id.ok_or_else(|| crate::state::AppError::Bad("id is required".into()))?,
                )?,
                "retrospective" => json!(management::retrospective(
                    app,
                    &a.project,
                    &a.actor,
                    &task,
                    &required(b.text, "text")?,
                    if b.accept_missing {
                        management::Coverage::AcceptedMissing
                    } else if b.incomplete {
                        management::Coverage::Incomplete(&b.reviewed)
                    } else {
                        management::Coverage::Reviewed(&b.reviewed)
                    }
                )?),
                "retry" => {
                    let name = app.with_tracker(&a.project, |t| {
                        let m = t.management().manager(&task)?.ok_or_else(|| genie_core::GenieError::not_found("manager"))?;
                        let name = genie_core::management::manager_name(&m.task);
                        t.management().notify(
                            &name,
                            &a.actor.name,
                            "Resume your task lifecycle and its pending retrospective from durable state.",
                            Some(&m.task),
                        )?;
                        Ok(name)
                    })?;
                    app.attempts.clear(&crate::runtime::AgentKey::mailbox(&a.project, &name));
                    json!({ "ok": true })
                }
                _ => return Err(crate::state::AppError::Bad(format!("unknown management action {action}"))),
            };
            crate::tasks::changed(app);
            Ok(v)
        })
        .await?;
    Ok(Json(out))
}

#[derive(Deserialize, Default)]
struct Search {
    #[serde(default)]
    query: String,
}
async fn memory(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<Search>) -> ApiResult<Json<Value>> {
    let a = ctx.access(&app, None).await?;
    Ok(Json(json!(app.blocking(move |app| app.with_tracker(&a.project, |t| t.management().memory(&q.query))).await?)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Memory {
    text: String,
    source: String,
    replaces: Option<i64>,
}
async fn remember(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<Memory>) -> ApiResult<Json<Value>> {
    let a = ctx.access(&app, None).await?;
    a.write()?;
    let intake = a.agent_role_id.as_deref() == Some("planner")
        && a.agent_team.as_ref().is_some_and(|team| {
            app.with_tracker(&a.project, |t| Ok(t.bus().get(team)?.template.as_deref() == Some("idea"))).unwrap_or(false)
        });
    if !intake {
        management::project_manager(&a.actor)?;
    }
    let id = app
        .blocking(move |app| app.with_tracker(&a.project, |t| t.management().remember(&a.actor, &b.text, &b.source, b.replaces)))
        .await?;
    Ok(Json(json!({ "id": id })))
}
