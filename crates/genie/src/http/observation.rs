//! HTTP entrance of the observation plane (`docs/platform/observation-plane.md` §14, §15).
//!
//! Each route checks the caller's rights in the project here (read, write or admin), then calls
//! the application service, which checks the actor's own rights (§14) and runs the domain. Work
//! runs in `App::blocking`; the service holds no database lock while a source is measured.

use std::str::FromStr;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::Utc;
use genie_core::GenieError;
use genie_core::observation::store::{CaseFilter, EvidenceFilter};
use genie_core::observation::{Evidence, Incident, IncidentStatus, ObservationCase, parse_case_id, parse_incident_id};
use serde::Deserialize;
use serde_json::{Value, json};

use super::Body;
use super::ctx::Ctx;
use super::{ApiError, ApiResult};
use crate::observation::service::{
    self, CaseView, ContractBody, CreateCaseBody, DeploymentBody, EvaluateBody, EvidenceBody, FollowupBody, NoteBody, ReasonBody,
    SecretBody, SourceBody, SourceTestBody, SourceTestView, SourceView, UpdateCaseBody,
};
use crate::state::App;

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/observation/cases", get(list_cases).post(create_case))
        .route("/observation/cases/{id}", get(show_case).patch(update_case))
        .route("/observation/cases/{id}/contract", put(set_contract))
        .route("/observation/cases/{id}/arm", post(arm))
        .route("/observation/cases/{id}/start", post(start))
        .route("/observation/cases/{id}/evaluate", post(evaluate))
        .route("/observation/cases/{id}/cancel", post(cancel))
        .route("/observation/cases/{id}/deployments", post(deploy))
        .route("/observation/cases/{id}/evidence", get(evidence).post(report_evidence))
        .route("/observation/cases/{id}/followup", post(followup))
        .route("/observation/incidents", get(list_incidents))
        .route("/observation/incidents/{id}/resolve", post(resolve_incident))
        .route("/observation/incidents/{id}/dismiss", post(dismiss_incident))
        .route("/observation/tasks/{task}", get(cases_of_task))
        .route("/observation/sources", get(list_sources).post(add_source))
        .route("/observation/sources/{name}", put(update_source).delete(remove_source))
        .route("/observation/sources/{name}/secret", put(set_secret).delete(clear_secret))
        .route("/observation/sources/{name}/test", post(test_source))
}

// --- path and query values ------------------------------------------------------------------------

/// A case key or number from the path (`O-7`, `7`). A malformed one is a bad request.
fn case_id(text: &str) -> ApiResult<i64> {
    parse_case_id(text).map_err(|e| ApiError::bad(e.to_string()))
}

/// An incident key or number from the path (`I-3`, `3`).
fn incident_id(text: &str) -> ApiResult<i64> {
    parse_incident_id(text).map_err(|e| ApiError::bad(e.to_string()))
}

