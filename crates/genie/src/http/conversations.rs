//! Authenticated native work receipts and decisions alongside the OpenAI wire adapters.

use super::{ApiResult, Body, ctx::Ctx};
use crate::{conversations, state::App, tasks::CreateBody};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use genie_core::conversations::ProposalAction;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/conversations/{id}", get(show))
        .route("/conversations/{id}/reply", post(reply))
        .route("/conversations/{id}/tasks", post(create_task))
        .route("/conversations/{id}/proposals", post(propose))
        .route("/conversations/{id}/dispatch", post(dispatch))
        .route("/conversations/{id}/answer", post(answer))
        .route("/conversation-proposals/{id}/decide", post(decide))
}

async fn show(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let a = ctx.access(&app, None).await?;
    Ok(Json(app.blocking(move |app| conversations::show(app, &a, &id)).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextBody {
    text: String,
}

async fn reply(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Body(b): Body<TextBody>) -> ApiResult<Json<Value>> {
    let a = ctx.access(&app, None).await?;
    Ok(Json(app.blocking(move |app| Ok(json!(conversations::reply(app, &a, &id, &b.text)?))).await?))
}

async fn create_task(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Body(b): Body<CreateBody>) -> ApiResult<Json<Value>> {
    let a = ctx.access(&app, None).await?;
    Ok(Json(app.blocking(move |app| Ok(json!(conversations::create_task(app, &a, &id, b)?))).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposalBody {
    task: String,
    action: ProposalAction,
}

async fn propose(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Body(b): Body<ProposalBody>) -> ApiResult<Json<Value>> {
    let a = ctx.access(&app, None).await?;
    Ok(Json(app.blocking(move |app| Ok(json!(conversations::propose(app, &a, &id, &b.task, b.action)?))).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskBody {
    task: String,
}

async fn dispatch(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Body(b): Body<TaskBody>) -> ApiResult<Json<Value>> {
    let a = ctx.access(&app, None).await?;
    Ok(Json(app.blocking(move |app| conversations::dispatch(app, &a, &id, &b.task)).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerBody {
    task: String,
    question: i64,
    text: String,
}

async fn answer(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Body(b): Body<AnswerBody>) -> ApiResult<Json<Value>> {
    let a = ctx.access(&app, None).await?;
    Ok(Json(app.blocking(move |app| conversations::answer(app, &a, &id, &b.task, b.question, &b.text)).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionBody {
    approved: bool,
}

async fn decide(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Body(b): Body<DecisionBody>) -> ApiResult<Json<Value>> {
    let a = ctx.access(&app, None).await?;
    Ok(Json(app.blocking(move |app| Ok(json!(conversations::decide(app, &a, &id, b.approved)?))).await?))
}
