//! Project-scoped task intelligence. Statistics remains server-admin-only; task
//! evidence follows the same membership and task scope as the existing task API.
use super::{ApiResult, Body, ctx::Ctx};
use crate::intelligence::{AssessBody, ReassessBody, ResolveBody, ShadowBody};
use crate::{state::App, tasks};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/tasks/{id}/intelligence", get(history))
        .route("/tasks/{id}/intelligence/assess", post(assess))
        .route("/tasks/{id}/intelligence/reassess", post(reassess))
        .route("/tasks/{id}/intelligence/shadow", post(shadow))
        .route("/tasks/{id}/intelligence/resolve", post(resolve))
}
async fn history(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let caller = access.caller();
    let history = app.blocking(move |app| tasks::intelligence_history(app, &caller, &id)).await?;
    Ok(Json(json!(history)))
}
async fn assess(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Body(body): Body<AssessBody>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    let caller = access.caller();
    Ok(Json(app.blocking(move |app| Ok(json!(tasks::assess_intelligence(app, &caller, &id, body)?))).await?))
}
async fn reassess(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Body(body): Body<ReassessBody>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    let caller = access.caller();
    Ok(Json(app.blocking(move |app| tasks::reassess_intelligence(app, &caller, &id, body)).await?))
}
async fn shadow(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Body(body): Body<ShadowBody>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.admin()?;
    let caller = access.caller();
    Ok(Json(app.blocking(move |app| tasks::shadow_intelligence(app, &caller, &id, body)).await?))
}

async fn resolve(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Body(body): Body<ResolveBody>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    let caller = access.caller();
    Ok(Json(app.blocking(move |app| Ok(json!(tasks::resolve_intelligence(app, &caller, &id, body)?))).await?))
}
