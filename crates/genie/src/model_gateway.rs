//! Server-owned model I/O for budgeted workers. The seam is one enrolled Pi
//! profile: normal turns, tool loops and compaction all require admission here.
//! Provider credentials and authoritative accounting stay with the server.

mod profile;
mod protocol;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use genie_core::model_calls::{Admission, Call, Money, NewCall, Worker};
use genie_core::server_db::{Principal, hash_secret, new_secret};
use genie_core::usage::Tokens;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Notify, Semaphore, mpsc, oneshot};

use crate::http::ctx::{Access, Ctx};
use crate::http::{ApiError, ApiResult};
use crate::llm_key::Keys;
use crate::runtime::{AgentKey, AgentSpec};
use crate::state::{App, AppError, AppResult};

pub struct Gateway {
    client: reqwest::Client,
    slots: Arc<Semaphore>,
    changed: Notify,
}
impl Gateway {
    pub(crate) fn new(concurrent: usize) -> AppResult<Self> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map_err(|_| AppError::Internal("cannot initialize the bounded model transport".into()))?;
        Ok(Self { client, slots: Arc::new(Semaphore::new(concurrent.max(1))), changed: Notify::new() })
    }
}

pub(crate) fn enabled(app: &App) -> bool {
    let b = app.cfg.budgets;
    b.per_task > 0.0 || b.per_epic > 0.0 || b.per_day > 0.0
}

#[derive(Clone)]
pub(crate) struct Prepared {
    pub path: PathBuf,
    pub source: PathBuf,
    key_variables: Vec<String>,
}

/// Only trusted runtime code can enroll a worker; external clients cannot pick
/// a billable provider, credential, model bound or execution identity.
pub(crate) async fn prepare(
    app: &Arc<App>,
    key: &AgentKey,
    spec: &AgentSpec,
    keys: &Keys,
    token: &str,
    dir: &Path,
) -> Result<Prepared, String> {
    let profile = profile::resolve(app, spec.model.as_deref().ok_or("bounded work needs an explicit model")?, keys, dir).await?;
    let (project, spec, keys, token, dir, agent) =
        (key.project().to_string(), spec.clone(), keys.clone(), token.to_string(), dir.to_path_buf(), key.label());
    app.blocking(move |app| {
        let initiator = app.with_server(|db| {
            let user = spec.initiator.as_deref().map(|login| db.user_by_login(login)).transpose()?.flatten();
            if db.user_count()? > 0 && user.as_ref().is_none_or(|u| u.disabled) {
                return Err(genie_core::GenieError::Denied("budgeted work needs an enabled initiator account".into()));
            }
            Ok(user.map(|u| u.id))
        })?;
        profile.key(app, &keys, initiator).map_err(AppError::Conflict)?;
        let id = format!("mw_{}", new_secret());
        let path = dir.join("models").join(&id);
        let url = format!("http://127.0.0.1:{}/api/agent/model-gateway/{id}/v1", app.cfg.port);
        profile.materialize(&path, &url).map_err(AppError::Internal)?;
        let worker = Worker {
            id,
            token_hash: hash_secret(&token),
            agent,
            task: spec.task.clone(),
            model: spec.model.clone().unwrap_or_default(),
            input_limit: profile.model.context_window,
            output_limit: profile.model.max_tokens,
            profile_fingerprint: hash_secret(
                &serde_json::to_string(&profile.model).map_err(|_| AppError::Internal("invalid model metadata".into()))?,
            ),
            profile: serde_json::to_value(&profile).map_err(|_| AppError::Internal("invalid model profile".into()))?,
            initiator,
            execution_stamp: spec.execution_stamp.clone(),
            closed: false,
        };
        // Do not call with_current_execution here: it already holds the tracker
        // lock while invoking its action. Enrollment needs that same tracker.
        let _gate = app.execution_gate.lock().map_err(|_| AppError::Internal("execution gate poisoned".into()))?;
        let principal = app
            .with_server(|db| db.resolve_token(&token))?
            .ok_or_else(|| AppError::Forbidden("model worker token is no longer active".into()))?;
        validate_principal(app, &project, &worker, &principal)?;
        app.with_tracker(&project, |t| {
            t.tx(|| {
                if let Some(task) = &worker.task
                    && worker.execution_stamp.as_ref() != Some(&t.execution().stamp(task)?)
                {
                    return Err(genie_core::GenieError::Denied("execution changed before model worker enrollment".into()));
                }
                t.model_calls().enroll(&worker)
            })
        })?;
        Ok(Prepared { key_variables: profile.custom_key_variables().map_err(AppError::Conflict)?, source: profile.source, path })
    })
    .await
    .map_err(|e| e.to_string())
}

