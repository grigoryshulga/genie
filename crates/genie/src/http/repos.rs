//! Repositories of a project: the hosts they live on, the project's list, a task's
//! repositories (docs/platform/git-repositories.md). Agents read the list with
//! their effective rules; administrators change it.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use genie_core::Role;
use genie_core::repos::{NewRepo, ProjectRepo, RepoPatch};
use genie_core::secrets::Secret;
use serde::Deserialize;
use serde_json::{Value, json};

use super::ctx::{Access, Ctx};
use super::tasks::{Touch, changed, in_scope};
use super::{ApiError, ApiResult};
use crate::git::delivery::{self, Caller, DeliveryError, OpenArgs};
use crate::git::hosts::{self, Hosts};
use crate::git::policy::Policy;
use crate::git::service::{self, AgentId};
use crate::git::store;
use crate::state::{App, AppError};

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/git/hosts", get(list_hosts))
        .route("/git/hosts/{id}/check", post(check_host))
        .route("/repos/{name}/check", post(check_repo))
        .route("/repos", get(list).post(add))
        .route("/repos/{name}", patch(update).delete(remove))
        .route("/repos/{name}/sync", post(sync))
        .route("/tasks/{id}/repos", get(task_repos).put(set_task_repos))
        .route("/tasks/{id}/repos/{name}/cr", get(cr_show).post(cr_open))
        .route("/tasks/{id}/repos/{name}/cr/comments", get(cr_comments).post(cr_comment))
        .route("/tasks/{id}/repos/{name}/cr/merge", post(cr_merge))
        .route("/tasks/{id}/repos/{name}/cr/rerun", post(cr_rerun))
}

impl From<DeliveryError> for ApiError {
    fn from(e: DeliveryError) -> Self {
        use crate::git::provider::ApiError as Host;
        let status = match &e {
            DeliveryError::Denied(_) => StatusCode::FORBIDDEN,
            DeliveryError::Invalid(_) | DeliveryError::NotFound(_) => StatusCode::UNPROCESSABLE_ENTITY,
            DeliveryError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            DeliveryError::Host(Host::RateLimited(_)) => StatusCode::TOO_MANY_REQUESTS,
            DeliveryError::Host(Host::Rejected(_) | Host::NotFound(_) | Host::Unsupported(_)) => StatusCode::UNPROCESSABLE_ENTITY,
            DeliveryError::Host(_) => StatusCode::BAD_GATEWAY,
        };
        ApiError::new(status, e.to_string())
    }
}

