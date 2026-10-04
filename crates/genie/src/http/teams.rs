//! Teams, mail and agent turns. The team JSON matches the TypeScript server so
//! the SPA's team screen works unchanged.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use genie_core::team::{self, ORCHESTRATOR, SendMail};
use genie_core::work::NewJob;
use genie_core::{Capability, Role};
use serde::Deserialize;
use serde_json::{Value, json};

use super::ctx::{Access, Ctx};
use super::tasks::changed;
use super::{ApiError, ApiResult};
use crate::agent_config::TeamSpec;
use crate::config::MemberSpec;
use crate::runtime::{self, SpawnRequest};
use crate::state::App;

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/teams", get(list).post(spawn))
        .route("/teams/{id}", get(show).delete(remove))
        .route("/teams/{id}/stop", post(stop))
        .route("/teams/{id}/mail", post(send))
        .route("/teams/{id}/members", post(add_member))
        .route("/teams/{id}/members/{member}", delete(remove_member).patch(set_model))
        .route("/teams/{id}/members/{member}/restart", post(restart_member))
        .route("/models", get(models))
        .route("/agent/status", post(member_status))
        .route("/agent/output", post(job_output))
        .route("/turns", get(turns))
        .route("/jobs", get(jobs).post(create_job))
        .route("/jobs/{id}", get(job))
}

fn view(t: &genie_core::Tracker, id: &str) -> genie_core::Result<Value> {
    let team = t.bus().get(id)?;
    let task = t.get(&team.task).ok().map(|x| json!({ "id": x.id, "title": x.title, "status": x.status }));
    let mut pending = serde_json::Map::new();
    for m in &team.members {
        pending.insert(m.name.clone(), json!(t.bus().pending_count(&team.id, &m.name)?));
    }
    let mut v = serde_json::to_value(&team)?;
    v["taskInfo"] = task.unwrap_or(Value::Null);
    v["pending"] = Value::Object(pending);
    Ok(v)
}

/// Live sessions of a team's members (`sessions: {name: live}`), for the team screen.
fn attach_sessions(app: &App, project: &str, v: &mut Value) {
    let Some(team) = v["id"].as_str().map(str::to_string) else { return };
    let mut sessions = serde_json::Map::new();
    for m in v["members"].as_array().cloned().unwrap_or_default() {
        let Some(name) = m["name"].as_str() else { continue };
        let key = runtime::AgentKey::Member { project: project.to_string(), team: team.clone(), member: name.to_string() };
        if let Some(s) = app.sessions.get(&key) {
            sessions.insert(name.to_string(), json!(s.live()));
        }
    }
    v["sessions"] = Value::Object(sessions);
}

/// Whether the team's template changed since the team took its snapshot (`templateChanged`).
fn attach_template_state(app: &App, v: &mut Value) {
    let Some(spec) = TeamSpec::from_value(&v["spec"]) else { return };
    let (Some(id), Some(hash)) = (&spec.template, &spec.template_hash) else { return };
    let changed = app.agents().teams.get(id).is_none_or(|t| &crate::agent_config::template_hash(t) != hash);
    v["templateChanged"] = json!(changed);
}

/// Agents may touch only their own team; the orchestrator and people any team.
fn own_team(access: &Access, team: &str) -> ApiResult<()> {
    match (&access.agent_team, access.agent) {
        (_, false) => Ok(()),
        (_, true) if access.actor.role == Role::Orchestrator => Ok(()),
        (Some(t), true) if t == team => Ok(()),
        _ => Err(ApiError::new(StatusCode::FORBIDDEN, format!("team {team} is not your team"))),
    }
}

#[derive(Deserialize, Default)]
struct ListQuery {
    all: Option<String>,
}

