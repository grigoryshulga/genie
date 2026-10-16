//! The genie MCP server: the catalog of operations ([`crate::ops`]) as MCP
//! tools — one per group, `genie_task` with its `action` — for a person's own
//! agent (Claude Code, pi, Codex…) and for agents genie runs.
//!
//! `POST /mcp` speaks MCP Streamable HTTP with JSON answers. The token says who
//! calls: a person sees what people do (admins also the administration), an
//! agent the commands of its prompt — those its class and role allow. Every
//! call runs the operation through the server's API with that token, so the API
//! decides as it does for the command line and the web.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use genie_core::server_db::ProjectRole;
use genie_core::{Capability, Role};
use serde_json::{Value, json};

use super::ctx::{Ctx, Who};
use super::{ApiError, ApiResult};
use crate::mcp_gateway::{PROTOCOL, Reply, rpc_error};
use crate::ops::{self, AgentKind, Auth, Cx, Entry, InProcess, Need};
use crate::state::App;

/// Messages in one batch request (answered one after another).
const MAX_BATCH: usize = 50;
/// Protocol versions the server speaks; a client asking for another gets the newest.
const VERSIONS: &[&str] = &[PROTOCOL, "2025-03-26", "2024-11-05"];

/// No server-initiated stream and no session to end: requests are answered in their response.
pub async fn no_stream() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")], "the genie MCP server answers POST requests only").into_response()
}

/// Who calls, as the catalog sees it.
enum Caller {
    Person {
        login: String,
        admin: bool,
        auth: Auth,
        project: Option<String>,
    },
    Agent {
        name: String,
        project: String,
        kind: AgentKind,
        caps: Vec<Capability>,
        token: String,
        team: Option<String>,
        task: Option<String>,
    },
}

impl Caller {
    fn sees(&self, e: &Entry) -> bool {
        match self {
            Caller::Person { admin, .. } => match e.need {
                Need::Read | Need::Write | Need::Orchestrator | Need::Person => true,
                Need::Admin => *admin,
                Need::Agent | Need::Member | Need::Job => false,
            },
            // The operations of its prompt's command table: what its work needs, in less context.
            Caller::Agent { kind, caps, .. } => e.listed_for(*kind, &|c| caps.contains(&c)),
        }
    }

    fn note(&self, e: &Entry) -> Option<String> {
        match self {
            Caller::Person { .. } => None,
            Caller::Agent { kind, caps, .. } => e.note_for(*kind, &|c| caps.contains(&c)),
        }
    }

    /// Where an operation runs for this caller: the server's router, with the caller's token.
    fn cx(&self, app: &Arc<App>, project: Option<String>) -> Cx {
        let (auth, project, task, team) = match self {
            Caller::Person { auth, project: default, .. } => (auth.clone(), project.or_else(|| default.clone()), None, None),
            Caller::Agent { token, project, task, team, .. } => {
                (Auth::Bearer(token.clone()), Some(project.clone()), task.clone(), team.clone())
            }
        };
        let api = InProcess::new(crate::http::router(app.clone()), app.cfg.port, auth, project.clone());
        Cx { api: Box::new(api), project, task, team, local: false }
    }
}