impl Prepared {
    pub(crate) fn protect_files(&self, app: &App, plan: &mut crate::sandbox::Plan) {
        use crate::sandbox::Access::Hidden;
        // Docker and *_FILE credentials remain readable files after their
        // environment variables are removed. Hide their source paths too.
        plan.set(Path::new("/run/secrets"), Hidden);
        let variables = crate::llm_key::PROVIDERS.iter().map(|p| p.key_var).chain(self.key_variables.iter().map(String::as_str));
        for variable in variables.filter(|name| !name.is_empty()) {
            let name = format!("{variable}_FILE");
            if let Some(path) = app.cfg.runtime.env.get(&name).cloned().or_else(|| std::env::var(&name).ok()) {
                plan.set(Path::new(&path), Hidden);
            }
        }
    }

    pub(crate) fn apply(&self, app: &App, command: &mut tokio::process::Command, token: &str) {
        // Do this after every configured/per-user environment override. The
        // worker has only its existing scoped Genie credential, never a model key.
        for provider in crate::llm_key::PROVIDERS {
            if !provider.key_var.is_empty() {
                command.env_remove(provider.key_var).env_remove(format!("{}_FILE", provider.key_var));
            }
        }
        for name in &self.key_variables {
            command.env_remove(name).env_remove(format!("{name}_FILE"));
        }
        for (name, _) in std::env::vars().chain(app.cfg.runtime.env.clone()) {
            if name.starts_with("PI_JEV_") {
                command.env_remove(name);
            }
        }
        command
            .env("GENIE_TOKEN", token)
            .env("PI_CODING_AGENT_DIR", &self.path)
            .env("GENIE_MODEL_GATEWAY", "1")
            .env("PI_OFFLINE", "1")
            .env("PI_SKIP_VERSION_CHECK", "1")
            .env("PI_TELEMETRY", "0");
    }
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/agent/model-gateway/{worker}/v1/chat/completions", post(completions))
        .route("/budget/calls", get(list))
        .route("/budget/calls/{id}", get(show))
        .route("/budget/calls/{id}/reconcile", post(reconcile))
}

async fn completions(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    UrlPath(worker): UrlPath<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> ApiResult<Response> {
    let access = ctx.access(&app, None).await?;
    if !access.agent {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "only an enrolled worker uses the model gateway"));
    }
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(ApiError::unauthorized)?
        .trim()
        .to_string();
    // The server owns the future, including accounting after a client drops its
    // connection. No detached worker-delivered receipt is needed after revocation.
    let (send, receive) = oneshot::channel();
    tokio::spawn(async move {
        let mut reply = Delivery { reply: Some(send), stream: None, completed: None };
        let result = run(&app, &access.project, &worker, &token, body, &mut reply).await;
        if let Err(error) = result {
            reply.send(error.into_response());
        }
    });
    receive.await.map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "model gateway stopped before returning a response"))
}

fn validate_principal(app: &App, project: &str, worker: &Worker, principal: &Principal) -> AppResult<()> {
    let Principal::Agent { project: token_project, name, team, job, .. } = principal else {
        return Err(AppError::Forbidden("the model gateway requires an agent token".into()));
    };
    let agent = if let Some(team) = team {
        format!("{team}/{name}")
    } else if let Some(job) = job {
        format!("job/{job}")
    } else {
        name.clone()
    };
    if token_project != project || agent != worker.agent || worker.closed {
        return Err(AppError::Forbidden("the model enrollment does not belong to this worker".into()));
    }
    if let Some(task) = &worker.task {
        crate::tasks::ensure_running(app, project, task)?;
    }
    if crate::conversations::request_id(name).is_some() {
        crate::conversations::validate_agent(app, project, name)?;
    }
    app.with_server(|db| {
        if let Some(id) = worker.initiator {
            let user = db.user(id)?;
            if user.disabled || !db.project_role(project, &user)?.is_some_and(|role| role.can_write()) {
                return Err(genie_core::GenieError::Denied("the model initiator no longer has write access to this project".into()));
            }
        } else if db.user_count()? > 0 {
            return Err(genie_core::GenieError::Denied("budgeted work has no current initiator authority".into()));
        }
        Ok(())
    })
}