async fn list(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<ListQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let all = q.all.as_deref() == Some("1");
    let slug = access.project.clone();
    let out = app
        .blocking(move |app| {
            app.with_tracker(&slug, |t| {
                let teams = t.bus().list(all)?;
                teams.iter().map(|x| view(t, &x.id)).collect::<genie_core::Result<Vec<_>>>()
            })
        })
        .await?;
    let mut out = out;
    for v in &mut out {
        attach_sessions(&app, &access.project, v);
    }
    Ok(Json(json!(out)))
}

async fn show(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let slug = access.project.clone();
    let (mut v, turns) = app
        .blocking(move |app| {
            let v = app.with_tracker(&slug, |t| {
                let mut v = view(t, &id)?;
                v["mail"] = serde_json::to_value(t.bus().history(&id, 200)?)?;
                v["log"] = serde_json::to_value(t.bus().read_log(&id, 200)?)?;
                Ok(v)
            })?;
            let turns = app.with_server(|db| {
                let all = db.turns(&slug, None, 400)?;
                Ok(all.into_iter().filter(|x| x.team.as_deref() == Some(id.as_str())).collect::<Vec<_>>())
            })?;
            Ok((v, turns))
        })
        .await?;
    v["turns"] = json!(turns);
    attach_sessions(&app, &access.project, &mut v);
    attach_template_state(&app, &mut v);
    Ok(Json(v))
}

#[derive(Deserialize)]
struct SpawnBody {
    task: String,
    template: Option<String>,
    #[serde(default)]
    members: Vec<MemberSpec>,
    /// Models for the template's members, by member key.
    #[serde(default)]
    models: std::collections::BTreeMap<String, String>,
    note: Option<String>,
}

async fn spawn(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<SpawnBody>) -> ApiResult<impl IntoResponse> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    if access.agent && access.actor.role != Role::Orchestrator {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "only the orchestrator (or a person) assembles teams"));
    }
    let slug = access.project.clone();
    let req = SpawnRequest {
        task: b.task,
        template: b.template,
        members: b.members,
        models: b.models,
        note: b.note,
        by: access.actor.clone(),
        initiator: None,
    };
    let team = app.blocking(move |app| runtime::spawn_team(app, &slug, req)).await?;
    changed(&app);
    Ok((StatusCode::CREATED, Json(json!(team))))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct StopBody {
    #[serde(default)]
    remove_worktree: bool,
}

async fn stop(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, body: Option<Json<StopBody>>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    if access.agent && access.actor.role != Role::Orchestrator {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "only the orchestrator (or a person) stops teams"));
    }
    let reason = if access.is_human() { "owner" } else { "orchestrator" };
    let (slug, by) = (access.project.clone(), access.actor.name.clone());
    let remove = body.map(|Json(b)| b.remove_worktree).unwrap_or(false);
    let report = app
        .blocking(move |app| {
            let mut r = runtime::stop_team(app, &slug, &id, reason, &by)?;
            if remove {
                r.push(crate::runtime::remove_worktree(app, &slug, &id));
            }
            Ok(r)
        })
        .await?;
    changed(&app);
    Ok(Json(json!({ "ok": true, "report": report })))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RemoveQuery {
    remove_worktree: Option<String>,
}

async fn remove(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Query(q): Query<RemoveQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.admin()?;
    let (slug, by) = (access.project.clone(), access.actor.name.clone());
    let remove = q.remove_worktree.as_deref() == Some("1");
    let report = app
        .blocking(move |app| {
            let active = app.with_tracker(&slug, |t| Ok(t.bus().get(&id)?.state == "active"))?;
            let mut r = if active { runtime::stop_team(app, &slug, &id, "owner", &by)? } else { Vec::new() };
            if remove {
                r.push(crate::runtime::remove_worktree(app, &slug, &id));
            }
            app.with_tracker(&slug, |t| t.bus().delete(&id))?;
            r.push(format!("team {id} deleted"));
            Ok(r)
        })
        .await?;
    changed(&app);
    Ok(Json(json!({ "ok": true, "report": report })))
}