/// The caller of a request operation and the task it concerns (an agent only for its own).
async fn cr_caller(app: &Arc<App>, ctx: &Ctx, id: &str, touch: Touch) -> ApiResult<(Access, String, Caller)> {
    let access = ctx.access(app, None).await?;
    if touch == Touch::Edit {
        access.write()?;
    }
    in_scope(app, &access, id, touch).await?;
    let (slug, raw) = (access.project.clone(), id.to_string());
    let task = app.blocking(move |app| app.with_tracker(&slug, |t| t.normalize_id(&raw))).await?;
    let caller = Caller { name: access.actor.name.clone(), role: access.actor.role, agent: agent_id(&access) };
    Ok((access, task, caller))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct OpenBody {
    title: Option<String>,
    body: String,
    base: Option<String>,
    draft: bool,
}

async fn cr_open(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path((id, name)): Path<(String, String)>,
    Json(b): Json<OpenBody>,
) -> ApiResult<Json<Value>> {
    let (access, task, caller) = cr_caller(&app, &ctx, &id, Touch::Edit).await?;
    let out = delivery::open_request(
        &app,
        &access.project,
        &task,
        &name,
        &caller,
        OpenArgs { title: b.title, body: b.body, base: b.base, draft: b.draft },
    )
    .await?;
    changed(&app);
    Ok(Json(out))
}

async fn cr_show(State(app): State<Arc<App>>, ctx: Ctx, Path((id, name)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (access, task, caller) = cr_caller(&app, &ctx, &id, Touch::Note).await?;
    Ok(Json(delivery::show(&app, &access.project, &task, &name, &caller).await?))
}

async fn cr_comments(State(app): State<Arc<App>>, ctx: Ctx, Path((id, name)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (access, task, caller) = cr_caller(&app, &ctx, &id, Touch::Note).await?;
    Ok(Json(delivery::comments(&app, &access.project, &task, &name, &caller).await?))
}

#[derive(Debug, Deserialize)]
struct CommentBody {
    text: String,
}

async fn cr_comment(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path((id, name)): Path<(String, String)>,
    Json(b): Json<CommentBody>,
) -> ApiResult<Json<Value>> {
    let (access, task, caller) = cr_caller(&app, &ctx, &id, Touch::Note).await?;
    access.write()?;
    delivery::comment(&app, &access.project, &task, &name, &caller, &b.text).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct MergeBody {
    /// A person asks to be held to the repository's policy (the merge button of an agent's request).
    policy: bool,
}

async fn cr_merge(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path((id, name)): Path<(String, String)>,
    body: Option<Json<MergeBody>>,
) -> ApiResult<Json<Value>> {
    let (access, task, caller) = cr_caller(&app, &ctx, &id, Touch::Edit).await?;
    let by_policy = body.is_some_and(|b| b.policy);
    let out = delivery::merge(&app, &access.project, &task, &name, &caller, by_policy).await?;
    changed(&app);
    Ok(Json(out))
}

/// Rerun the failed checks of the task's watched commit, within the configured limits.
async fn cr_rerun(State(app): State<Arc<App>>, ctx: Ctx, Path((id, name)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let (access, task, caller) = cr_caller(&app, &ctx, &id, Touch::Edit).await?;
    let out = delivery::rerun(&app, &access.project, &task, &name, &caller).await?;
    changed(&app);
    Ok(Json(out))
}

/// Who the caller is as an agent, when it is one.
pub fn agent_id(access: &Access) -> Option<AgentId> {
    access.agent.then(|| AgentId {
        role: access.actor.role,
        role_id: access.agent_role_id.clone(),
        team: access.agent_team.clone(),
        job: access.agent_job,
    })
}

fn repo_json(app: &App, hosts: &Hosts, repo: &ProjectRepo, agent: Option<&AgentId>) -> Value {
    let resolved = store::resolved(app, repo);
    let host = match hosts.map.get(&repo.host) {
        Some(h) => {
            json!({ "id": h.id, "kind": h.kind, "url": h.url, "webUrl": h.web_url(&repo.remote) })
        }
        None => {
            let why = hosts.errors.iter().find(|e| e.starts_with(&format!("host {}:", repo.host))).cloned();
            json!({ "id": repo.host, "error": why.unwrap_or_else(|| "not configured in git.json".to_string()) })
        }
    };
    let mut v = serde_json::to_value(repo).unwrap_or(Value::Null);
    v["defaultBranch"] = json!(resolved.default_branch);
    v["host"] = host;
    v["policyValid"] = json!(Policy::parse(&repo.policy).is_ok());
    // The repository's own access token: what can be shown (never the value). Agents do not see it at all.
    if agent.is_none() {
        v["token"] = match app.with_server(|db| db.repo_token_info(&repo.project, &repo.name)) {
            Ok(Some(i)) => json!({ "set": true, "hint": i.hint, "updated": i.updated, "unreadable": i.unreadable }),
            _ => json!({ "set": false }),
        };
    }
    if let Some(a) = agent {
        v["effective"] = match service::effective_for(app, &repo.project, repo, a) {
            Ok(e) => json!({
                "read": e.read,
                "write": e.write,
                "branch": e.task_branch(),
                "rules": e.describe(),
                "policy": e.policy,
            }),
            Err(e) => json!({ "error": e.to_string() }),
        };
    }
    v
}

/// The hosts of `git.json` (no secrets): for the people who attach repositories, the admins of a project and of the server.
async fn list_hosts(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    if ctx.server_admin().is_err() {
        ctx.access(&app, None).await?.admin()?;
    }
    let h = app.blocking(|app| Ok(hosts::load(&app.data))).await?;
    Ok(Json(json!({ "hosts": h.map.values().map(|h| h.summary()).collect::<Vec<_>>(), "errors": h.errors })))
}

async fn check_host(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    ctx.server_admin()?;
    let lines = crate::git::check::host(&app, &id).await;
    Ok(Json(json!({ "ok": !crate::git::check::failed(&lines), "lines": lines })))
}

#[derive(Debug, Default, Deserialize)]
struct CheckQuery {
    /// Push a throw-away branch to prove the token can push.
    probe: Option<String>,
}

async fn check_repo(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path(name): Path<String>,
    axum::extract::Query(q): axum::extract::Query<CheckQuery>,
) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.admin()?;
    let probe = q.probe.is_some_and(|p| p != "0" && p != "false");
    let lines = crate::git::check::repo(&app, &access.project, &name, probe).await;
    Ok(Json(json!({ "ok": !crate::git::check::failed(&lines), "lines": lines })))
}

async fn list(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let agent = agent_id(&access);
    let out = app
        .blocking(move |app| {
            let h = hosts::load(&app.data);
            let repos = app.with_server(|db| db.repos(&access.project))?;
            Ok(repos.iter().map(|r| repo_json(app, &h, r, agent.as_ref())).collect::<Vec<_>>())
        })
        .await?;
    Ok(Json(json!(out)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddBody {
    name: String,
    host: String,
    remote: String,
    mount: Option<String>,
    default_branch: Option<String>,
    access: Option<String>,
    policy: Option<Value>,
    /// The access token (PAT) on the host, kept sealed on the server; a repository's own wins over the host's.
    token: Option<Secret>,
}

fn check_policy(policy: &Option<Value>) -> ApiResult<()> {
    if let Some(p) = policy {
        Policy::parse(p).map_err(ApiError::bad)?;
    }
    Ok(())
}

async fn add(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<AddBody>) -> ApiResult<(StatusCode, Json<Value>)> {
    let access = ctx.access(&app, None).await?;
    access.admin()?;
    check_policy(&b.policy)?;
    let out = app
        .blocking(move |app| {
            let new = NewRepo {
                name: b.name,
                host: b.host,
                remote: b.remote,
                mount: b.mount,
                default_branch: b.default_branch,
                access: b.access,
                policy: b.policy,
                token: b.token,
            };
            let (repo, warning) = service::add_repo(app, &access.project, new)?;
            let mut v = repo_json(app, &hosts::load(&app.data), &repo, None);
            v["warning"] = json!(warning);
            Ok(v)
        })
        .await?;
    Ok((StatusCode::CREATED, Json(out)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PatchBody {
    mount: Option<String>,
    default_branch: Option<String>,
    access: Option<String>,
    policy: Option<Value>,
    /// A new access token; an empty one removes the repository's own (the host's applies again).
    token: Option<Secret>,
}

async fn update(State(app): State<Arc<App>>, ctx: Ctx, Path(name): Path<String>, Json(b): Json<PatchBody>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.admin()?;
    check_policy(&b.policy)?;
    let out = app
        .blocking(move |app| {
            let repo = app.with_server(|db| {
                db.update_repo(
                    &access.project,
                    &name,
                    RepoPatch { mount: b.mount, default_branch: b.default_branch, access: b.access, policy: b.policy, token: b.token },
                )
            })?;
            Ok(repo_json(app, &hosts::load(&app.data), &repo, None))
        })
        .await?;
    Ok(Json(out))
}

async fn remove(State(app): State<Arc<App>>, ctx: Ctx, Path(name): Path<String>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.admin()?;
    app.blocking(move |app| app.with_server(|db| db.remove_repo(&access.project, &name))).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn sync(State(app): State<Arc<App>>, ctx: Ctx, Path(name): Path<String>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    if access.agent {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "agents cannot refresh mirrors"));
    }
    let out = app
        .blocking(move |app| {
            let repo = app.with_server(|db| db.repo(&access.project, &name))?;
            let info = store::sync(app, &repo).map_err(AppError::Internal)?;
            if repo.default_branch.is_empty()
                && let Some(d) = info["defaultBranch"].as_str()
            {
                app.with_server(|db| {
                    db.update_repo(&repo.project, &repo.name, RepoPatch { default_branch: Some(d.to_string()), ..Default::default() })
                })?;
            }
            Ok(info)
        })
        .await?;
    Ok(Json(out))
}

async fn task_repos(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let agent = agent_id(&access);
    let out = app
        .blocking(move |app| {
            let task = app.with_tracker(&access.project, |t| t.normalize_id(&id))?;
            let rows = app.with_server(|db| db.task_repos(&access.project, &task))?;
            let mut v = json!({ "task": task, "repos": rows });
            if let Some(a) = agent {
                let eff = service::effective_all(app, &access.project, &a)?;
                v["rules"] = json!(eff.iter().map(|e| e.describe()).collect::<Vec<_>>());
            }
            Ok(v)
        })
        .await?;
    Ok(Json(out))
}

#[derive(Debug, Deserialize)]
struct TaskReposBody {
    repos: Vec<Value>,
}

/// `{"name": "api", "access": "write"}` or `"api:write"` (`"api"` alone is read).
fn parse_wanted(v: &Value) -> Result<(String, String), ApiError> {
    match v {
        Value::String(s) => {
            let (n, a) = s.split_once(':').unwrap_or((s.as_str(), "read"));
            Ok((n.trim().to_string(), a.trim().to_string()))
        }
        Value::Object(o) => Ok((
            o.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
            o.get("access").and_then(Value::as_str).unwrap_or("read").to_string(),
        )),
        _ => Err(ApiError::bad("a repository is {\"name\": \"api\", \"access\": \"write\"} or \"api:write\"")),
    }
}

async fn set_task_repos(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path(id): Path<String>,
    Json(b): Json<TaskReposBody>,
) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    if access.agent && access.actor.role != Role::Orchestrator {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "only people and the orchestrator name a task's repositories"));
    }
    let wanted = b.repos.iter().map(parse_wanted).collect::<Result<Vec<_>, _>>()?;
    let out = app
        .blocking(move |app| {
            let task = app.with_tracker(&access.project, |t| t.normalize_id(&id))?;
            let rows = app.with_server(|db| db.set_task_repos(&access.project, &task, &wanted))?;
            Ok(json!({ "task": task, "repos": rows }))
        })
        .await?;
    Ok(Json(out))
}