/// A query value; an absent or blank one counts as not given.
fn given(text: Option<String>) -> Option<String> {
    text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

/// A whole number in a query value. The store checks the range of a `limit`.
fn number(name: &str, text: Option<String>) -> ApiResult<Option<i64>> {
    given(text).map(|t| t.parse::<i64>().map_err(|_| ApiError::bad(format!("{name} must be a whole number, not {t}")))).transpose()
}

/// A string-backed enum in a query value, such as `status=observing`.
fn word<T: FromStr<Err = GenieError>>(text: Option<String>) -> ApiResult<Option<T>> {
    given(text).map(|t| t.parse::<T>().map_err(|e| ApiError::bad(e.to_string()))).transpose()
}

#[derive(Debug, Default, Deserialize)]
struct CaseQuery {
    status: Option<String>,
    task: Option<String>,
    limit: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct EvidenceQuery {
    after: Option<String>,
    limit: Option<String>,
    criterion: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct IncidentQuery {
    status: Option<String>,
}

// --- cases ----------------------------------------------------------------------------------------

async fn list_cases(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<CaseQuery>) -> ApiResult<Json<Vec<ObservationCase>>> {
    let a = ctx.access(&app, None).await?;
    let filter = CaseFilter { status: word(q.status)?, task: given(q.task), limit: number("limit", q.limit)? };
    let cases = app.blocking(move |app| service::list(app, &a.project, &filter)).await?;
    Ok(Json(cases))
}

async fn create_case(State(app): State<Arc<App>>, ctx: Ctx, Body(b): Body<CreateCaseBody>) -> ApiResult<(StatusCode, Json<CaseView>)> {
    let a = ctx.access(&app, None).await?;
    a.write()?;
    let view = app.blocking(move |app| service::create(app, &a.project, &a.actor, b)).await?;
    Ok((StatusCode::CREATED, Json(view)))
}

async fn show_case(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<CaseView>> {
    let a = ctx.access(&app, None).await?;
    let id = case_id(&id)?;
    let view = app.blocking(move |app| service::show(app, &a.project, id)).await?;
    Ok(Json(view))
}

async fn update_case(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path(id): Path<String>,
    Body(b): Body<UpdateCaseBody>,
) -> ApiResult<Json<CaseView>> {
    let a = ctx.access(&app, None).await?;
    a.write()?;
    let id = case_id(&id)?;
    let view = app.blocking(move |app| service::update(app, &a.project, &a.actor, id, b, Utc::now())).await?;
    Ok(Json(view))
}

async fn set_contract(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path(id): Path<String>,
    Body(b): Body<ContractBody>,
) -> ApiResult<Json<CaseView>> {
    let a = ctx.access(&app, None).await?;
    a.write()?;
    let id = case_id(&id)?;
    let view = app.blocking(move |app| service::set_contract(app, &a.project, &a.actor, id, b, Utc::now())).await?;
    Ok(Json(view))
}

async fn arm(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<CaseView>> {
    let a = ctx.access(&app, None).await?;
    a.write()?;
    let id = case_id(&id)?;
    let view = app.blocking(move |app| service::arm(app, &a.project, &a.actor, id, Utc::now())).await?;
    Ok(Json(view))
}

async fn start(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<CaseView>> {
    let a = ctx.access(&app, None).await?;
    a.write()?;
    let id = case_id(&id)?;
    let view = app.blocking(move |app| service::start(app, &a.project, &a.actor, id, Utc::now())).await?;
    Ok(Json(view))
}

async fn evaluate(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Body(b): Body<EvaluateBody>) -> ApiResult<Json<CaseView>> {
    let a = ctx.access(&app, None).await?;
    a.write()?;
    let id = case_id(&id)?;
    let view = app.blocking(move |app| service::evaluate(app, &a.project, &a.actor, id, b)).await?;
    Ok(Json(view))
}

async fn cancel(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Body(b): Body<ReasonBody>) -> ApiResult<Json<CaseView>> {
    let a = ctx.access(&app, None).await?;
    a.write()?;
    let id = case_id(&id)?;
    let view = app.blocking(move |app| service::cancel(app, &a.project, &a.actor, id, b, Utc::now())).await?;
    Ok(Json(view))
}

async fn deploy(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Body(b): Body<DeploymentBody>) -> ApiResult<Json<CaseView>> {
    let a = ctx.access(&app, None).await?;
    a.write()?;
    let id = case_id(&id)?;
    let view = app.blocking(move |app| service::deploy(app, &a.project, &a.actor, id, b, Utc::now())).await?;
    Ok(Json(view))
}

async fn followup(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Body(b): Body<FollowupBody>) -> ApiResult<Json<CaseView>> {
    let a = ctx.access(&app, None).await?;
    a.write()?;
    let id = case_id(&id)?;
    let view = app.blocking(move |app| service::followup(app, &a.project, &a.actor, id, b)).await?;
    Ok(Json(view))
}

// --- evidence -------------------------------------------------------------------------------------

async fn evidence(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path(id): Path<String>,
    Query(q): Query<EvidenceQuery>,
) -> ApiResult<Json<Vec<Evidence>>> {
    let a = ctx.access(&app, None).await?;
    let id = case_id(&id)?;
    let filter = EvidenceFilter {
        after: number("after", q.after)?,
        limit: number("limit", q.limit)?,
        criterion: given(q.criterion),
        newest_first: false,
    };
    let rows = app.blocking(move |app| service::evidence(app, &a.project, id, &filter)).await?;
    Ok(Json(rows))
}

async fn report_evidence(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path(id): Path<String>,
    Body(b): Body<EvidenceBody>,
) -> ApiResult<Json<CaseView>> {
    let a = ctx.access(&app, None).await?;
    a.write()?;
    let id = case_id(&id)?;
    let view = app.blocking(move |app| service::report_evidence(app, &a.project, &a.actor, id, b, Utc::now())).await?;
    Ok(Json(view))
}

// --- incidents ------------------------------------------------------------------------------------

async fn list_incidents(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<IncidentQuery>) -> ApiResult<Json<Vec<Incident>>> {
    let a = ctx.access(&app, None).await?;
    let status: Option<IncidentStatus> = word(q.status)?;
    let incidents = app.blocking(move |app| service::incidents(app, &a.project, status)).await?;
    Ok(Json(incidents))
}

async fn resolve_incident(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path(id): Path<String>,
    Body(b): Body<NoteBody>,
) -> ApiResult<Json<Incident>> {
    let a = ctx.access(&app, None).await?;
    a.write()?;
    let id = incident_id(&id)?;
    let incident = app.blocking(move |app| service::resolve_incident(app, &a.project, &a.actor, id, b)).await?;
    Ok(Json(incident))
}

async fn dismiss_incident(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path(id): Path<String>,
    Body(b): Body<NoteBody>,
) -> ApiResult<Json<Incident>> {
    let a = ctx.access(&app, None).await?;
    a.write()?;
    let id = incident_id(&id)?;
    let incident = app.blocking(move |app| service::dismiss_incident(app, &a.project, &a.actor, id, b)).await?;
    Ok(Json(incident))
}

// --- tasks ----------------------------------------------------------------------------------------

async fn cases_of_task(State(app): State<Arc<App>>, ctx: Ctx, Path(task): Path<String>) -> ApiResult<Json<Vec<ObservationCase>>> {
    let a = ctx.access(&app, None).await?;
    let cases = app.blocking(move |app| service::for_task(app, &a.project, &task)).await?;
    Ok(Json(cases))
}

// --- sources and secrets (project admins, people only; §14, §15) ----------------------------------

async fn list_sources(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Vec<SourceView>>> {
    let a = ctx.access(&app, None).await?;
    a.admin()?;
    let sources = app.blocking(move |app| service::sources(app, &a.project, &a.actor)).await?;
    Ok(Json(sources))
}

async fn add_source(State(app): State<Arc<App>>, ctx: Ctx, Body(b): Body<SourceBody>) -> ApiResult<(StatusCode, Json<SourceView>)> {
    let a = ctx.access(&app, None).await?;
    a.admin()?;
    let view = app.blocking(move |app| service::add_source(app, &a.project, &a.actor, b)).await?;
    Ok((StatusCode::CREATED, Json(view)))
}

async fn update_source(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path(name): Path<String>,
    Body(b): Body<SourceBody>,
) -> ApiResult<Json<SourceView>> {
    let a = ctx.access(&app, None).await?;
    a.admin()?;
    let view = app.blocking(move |app| service::update_source(app, &a.project, &a.actor, &name, b)).await?;
    Ok(Json(view))
}

async fn remove_source(State(app): State<Arc<App>>, ctx: Ctx, Path(name): Path<String>) -> ApiResult<Json<Value>> {
    let a = ctx.access(&app, None).await?;
    a.admin()?;
    app.blocking(move |app| service::remove_source(app, &a.project, &a.actor, &name)).await?;
    Ok(Json(json!({ "ok": true })))
}

/// The request carries the secret; the response carries only its hint (§13.2, §15).
async fn set_secret(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path(name): Path<String>,
    Body(b): Body<SecretBody>,
) -> ApiResult<Json<SourceView>> {
    let a = ctx.access(&app, None).await?;
    a.admin()?;
    let view = app.blocking(move |app| service::set_source_secret(app, &a.project, &a.actor, &name, b)).await?;
    Ok(Json(view))
}

async fn clear_secret(State(app): State<Arc<App>>, ctx: Ctx, Path(name): Path<String>) -> ApiResult<Json<SourceView>> {
    let a = ctx.access(&app, None).await?;
    a.admin()?;
    let view = app.blocking(move |app| service::clear_source_secret(app, &a.project, &a.actor, &name)).await?;
    Ok(Json(view))
}

async fn test_source(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path(name): Path<String>,
    Body(b): Body<SourceTestBody>,
) -> ApiResult<Json<SourceTestView>> {
    let a = ctx.access(&app, None).await?;
    a.admin()?;
    let view = app.blocking(move |app| service::test_source(app, &a.project, &a.actor, &name, b, Utc::now())).await?;
    Ok(Json(view))
}