#[derive(Deserialize)]
struct MailBody {
    to: String,
    text: String,
    level: Option<String>,
    intent: Option<String>,
    #[serde(default)]
    urgent: bool,
    topic: Option<String>,
}

async fn send(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Json(b): Json<MailBody>) -> ApiResult<impl IntoResponse> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    own_team(&access, &id)?;
    access.can(if b.to == ORCHESTRATOR { Capability::MailOrchestrator } else { Capability::MailTeam })?;
    let (slug, from, role) = (access.project.clone(), access.actor.name.clone(), access.actor.role);
    let level = b.level.or_else(|| b.urgent.then(|| "high".to_string()));
    if level.as_deref() == Some("interrupt") && !matches!(role, Role::Orchestrator | Role::Human) {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "only the orchestrator and people interrupt an agent; use --level high"));
    }
    let member = access.agent && role != Role::Orchestrator;
    let mail = app
        .blocking(move |app| {
            app.with_tracker(&slug, |t| {
                if member {
                    flow_route(&t.bus().get(&id)?, &from, &b.to, b.intent.as_deref())?;
                }
                t.bus().send(SendMail {
                    team: &id,
                    from: if role == Role::Orchestrator { ORCHESTRATOR } else { &from },
                    from_role: role.as_str(),
                    to: &b.to,
                    text: &b.text,
                    level: level.as_deref(),
                    intent: b.intent.as_deref(),
                    kind: if role == Role::Human { "owner" } else { "message" },
                    topic: b.topic.as_deref(),
                    ..Default::default()
                })
            })
        })
        .await?;
    app.wake_runtime.notify_one();
    Ok((StatusCode::CREATED, Json(json!(mail))))
}

/// A member of a `mail: flow` team writes only along the template's route.
pub(super) fn flow_route(team: &team::Team, from: &str, to: &str, intent: Option<&str>) -> Result<(), genie_core::GenieError> {
    let why = team.spec.as_ref().and_then(TeamSpec::from_value).and_then(|s| s.flow_refusal(from, to, intent));
    match why {
        Some(why) => Err(genie_core::GenieError::Denied(format!("genie: {why}"))),
        None => Ok(()),
    }
}

async fn add_member(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path(id): Path<String>,
    Json(spec): Json<MemberSpec>,
) -> ApiResult<impl IntoResponse> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    if access.agent && access.actor.role != Role::Orchestrator {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "only the orchestrator (or a person) changes a team"));
    }
    let (slug, by) = (access.project.clone(), access.actor.name.clone());
    let model = spec.model.clone();
    let added = app.blocking(move |app| runtime::add_member(app, &slug, &id, spec, &by)).await?;
    let added = json!([{ "name": added.name, "role": added.role, "key": added.key, "model": model }]);
    changed(&app);
    Ok((StatusCode::CREATED, Json(added)))
}

async fn remove_member(State(app): State<Arc<App>>, ctx: Ctx, Path((id, member)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    if access.agent && access.actor.role != Role::Orchestrator {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "only the orchestrator (or a person) changes a team"));
    }
    let (slug, by) = (access.project.clone(), access.actor.name.clone());
    app.blocking(move |app| {
        app.with_tracker(&slug, |t| {
            let team = t.bus().get(&id)?;
            let role = team.members.iter().find(|m| m.name == member).map(|m| m.role.clone()).unwrap_or_default();
            t.bus().remove_member(&id, &member)?;
            let note = format!("{} — {role} left the team (removed by {by}).", team::display_name(&member));
            for other in team.members.iter().filter(|x| x.name != member) {
                t.bus().send(SendMail {
                    team: &id,
                    from: ORCHESTRATOR,
                    from_role: "orchestrator",
                    to: &other.name,
                    text: &note,
                    level: Some("low"),
                    intent: Some("fyi"),
                    kind: "system",
                    ..Default::default()
                })?;
            }
            Ok(())
        })
    })
    .await?;
    changed(&app);
    Ok(Json(json!({ "ok": true })))
}

