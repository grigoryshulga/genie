//! Project-scoped Jev policy, evaluations and evidence. Agents cannot configure it.
use super::{
    ApiError, ApiResult, Body,
    ctx::{Access, Ctx},
};
use crate::{
    decisions::{self, Policy, Request},
    state::App,
    tasks,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/decisions/policy", get(policy).put(save_policy))
        .route("/decisions/evaluate", post(evaluate))
        .route("/decisions", get(list))
        .route("/decisions/{id}", get(show))
}

async fn policy(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let slug = access.project;
    let v = app
        .blocking(move |app| {
            Ok(json!({
                "policy":decisions::policy(app,&slug)?,"available":app.cfg.decisions.enabled,
                "model":app.cfg.decisions.model,"inputPrice":app.cfg.decisions.input_price,
                "endpoint":app.cfg.decisions.endpoint_url().ok().map(|u| u.to_string()),
                "keyProvider":app.cfg.decisions.key_provider,
                "serverKey":app.cfg.decisions.api_key_env.is_some()
            }))
        })
        .await?;
    Ok(Json(v))
}

async fn save_policy(State(app): State<Arc<App>>, ctx: Ctx, Body(p): Body<Policy>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.admin()?;
    app.blocking(move |app| {
        decisions::validate_policy(app, &access.project, &p)?;
        app.with_server(|db| db.set_decision_policy(&access.project, &access.actor.name, &serde_json::to_value(&p)?))?;
        Ok(())
    })
    .await?;
    Ok(Json(json!({"ok":true})))
}

fn task_access(app: &App, access: &Access, task: &str) -> crate::state::AppResult<String> {
    let task = app.with_tracker(&access.project, |t| Ok(t.get(task)?.id))?;
    tasks::authorize(app, &access.caller(), &task, tasks::Touch::Edit)?;
    Ok(task)
}

async fn evaluate(State(app): State<Arc<App>>, ctx: Ctx, Body(mut r): Body<Request>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    let p = decisions::policy(&app, &access.project)?;
    if !p.tool_enabled {
        return Err(ApiError::new(axum::http::StatusCode::FORBIDDEN, "Jev decision tool is disabled"));
    }
    if access.agent {
        let role = access.agent_role_id.clone().unwrap_or_else(|| access.actor.role.to_string());
        if !p.tool_roles.contains(&role) {
            return Err(ApiError::new(axum::http::StatusCode::FORBIDDEN, format!("role {role} may not call Jev")));
        }
    }
    let task = r.task.clone().ok_or_else(|| ApiError::bad("a decision requires a task"))?;
    let a = access.clone();
    let (task, initiator) = app
        .blocking(move |app| {
            let task = task_access(app, &a, &task)?;
            let initiator = decisions::initiator(app, &a.project, &a.actor, &task);
            Ok((task, initiator))
        })
        .await?;
    r.task = Some(task);
    // evaluate uses brief database sections around a bounded async provider call.
    let app2 = app.clone();
    let result = app
        .blocking(move |_| {
            crate::state::block_on(decisions::evaluate(&app2, &access.project, &access.actor, initiator.as_deref(), &r, None))
        })
        .await?;
    Ok(Json(json!(result)))
}

#[derive(Deserialize, Default)]
struct ListQuery {
    task: Option<String>,
}
async fn list(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<ListQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    if access.agent && q.task.is_none() {
        return Err(ApiError::bad("agents must name their task when listing decisions"));
    }
    let records = app
        .blocking(move |app| {
            let task = q.task.map(|task| task_access(app, &access, &task)).transpose()?;
            app.with_server(|db| db.decisions(&access.project, task.as_deref(), 100))
        })
        .await?;
    Ok(Json(json!(records)))
}

async fn show(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let record = app
        .blocking(move |app| {
            let record = app.with_server(|db| db.decision(&access.project, id))?;
            task_access(app, &access, &record.task)?;
            Ok(record)
        })
        .await?;
    Ok(Json(json!(record)))
}