pub async fn endpoint(State(app): State<Arc<App>>, ctx: Ctx, headers: HeaderMap, body: Bytes) -> Response {
    serve(&app, &ctx, &headers, &body).await.unwrap_or_else(|e| e.into_response())
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")).map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

async fn caller(app: &Arc<App>, ctx: &Ctx, headers: &HeaderMap) -> ApiResult<Caller> {
    match &ctx.who {
        Who::Anonymous => Err(ApiError::unauthorized()),
        Who::Agent { project, role, role_id, name, team, job } => {
            let token = bearer(headers).ok_or_else(ApiError::unauthorized)?;
            let kind = match (role, job) {
                (Role::Orchestrator, _) => AgentKind::Orchestrator,
                (_, Some(_)) => AgentKind::Job,
                _ => AgentKind::Member,
            };
            let caps = role_id
                .as_deref()
                .and_then(|id| app.agents().roles.get(id).map(|r| r.capabilities.clone()))
                .unwrap_or_else(|| genie_core::class_capabilities(*role));
            // A team works on the task it is named after; a job on its own.
            let task = match job {
                Some(j) => {
                    let j = *j;
                    app.blocking(move |app| app.with_server(|db| db.job(j))).await?.task
                }
                None => genie_core::management::managed_task(name).map(str::to_string).or_else(|| team.clone()),
            };
            Ok(Caller::Agent { name: name.clone(), project: project.clone(), kind, caps, token, team: team.clone(), task })
        }
        Who::User { user, local } => {
            let auth = match (bearer(headers), local) {
                (Some(token), _) => Auth::Bearer(token),
                // The implicit owner of a server without users, on its machine.
                (None, true) => Auth::Operator,
                (None, false) => {
                    return Err(ApiError::new(
                        StatusCode::UNAUTHORIZED,
                        "the genie MCP server takes a token: `genie user token <login>`, sent as `Authorization: Bearer <token>`",
                    ));
                }
            };
            let u = user.clone();
            let admin = user.is_admin
                || app
                    .blocking(move |app| {
                        app.with_server(|db| {
                            for p in db.projects()? {
                                if matches!(db.project_role(&p.slug, &u)?, Some(ProjectRole::Admin | ProjectRole::Owner)) {
                                    return Ok(true);
                                }
                            }
                            Ok(false)
                        })
                    })
                    .await?;
            Ok(Caller::Person { login: user.login.clone(), admin, auth, project: ctx.project_hint.clone() })
        }
    }
}

async fn serve(app: &Arc<App>, ctx: &Ctx, headers: &HeaderMap, body: &[u8]) -> ApiResult<Response> {
    let who = caller(app, ctx, headers).await?;
    let Ok(msg) = serde_json::from_slice::<Value>(body) else {
        let e = json!({ "jsonrpc": "2.0", "id": null, "error": rpc_error(-32700, "the request is not JSON") });
        return Ok((StatusCode::BAD_REQUEST, Json(e)).into_response());
    };
    let (batch, messages) = match msg {
        Value::Array(a) if a.len() > MAX_BATCH => {
            let e = json!({ "jsonrpc": "2.0", "id": null, "error": rpc_error(-32600, format!("at most {MAX_BATCH} messages in a batch")) });
            return Ok((StatusCode::BAD_REQUEST, Json(e)).into_response());
        }
        Value::Array(a) => (true, a),
        m => (false, vec![m]),
    };
    let mut replies = Vec::new();
    for m in messages {
        // Notifications and the client's answers need no reply.
        let (Some(method), Some(id)) = (m.get("method").and_then(Value::as_str), m.get("id")) else { continue };
        let params = m.get("params").cloned().unwrap_or_else(|| json!({}));
        replies.push(match handle(app, &who, method, params).await {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
        });
    }
    Ok(match (batch, replies.len()) {
        (_, 0) => StatusCode::ACCEPTED.into_response(),
        (false, _) => Json(replies.swap_remove(0)).into_response(),
        (true, _) => Json(Value::Array(replies)).into_response(),
    })
}

async fn handle(app: &Arc<App>, who: &Caller, method: &str, params: Value) -> Reply {
    match method {
        "ping" => Ok(json!({})),
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or(PROTOCOL);
            let version = VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(PROTOCOL);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "genie", "title": "genie", "version": env!("CARGO_PKG_VERSION") },
                "instructions": instructions(who),
            }))
        }
        "tools/list" => {
            let extra = match who {
                Caller::Person { .. } => {
                    vec![(
                        "project",
                        json!({ "type": "string", "description": "The project to act in, one of yours (default: the connection's, else your first)." }),
                    )]
                }
                Caller::Agent { .. } => Vec::new(),
            };
            Ok(json!({ "tools": ops::tools::tools(&|e| who.sees(e), &|e| who.note(e), &extra) }))
        }
        "tools/call" => call(app, who, params).await,
        _ => Err(rpc_error(-32601, format!("{method} is not offered by the genie MCP server"))),
    }
}

fn instructions(who: &Caller) -> String {
    let me = match who {
        Caller::Person { login, admin, .. } => {
            format!(
                "You act as the person {login}{}; pass `project` to act in another of their projects.",
                if *admin { " (an admin)" } else { "" }
            )
        }
        Caller::Agent { name, project, .. } => format!("You act as the agent {name} of project {project}."),
    };
    format!(
        "genie: tasks, agent teams, mail and the knowledge base of a project. Each tool is a group of genie's operations; `action` picks one, and its arguments are listed in the tool's description. {me} The server checks every right; answers are plain text."
    )
}

fn text(t: impl Into<String>, error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": t.into() }], "isError": error })
}

async fn call(app: &Arc<App>, who: &Caller, params: Value) -> Reply {
    let tool = params["name"].as_str().unwrap_or_default();
    let group_known = ops::GROUPS.iter().any(|(g, _)| ops::tools::name(g) == tool);
    if !group_known {
        return Err(rpc_error(-32602, format!("unknown tool {tool}")));
    }
    let mut args = match params.get("arguments").cloned().unwrap_or_else(|| json!({})) {
        Value::Object(o) => o,
        _ => return Ok(text("arguments must be an object", true)),
    };
    let action = args.remove("action").and_then(|a| a.as_str().map(str::to_string)).unwrap_or_default();
    let Some(entry) = ops::tools::find(tool, &action).filter(|e| who.sees(e)) else {
        let yours: Vec<&str> = ops::catalog().iter().filter(|e| ops::tools::name(e.group) == tool && who.sees(e)).map(|e| e.name).collect();
        return Ok(text(format!("{tool} has no action `{action}` for you; yours: {}", yours.join(", ")), true));
    };
    let project = match who {
        Caller::Person { .. } => args.remove("project").and_then(|p| p.as_str().map(str::to_string)).filter(|p| !p.is_empty()),
        Caller::Agent { .. } => None,
    };
    let cx = who.cx(app, project);
    Ok(match entry.run_json(Value::Object(args), &cx).await {
        Ok(out) => match out.failed {
            Some(why) => text(format!("{}\n\n{why}", out.text), true),
            None => text(out.text, false),
        },
        Err(e) => text(e, true),
    })
}