async fn restart_member(State(app): State<Arc<App>>, ctx: Ctx, Path((id, member)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    if access.agent && access.actor.role != Role::Orchestrator {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "only the orchestrator (or a person) restarts members"));
    }
    let slug = access.project.clone();
    app.blocking(move |app| runtime::restart_member(app, &slug, &id, &member)).await?;
    Ok(Json(json!({ "ok": true })))
}

/// Thinking levels a member can be given (pi's, plus `off`).
pub const THINKING: &[&str] = &["off", "minimal", "low", "medium", "high", "xhigh", "max"];

#[derive(Deserialize, Default)]
struct ModelBody {
    /// `provider/model`; empty or absent: the role's model.
    model: Option<String>,
    /// A thinking level; empty or absent: the role's.
    thinking: Option<String>,
}

/// Give one member its own model (and thinking level), or take it back to its
/// role's. The member keeps its conversation: a busy one switches when its
/// current step ends, an idle one on its next mail.
async fn set_model(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path((id, member)): Path<(String, String)>,
    Json(b): Json<ModelBody>,
) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    if access.agent && access.actor.role != Role::Orchestrator {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "only the orchestrator (or a person) changes a member's model"));
    }
    let clean = |v: Option<String>| v.map(|x| x.trim().to_string()).filter(|x| !x.is_empty());
    let (model, thinking) = (clean(b.model), clean(b.thinking));
    if let Some(m) = &model
        && m.chars().any(char::is_whitespace)
    {
        return Err(ApiError::bad(format!("invalid model \"{m}\"; expected provider/model")));
    }
    if let Some(t) = &thinking
        && !THINKING.contains(&t.as_str())
    {
        return Err(ApiError::bad(format!("invalid thinking level \"{t}\"; expected one of {}", THINKING.join(", "))));
    }
    let (slug, by, t2, m2, model2, thinking2) =
        (access.project.clone(), access.actor.name.clone(), id.clone(), member.clone(), model.clone(), thinking.clone());
    app.blocking(move |app| app.with_tracker(&slug, |t| t.bus().set_member_model(&t2, &m2, model2.as_deref(), thinking2.as_deref(), &by)))
        .await?;
    let key = runtime::AgentKey::Member { project: access.project.clone(), team: id, member };
    crate::sessions::reload(&app, &key);
    app.wake_runtime.notify_one();
    changed(&app);
    Ok(Json(json!({ "ok": true, "agent": key.label(), "model": model, "thinking": thinking })))
}

/// `provider/model:high` names a thinking level too; model ids may hold a `:` of their own.
fn without_thinking(model: &str) -> &str {
    match model.rsplit_once(':') {
        Some((id, level)) if THINKING.contains(&level) => id,
        _ => model,
    }
}

/// pi's catalogue, read at most every few minutes (it starts a process).
static PI_MODELS: std::sync::Mutex<Option<(std::time::Instant, Option<BTreeSet<String>>)>> = std::sync::Mutex::new(None);
const PI_MODELS_TTL: std::time::Duration = std::time::Duration::from_secs(300);

#[derive(Deserialize, Default)]
struct ModelsQuery {
    refresh: Option<String>,
}