fn authorize(app: &App, project: &str, id: &str, token: &str) -> AppResult<(Worker, profile::Profile)> {
    let worker = app.with_tracker(project, |t| t.model_calls().worker(id))?;
    if worker.token_hash != hash_secret(token) {
        return Err(AppError::Forbidden("this token is not enrolled for that model worker".into()));
    }
    let principal = app
        .with_server(|db| db.resolve_token(token))?
        .ok_or_else(|| AppError::Forbidden("model worker token is expired or revoked".into()))?;
    validate_principal(app, project, &worker, &principal)?;
    let profile: profile::Profile =
        serde_json::from_value(worker.profile.clone()).map_err(|_| AppError::Internal("invalid enrolled model profile".into()))?;
    profile.validate().map_err(AppError::Conflict)?;
    Ok((worker, profile))
}

async fn run(app: &Arc<App>, project: &str, id: &str, token: &str, body: Value, reply: &mut Delivery) -> Result<(), ApiError> {
    let (p, w, secret) = (project.to_string(), id.to_string(), token.to_string());
    let (worker, profile) = app.blocking(move |app| authorize(app, &p, &w, &secret)).await?;
    let request = protocol::Request::parse(body, &profile.model.id, worker.output_limit).map_err(ApiError::bad)?;
    let timeout = request_timeout(app);
    // Provider calls have a separate semaphore: a runtime slot can own a call,
    // but must never wait to acquire that same runtime slot a second time.
    let permit = tokio::time::timeout(timeout, app.model_gateway.slots.clone().acquire_owned())
        .await
        .map_err(|_| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "model capacity wait timed out"))?
        .map_err(|_| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "model gateway is stopping"))?;
    let deadline = tokio::time::Instant::now() + timeout;
    let call_id = format!("mc_{}", new_secret());
    let (call, credential) = loop {
        if reply.reply.as_ref().is_none_or(oneshot::Sender::is_closed) {
            return Ok(());
        }
        let changed = app.model_gateway.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        let (p, w, secret, call_id, cap) = (project.to_string(), id.to_string(), token.to_string(), call_id.clone(), request.output_limit);
        let attempt = app
            .blocking(move |app| {
                let _gate = app.execution_gate.lock().map_err(|_| AppError::Internal("execution gate poisoned".into()))?;
                let (worker, profile) = authorize(app, &p, &w, &secret)?;
                let credential = profile.key(app, &Keys::Server, worker.initiator).map_err(AppError::Conflict)?;
                let prices = app.budget_prices()?;
                let tariff = crate::budget::budget_price(&prices, &worker.model)
                    .copied()
                    .ok_or_else(|| AppError::Conflict("model tariff is unavailable".into()))?;
                let task = app
                    .with_server(|db| db.resolve_token(&secret))?
                    .and_then(|principal| match principal {
                        Principal::Agent { name, team, job, .. } => Some((name, team, job)),
                        _ => None,
                    })
                    .ok_or_else(|| AppError::Forbidden("model work token is no longer active".into()))?;
                let current_task = worker.task.clone().or(crate::runtime::agent_task(app, &p, &task.0, task.1.as_deref(), task.2)?);
                let admission = app.with_tracker(&p, |t| {
                    crate::conversations::ensure_active(t, &task.0)?;
                    t.model_calls().admit(
                        NewCall { id: call_id, worker: w, task: current_task.clone(), output_limit: cap, tariff },
                        |scope, day, t| crate::budget::allowance(t, &prices, &app.cfg.budgets, scope, day),
                    )
                })?;
                Ok((admission, credential, current_task))
            })
            .await?;
        match attempt {
            (Admission::Granted(call), credential, _) => break (call, credential),
            (Admission::Existing(_), _, _) => {
                return Err(ApiError::new(StatusCode::CONFLICT, "an admitted call cannot be dispatched twice"));
            }
            (Admission::Refused(refusal), _, _) if refusal.temporarily_reserved() => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "model allowance is temporarily reserved by active calls"));
                }
                tokio::select! { _ = changed => {}, _ = tokio::time::sleep_until(deadline.min(tokio::time::Instant::now() + Duration::from_secs(1))) => {} }
            }
            (Admission::Refused(refusal), _, current_task) => {
                let note = format!(
                    "budget: {} needs ${:.9} of model allowance; ${:.9} is occupied of ${:.9}. Review uncertain calls or raise the configured limit before resuming.",
                    refusal.scope.words(),
                    refusal.needed.as_dollars(),
                    refusal.used.as_dollars(),
                    refusal.limit.as_dollars()
                );
                let (p, task, note2) = (project.to_string(), current_task, note.clone());
                if let Some(task) = task {
                    app.blocking(move |app| crate::tasks::pause_for_budget(app, &p, &task, &note2)).await?;
                }
                return Err(ApiError::new(StatusCode::PAYMENT_REQUIRED, note));
            }
        }
    };
    let outcome = forward(app, &profile, &credential, request, timeout, reply).await;
    let failure = outcome.as_ref().err().cloned();
    let (p, call_id) = (project.to_string(), call.id.clone());
    let accounted = app
        .blocking(move |app| {
            let receipt = app.with_tracker(&p, |t| match outcome {
                Ok(tokens) => t.model_calls().settle(&call_id, tokens),
                Err(note) => t.model_calls().uncertain(&call_id, &note),
            });
            let receipt = match receipt {
                Ok(receipt) => receipt,
                Err(error) => {
                    // Keep terminal uncertainty administratively recoverable
                    // even when an invalid receipt cannot be settled.
                    let _ = app.with_tracker(&p, |t| {
                        t.model_calls()
                            .uncertain(&call_id, "Authoritative usage could not be settled; verify the provider receipt before reconciling")
                    });
                    return Err(error);
                }
            };
            if let Err(error) = crate::budget::enforce_call(app, &p, &receipt) {
                eprintln!("genie budget: {p}: {error}");
            }
            if let Some(task) = receipt.task.as_deref() {
                let fault =
                    app.with_tracker(&p, |t| t.model_calls().profile_fault(&t.model_calls().worker(&receipt.worker)?.profile_fingerprint))?;
                if let Some(note) = fault {
                    crate::tasks::pause_for_budget(app, &p, task, &note)?;
                }
            }
            Ok(())
        })
        .await;
    app.model_gateway.changed.notify_waiters();
    drop(permit);
    if let Err(error) = accounted {
        eprintln!("genie model gateway: receipt {} was not settled: {error}", call.id);
        return Err(ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "model usage could not be settled; reservation retained"));
    }
    if let Some(note) = failure {
        return Err(ApiError::new(StatusCode::BAD_GATEWAY, note));
    }
    if let Some(response) = reply.completed.take() {
        reply.send(response);
    }
    if let Some(send) = reply.stream.take() {
        let _ = tokio::time::timeout(Duration::from_secs(1), send.send(Frame::End)).await;
    }
    Ok(())
}

