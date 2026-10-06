//! The orchestrator console: a person's own agent session (`genie orchestrate`)
//! acts as a project's orchestrator while the server's orchestrator waits.
//!
//! Taking the console gives the session an orchestrator token and the
//! orchestrator's prompt; the session renews the console with that token and
//! gives it back when it ends. A console nobody renews lapses, and the server's
//! orchestrator carries on with the mail that came meanwhile. While a console is
//! held only its token takes the orchestrator's mail.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use super::ctx::Ctx;
use super::{ApiError, ApiResult};
use crate::runtime::{self, AgentKey};
use crate::state::App;

/// How long a console holds without being renewed.
pub const CONSOLE_TTL_SECS: i64 = 120;

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/orchestrator/console", get(status).post(take).delete(release)).route("/orchestrator/console/renew", post(renew))
}

fn ttl() -> chrono::Duration {
    chrono::Duration::seconds(CONSOLE_TTL_SECS)
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    v.strip_prefix("Bearer ").map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

/// While a console is held, only its session takes the orchestrator's mail.
pub(crate) async fn may_take_orchestrator_mail(app: &Arc<App>, project: &str, headers: &HeaderMap) -> ApiResult<()> {
    let (slug, token) = (project.to_string(), bearer(headers).unwrap_or_default());
    let held = app
        .blocking(move |app| app.with_server(|db| Ok(db.console(&slug)?.filter(|_| !db.is_console_token(&slug, &token).unwrap_or(false)))))
        .await?;
    match held {
        Some(c) => Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("the orchestrator console of {} is held by {}: the server's orchestrator waits", c.project, c.user),
        )),
        None => Ok(()),
    }
}

async fn status(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let project = access.project.clone();
    let console = app.blocking(move |app| app.with_server(|db| db.console(&project))).await?;
    Ok(Json(json!({ "project": access.project, "console": console, "ttlSecs": CONSOLE_TTL_SECS })))
}

#[derive(Deserialize, Default)]
struct TakeBody {
    #[serde(default)]
    force: bool,
}

/// A project admin takes the console for their own agent session.
async fn take(State(app): State<Arc<App>>, ctx: Ctx, body: Option<Json<TakeBody>>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    if access.agent {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "a person takes the orchestrator console, not an agent"));
    }
    access.admin()?;
    let force = body.map(|Json(b)| b.force).unwrap_or_default();
    let (project, user) = (access.project.clone(), access.actor.name.clone());
    let (console, token, prompt, model, thinking) = app
        .blocking(move |app| {
            let spec = runtime::console_spec(app, &project, &user)?;
            let (console, token) = app.with_server(|db| db.take_console(&project, &user, Some(&spec.role_id), ttl(), force))?;
            // The server's session stops now rather than at the next sweep.
            crate::sessions::reset(app, &AgentKey::Orchestrator { project: project.clone() });
            Ok((console, token, spec.prompt, spec.model, spec.thinking))
        })
        .await
        .map_err(|e| {
            let e: ApiError = e.into();
            if e.message.contains(" is held by ") { ApiError::new(StatusCode::CONFLICT, e.message) } else { e }
        })?;
    // Mail already waiting for the orchestrator now waits for the console to lapse: the scheduler works out when.
    app.wake_runtime.notify_one();
    Ok(Json(json!({
        "console": console,
        "token": token,
        "ttlSecs": CONSOLE_TTL_SECS,
        "prompt": prompt,
        "model": model,
        "thinking": thinking,
    })))
}

/// The console's session keeps it (with the console's token).
async fn renew(State(app): State<Arc<App>>, ctx: Ctx, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let token = bearer(&headers).ok_or_else(ApiError::unauthorized)?;
    let project = access.project.clone();
    let console = app.blocking(move |app| app.with_server(|db| db.renew_console(&project, &token, ttl()))).await.map_err(|e| {
        let e: ApiError = e.into();
        ApiError::new(StatusCode::CONFLICT, e.message)
    })?;
    Ok(Json(json!({ "console": console })))
}

/// The console's session gives it back when it ends; a project admin may take it away.
async fn release(State(app): State<Arc<App>>, ctx: Ctx, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let token = if access.agent {
        Some(bearer(&headers).ok_or_else(ApiError::unauthorized)?)
    } else {
        access.admin()?;
        None
    };
    let project = access.project.clone();
    let released = app.blocking(move |app| app.with_server(|db| db.release_console(&project, token.as_deref()))).await?;
    // The server's orchestrator picks up the mail that came meanwhile.
    app.wake_runtime.notify_one();
    Ok(Json(json!({ "released": released })))
}