/// The models an agent can be given: pi's catalogue and the models the agent
/// configuration already names (`listed: false` when pi does not list them).
async fn models(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<ModelsQuery>) -> ApiResult<Json<Value>> {
    ctx.access(&app, None).await?;
    let cached = PI_MODELS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .filter(|(at, _)| q.refresh.as_deref() != Some("1") && at.elapsed() < PI_MODELS_TTL)
        .map(|(_, list)| list);
    let catalogue = match cached {
        Some(list) => list,
        None => {
            let list = app.blocking(|app| Ok(crate::doctor::pi_models(&app.cfg))).await?;
            *PI_MODELS.lock().unwrap_or_else(|e| e.into_inner()) = Some((std::time::Instant::now(), list.clone()));
            list
        }
    };
    let agents = app.agents();
    let mut named: BTreeSet<String> = app.cfg.role_models.values().filter_map(|m| m.model.clone()).collect();
    named.extend(agents.roles.values().filter_map(|r| r.model.clone()));
    named.extend(agents.teams.values().flat_map(|t| t.members.iter().filter_map(|m| m.model.clone())));
    let listed = catalogue.clone().unwrap_or_default();
    let all: BTreeSet<String> = listed.iter().cloned().chain(named.into_iter().map(|m| without_thinking(&m).to_string())).collect();
    let models: Vec<Value> = all
        .iter()
        .map(|id| {
            let (provider, name) = id.split_once('/').unwrap_or(("", id));
            json!({ "id": id, "provider": provider, "name": name, "listed": listed.contains(id) })
        })
        .collect();
    Ok(Json(json!({ "catalogue": catalogue.is_some(), "models": models, "thinking": THINKING })))
}

#[derive(Deserialize)]
struct StatusBody {
    text: String,
}

async fn member_status(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<StatusBody>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let Some(team) = access.agent_team.clone() else {
        return Err(ApiError::bad("only team members have a status line"));
    };
    let (slug, name) = (access.project.clone(), access.actor.name.clone());
    app.blocking(move |app| app.with_tracker(&slug, |t| t.bus().set_member_status(&team, &name, &b.text))).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn job_output(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<Value>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let Some(job) = access.agent_job else {
        return Err(ApiError::bad("only one-shot jobs report an output"));
    };
    let output = b.get("output").cloned().unwrap_or(b);
    if !output.is_object() {
        return Err(ApiError::bad("the output must be a JSON object"));
    }
    app.blocking(move |app| app.with_server(|db| db.set_job_output(job, &output))).await?;
    Ok(Json(json!({ "ok": true, "job": job })))
}

#[derive(Deserialize, Default)]
struct TurnsQuery {
    agent: Option<String>,
    limit: Option<i64>,
}

async fn turns(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<TurnsQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let slug = access.project.clone();
    let out = app.blocking(move |app| app.with_server(|db| db.turns(&slug, q.agent.as_deref(), q.limit.unwrap_or(100).min(500)))).await?;
    Ok(Json(json!(out)))
}

async fn jobs(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let slug = access.project.clone();
    let out = app.blocking(move |app| app.with_server(|db| db.jobs(&slug, 200))).await?;
    Ok(Json(json!(out)))
}

async fn job(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let job = app.blocking(move |app| app.with_server(|db| db.job(id))).await?;
    if job.project != access.project {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "job not found"));
    }
    Ok(Json(json!(job)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobBody {
    role: String,
    goal: String,
    task: Option<String>,
    model: Option<String>,
    #[serde(default)]
    inputs: Value,
    output_schema: Option<Value>,
    workspace: Option<String>,
}

async fn create_job(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<JobBody>) -> ApiResult<impl IntoResponse> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    if access.agent && access.actor.role != Role::Orchestrator {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "only the orchestrator (or a person) starts jobs"));
    }
    let slug = access.project.clone();
    let job = app
        .blocking(move |app| {
            if app.agents().role_for(&slug, &b.role)?.class == Role::Orchestrator {
                return Err(genie_core::GenieError::invalid("the orchestrator does not run one-shot jobs").into());
            }
            let initiator = crate::llm_key::initiator_of(app, &slug, &access.actor, b.task.as_deref());
            app.with_server(|db| {
                db.create_job(NewJob {
                    project: slug,
                    task: b.task,
                    run_step: None,
                    role: b.role,
                    model: b.model,
                    goal: b.goal,
                    inputs: b.inputs,
                    output_schema: b.output_schema,
                    workspace: b.workspace.unwrap_or_else(|| "none".into()),
                    initiator,
                })
            })
        })
        .await?;
    app.wake_runtime.notify_one();
    Ok((StatusCode::CREATED, Json(json!(job))))
}