fn request_timeout(app: &App) -> Duration {
    let turn = app.cfg.runtime.turn_timeout_secs.max(1);
    Duration::from_secs(if app.cfg.runtime.run_timeout_secs > 0 { turn.min(app.cfg.runtime.run_timeout_secs) } else { turn })
}

async fn forward(
    app: &Arc<App>,
    profile: &profile::Profile,
    credential: &str,
    request: protocol::Request,
    timeout: Duration,
    reply: &mut Delivery,
) -> Result<Tokens, String> {
    let url = format!("{}/chat/completions", profile.model.base_url.trim_end_matches('/'));
    let mut response = app
        .model_gateway
        .client
        .post(url)
        .bearer_auth(credential)
        .json(&request.body)
        .timeout(timeout)
        .send()
        .await
        .map_err(|_| "provider transport failed; maximum exposure retained")?;
    if !response.status().is_success() {
        // An error status alone does not prove a provider did no billable work.
        return Err(format!("provider returned HTTP {}; maximum exposure retained", response.status().as_u16()));
    }
    let content_type = response.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or_default();
    if request.stream {
        if !content_type.split(';').next().is_some_and(|v| v.trim() == "text/event-stream") {
            return Err("provider returned an unexpected stream type".into());
        }
        let (send, receive) = mpsc::channel::<Frame>(8);
        let body = Body::from_stream(futures_util::stream::unfold((receive, false), |(mut rx, ended)| async move {
            if ended {
                return None;
            }
            match rx.recv().await {
                Some(Frame::Data(bytes)) => Some((Ok(bytes), (rx, false))),
                Some(Frame::End) => Some((Ok(Bytes::from_static(b"data: [DONE]\n\n")), (rx, true))),
                None => Some((Err(std::io::Error::other("model stream ended without settled usage")), (rx, true))),
            }
        }));
        reply.stream = Some(send);
        // The server retains the sender until accounting commits. [DONE] is
        // emitted only on Frame::End, so clients cannot finish ahead of usage.
        reply.send(([(header::CONTENT_TYPE, "text/event-stream"), (header::CACHE_CONTROL, "no-store")], body).into_response());
        let mut parser = protocol::Stream::default();
        while let Some(chunk) = response.chunk().await.map_err(|_| "provider stream was interrupted; maximum exposure retained")? {
            let bytes = parser.push(&chunk)?;
            if !bytes.is_empty()
                && let Some(send) = reply.stream.as_ref()
                && !matches!(tokio::time::timeout(Duration::from_secs(1), send.send(Frame::Data(bytes.into()))).await, Ok(Ok(())))
            {
                // Continue consuming usage for a lost/slow client. Closing the
                // body reports a stream error; no output chunks are silently lost.
                reply.stream.take();
            }
        }
        parser.finish()
    } else {
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "provider response was interrupted; maximum exposure retained")? {
            if bytes.len().saturating_add(chunk.len()) > 16 * 1024 * 1024 {
                return Err("provider response exceeds the bounded wire limit".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid provider JSON; maximum exposure retained")?;
        let tokens = protocol::usage(&value["usage"])?;
        reply.completed = Some(([(header::CONTENT_TYPE, "application/json"), (header::CACHE_CONTROL, "no-store")], bytes).into_response());
        Ok(tokens)
    }
}

enum Frame {
    Data(Bytes),
    End,
}
struct Delivery {
    reply: Option<oneshot::Sender<Response>>,
    stream: Option<mpsc::Sender<Frame>>,
    completed: Option<Response>,
}
impl Delivery {
    fn send(&mut self, response: Response) {
        if let Some(send) = self.reply.take() {
            let _ = send.send(response);
        }
    }
}

#[derive(Deserialize)]
struct ListQuery {
    task: Option<String>,
}
async fn list(State(app): State<Arc<App>>, ctx: Ctx, Query(query): Query<ListQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.admin()?;
    Ok(Json(
        app.blocking(move |app| {
            let calls = app.with_tracker(&access.project, |t| t.model_calls().list(query.task.as_deref(), 200))?;
            Ok(json!({"calls":calls.into_iter().map(public_call).collect::<Vec<_>>()}))
        })
        .await?,
    ))
}
async fn show(State(app): State<Arc<App>>, ctx: Ctx, UrlPath(id): UrlPath<String>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.admin()?;
    Ok(Json(app.blocking(move |app| app.with_tracker(&access.project, |t| t.model_calls().call(&id)).map(public_call)).await?))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Reconciliation {
    cost_nanos: i64,
    note: String,
}
async fn reconcile(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    UrlPath(id): UrlPath<String>,
    crate::http::Body(body): crate::http::Body<Reconciliation>,
) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.admin()?;
    let value = app.blocking(move |app| reconcile_call(app, &access, &id, body.cost_nanos, &body.note)).await?;
    app.model_gateway.changed.notify_waiters();
    app.wake_runtime.notify_one();
    Ok(Json(value))
}
fn reconcile_call(app: &App, access: &Access, id: &str, cost: i64, note: &str) -> AppResult<Value> {
    if note.len() > 8000 {
        return Err(AppError::Bad("reconciliation evidence is too long".into()));
    }
    let cost = Money::nanos(cost)?;
    let receipt = app.with_tracker(&access.project, |t| t.model_calls().reconcile(&access.actor, id, cost, note))?;
    if let Err(error) = crate::budget::enforce_call(app, &access.project, &receipt) {
        eprintln!("genie budget: {}: {error}", access.project);
    }
    Ok(public_call(receipt))
}
fn public_call(call: Call) -> Value {
    let mut value = serde_json::to_value(call).unwrap_or(Value::Null);
    // An intake agent label includes a private response ID. Financial evidence
    // exposes opaque call IDs, never another person's conversation identity.
    if let Some(object) = value.as_object_mut() {
        object.remove("agent");
    }
    value
}
