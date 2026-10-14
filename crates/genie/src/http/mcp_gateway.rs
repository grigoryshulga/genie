//! The MCP gateway over HTTP (see `crate::mcp_gateway`): the agents' MCP
//! endpoint, the connection check for administrators and the log of calls.
//!
//! `POST /api/mcp-gateway/<server>` speaks MCP Streamable HTTP with JSON
//! answers. The agent's token says who calls; the role's grant is looked up in
//! the configuration at every request, so a revoked connection or tool stops
//! working at once. A grant limited to some tools keeps the rest of the
//! server's list, and its resources and prompts, out of reach.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use genie_core::Role;
use genie_core::events;
use serde::Deserialize;
use serde_json::{Value, json};

use super::ctx::{Ctx, Who};
use super::{ApiError, ApiResult};
use crate::mcp_gateway::{self, Reply, Upstream, rpc_error};
use crate::state::App;

/// How long a check of a connection may take (a first `npx` download included).
const CHECK_TIMEOUT: Duration = Duration::from_secs(90);
/// Messages in one batch request (answered one after another).
const MAX_BATCH: usize = 50;

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/mcp-gateway/{server}", post(endpoint).get(no_stream).delete(no_stream))
        .route("/mcp/{server}/check", post(check))
        .route("/mcp/calls", get(calls))
}

/// No server-initiated stream and no session to end: requests are answered in their response.
async fn no_stream() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")], "the genie MCP gateway answers POST requests only").into_response()
}

/// The agent calling the gateway.
struct Caller {
    project: String,
    name: String,
    class: Role,
    /// Its configured role.
    role: String,
    team: Option<String>,
    job: Option<i64>,
}

/// What the caller may use of one connection.
struct Grant {
    server: String,
    /// Tool patterns (`None`: the whole connection).
    tools: Option<Vec<String>>,
    /// Tools the administrator closed for everyone (`hiddenTools`), grants notwithstanding.
    hidden: Vec<String>,
    /// The resolved entry, run in the agent's working directory.
    config: Value,
    /// The agent's connection (`<agent>/<server>`).
    key: String,
}

async fn endpoint(State(app): State<Arc<App>>, ctx: Ctx, Path(server): Path<String>, body: Bytes) -> Response {
    serve(&app, &ctx, &server, &body).await.unwrap_or_else(|e| e.into_response())
}

async fn serve(app: &Arc<App>, ctx: &Ctx, server: &str, body: &[u8]) -> ApiResult<Response> {
    let Who::Agent { project, role, role_id, name, team, job } = &ctx.who else {
        return Err(match ctx.who {
            Who::Anonymous => ApiError::unauthorized(),
            _ => {
                ApiError::new(StatusCode::FORBIDDEN, "the MCP gateway serves agents; check a connection with POST /api/mcp/<server>/check")
            }
        });
    };
    let agents = app.agents();
    let def = role_id
        .as_deref()
        .and_then(|id| agents.roles.get(id))
        .ok_or_else(|| ApiError::new(StatusCode::FORBIDDEN, "this agent acts in no configured role, so it has no MCP connections"))?;
    let (srv, tools) = agents.mcp_for(project, def).into_iter().find(|(s, _)| s.id == server).ok_or_else(|| {
        ApiError::new(StatusCode::FORBIDDEN, format!("role {} has no access to the MCP connection {server} in project {project}", def.id))
    })?;
    let agent = mcp_gateway::agent_key(project, team.as_deref(), name);
    // The values an administrator keeps in the server's database come before its environment.
    let env = app.blocking(|app| app.with_server(|db| db.mcp_env())).await.unwrap_or_default();
    let mut config = srv.resolved_with(&env);
    if config.get("command").is_some()
        && config.get("cwd").is_none()
        && let Some(o) = config.as_object_mut()
    {
        let cwd = app.mcp.place_of(&agent).unwrap_or_else(|| crate::runtime::project_workspace_dir(app, project, name));
        o.insert("cwd".into(), json!(cwd));
    }
    let grant = Grant { server: server.to_string(), tools, hidden: srv.hidden.clone(), config, key: format!("{agent}/{server}") };
    let caller = Caller { project: project.clone(), name: name.clone(), class: *role, role: def.id.clone(), team: team.clone(), job: *job };

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
        // Notifications and the client's answers need no reply and reach no server.
        let (Some(method), Some(id)) = (m.get("method").and_then(Value::as_str), m.get("id")) else { continue };
        let params = m.get("params").cloned().unwrap_or_else(|| json!({}));
        replies.push(match handle(app, &caller, &grant, method, params).await {
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

async fn handle(app: &Arc<App>, who: &Caller, g: &Grant, method: &str, params: Value) -> Reply {
    let whole = g.tools.is_none();
    match method {
        "ping" => Ok(json!({})),
        "initialize" => Ok(introduce(&connection(app, g).await?.init, whole)),
        "tools/list" => {
            let mut r = connection(app, g).await?.request(method, params).await?;
            if let Some(list) = r.get_mut("tools").and_then(Value::as_array_mut) {
                // The whole listing refreshes the server's cache for the web's tool
                // pickers; what the agent sees also leaves out the closed tools.
                let full: Vec<Value> = list.iter().filter(|t| t["name"].is_string()).cloned().collect();
                cache_tools(app, &g.server, full);
                list.retain(|t| {
                    t["name"].as_str().is_some_and(|n| !mcp_gateway::hidden(&g.hidden, n) && mcp_gateway::allowed(g.tools.as_deref(), n))
                });
            }
            Ok(r)
        }
        "tools/call" => call_tool(app, who, g, params).await,
        "resources/list" | "resources/templates/list" | "resources/read" | "prompts/list" | "prompts/get" | "completion/complete" => {
            if !whole {
                return Err(rpc_error(-32601, format!("role {} may use only some tools of {}, not its {method}", who.role, g.server)));
            }
            connection(app, g).await?.request(method, params).await
        }
        // Log messages would come as notifications, which the gateway does not pass on.
        "logging/setLevel" => Ok(json!({})),
        _ => Err(rpc_error(-32601, format!("{method} is not offered through the genie gateway"))),
    }
}

async fn connection(app: &Arc<App>, g: &Grant) -> Result<Arc<Upstream>, Value> {
    app.mcp.get(&g.key, &g.config).await.map_err(|e| {
        eprintln!("genie mcp: {}: {e}", g.key);
        rpc_error(-32000, format!("cannot connect to the MCP server {}: {e}", g.server))
    })
}

/// The server's `initialize` answer as the agent gets it: without what the
/// gateway does not pass on (notifications, subscriptions) and, for a grant of
/// some tools, without resources and prompts.
fn introduce(init: &Value, whole: bool) -> Value {
    let mut out = init.clone();
    if let Some(caps) = out.get_mut("capabilities").and_then(Value::as_object_mut) {
        caps.remove("logging");
        if !whole {
            caps.retain(|k, _| k == "tools");
        }
        for c in caps.values_mut() {
            if let Some(o) = c.as_object_mut() {
                o.remove("listChanged");
                o.remove("subscribe");
            }
        }
    }
    out
}

async fn call_tool(app: &Arc<App>, who: &Caller, g: &Grant, params: Value) -> Reply {
    let tool = params["name"].as_str().unwrap_or_default().to_string();
    let started = Instant::now();
    let (granted, closed) = (mcp_gateway::allowed(g.tools.as_deref(), &tool), mcp_gateway::hidden(&g.hidden, &tool));
    let reply = if closed {
        Err(rpc_error(-32602, format!("the administrator closed the tool {tool} of {}", g.server)))
    } else if granted {
        match connection(app, g).await {
            Ok(up) => up.request("tools/call", params.clone()).await,
            Err(e) => Err(e),
        }
    } else {
        Err(rpc_error(-32602, format!("the tool {tool} of {} is not granted to role {}", g.server, who.role)))
    };
    let (ok, error) = match &reply {
        Ok(r) if r["isError"].as_bool() == Some(true) => (false, Some(first_text(r))),
        Ok(_) => (true, None),
        Err(e) => (false, Some(e["message"].as_str().unwrap_or("error").to_string())),
    };
    // What the call moved both ways, for the journal and the usage statistics.
    let (bytes_in, bytes_out) = (
        params.get("arguments").map(Value::to_string).unwrap_or_default().len() as u64,
        match &reply {
            Ok(r) => r.to_string().len(),
            Err(e) => e.to_string().len(),
        } as u64,
    );
    let mut payload = json!({
        "server": g.server,
        "tool": tool,
        "ok": ok,
        "ms": started.elapsed().as_millis() as u64,
        "role": who.role,
        "bytesIn": bytes_in,
        "bytesOut": bytes_out,
        "args": clip(&params.get("arguments").map(Value::to_string).unwrap_or_default(), 500),
    });
    if let Some(e) = error {
        payload["error"] = json!(clip(&e, 500));
    }
    if closed {
        payload["closed"] = json!(true);
    } else if !granted {
        payload["refused"] = json!(true);
    }
    if let Some(j) = who.job {
        payload["job"] = json!(j);
    }
    let (project, subject, actor, class) = (who.project.clone(), who.team.clone(), who.name.clone(), who.class);
    let (server, tool_for_log) = (g.server.clone(), tool.clone());
    let ms = started.elapsed().as_millis() as u64;
    let reached_server = granted && !closed;
    let written = app
        .blocking(move |app| {
            app.with_tracker(&project, |t| {
                t.append_event(events::MCP_CALLED, subject.as_deref(), &actor, class.as_str(), payload).map(|_| ())
            })?;
            // A refused or closed call reached no server: the journal has it, the statistics do not.
            if reached_server
                && let Err(e) = app.with_server(|db| db.record_mcp_usage(&project, &server, &tool, ok, ms, bytes_in, bytes_out))
            {
                eprintln!("genie mcp: cannot count a call of {server}/{tool}: {e}");
            }
            Ok(())
        })
        .await;
    if let Err(e) = written {
        eprintln!("genie mcp: {}: cannot record a call of {tool_for_log}: {e}", g.key);
    }
    reply
}

/// How long a cached tool listing is left alone between an agent's listings.
const CACHE_TTL: Duration = Duration::from_secs(3600);

/// Refresh the server's cached listing of `server` from a full `tools/list` answer,
/// at most once an hour: the web's tool pickers show the cache, so an administrator
/// need not check every connection by hand. Never blocks the agent's reply.
fn cache_tools(app: &Arc<App>, server: &str, tools: Vec<Value>) {
    let (app, server) = (app.clone(), server.to_string());
    let tools = serde_json::to_string(&tools).unwrap_or_default();
    tokio::spawn(async move {
        let name = server.clone();
        let written = app
            .blocking(move |app| {
                app.with_server(|db| {
                    let stale = db
                        .mcp_tools_checked(&server)?
                        .and_then(|at| chrono::DateTime::parse_from_rfc3339(&at).ok())
                        .is_none_or(|at| chrono::Utc::now().signed_duration_since(at) > chrono::Duration::seconds(CACHE_TTL.as_secs() as i64));
                    if stale {
                        db.set_mcp_tools(&server, &tools, &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true))?;
                    }
                    Ok(())
                })
            })
            .await;
        if let Err(e) = written {
            eprintln!("genie mcp: cannot cache the tools of {name}: {e}");
        }
    });
}

/// The first text of a tool result (what an error says).
fn first_text(result: &Value) -> String {
    result["content"].as_array().and_then(|c| c.iter().find_map(|x| x["text"].as_str())).unwrap_or("the tool reported an error").to_string()
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max { text.to_string() } else { format!("{}…", text.chars().take(max).collect::<String>()) }
}

/// Start a connection as agents would get it and list its tools (administrators).
async fn check(State(app): State<Arc<App>>, ctx: Ctx, Path(server): Path<String>) -> ApiResult<Json<Value>> {
    ctx.server_admin()?;
    let agents = app.agents();
    let srv = agents.mcp.get(&server).ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, format!("MCP connection {server} not found")))?;
    // The values an administrator keeps in the server's database come before its environment.
    let env = app.blocking(|app| app.with_server(|db| db.mcp_env())).await.unwrap_or_default();
    let mut config = srv.resolved_with(&env);
    if config.get("command").is_some()
        && config.get("cwd").is_none()
        && let Some(o) = config.as_object_mut()
    {
        let dir = app.data.join("runtime").join("mcp-check");
        std::fs::create_dir_all(&dir).map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("{}: {e}", dir.display())))?;
        o.insert("cwd".into(), json!(dir));
    }
    let started = Instant::now();
    let run = async {
        let up = Upstream::connect(&config, true).await?;
        let mut tools: Vec<Value> = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..20 {
            let params = cursor.as_ref().map_or_else(|| json!({}), |c| json!({ "cursor": c }));
            let r = up
                .request("tools/list", params)
                .await
                .map_err(|e| up.explain(&format!("tools/list: {}", e["message"].as_str().unwrap_or("failed"))))?;
            tools
                .extend(r["tools"].as_array().into_iter().flatten().map(|t| json!({ "name": t["name"], "description": t["description"] })));
            cursor = r["nextCursor"].as_str().map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        up.close();
        Ok::<_, String>((up.init.clone(), tools))
    };
    let ms = || started.elapsed().as_millis() as u64;
    Ok(Json(match tokio::time::timeout(CHECK_TIMEOUT, run).await {
        Ok(Ok((init, tools))) => {
            // The listing becomes the server's cache: the web's tool pickers and the
            // role editors choose from it.
            let cache = serde_json::to_string(&tools).unwrap_or_default();
            let checked = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            let (for_db, id, when) = (server.clone(), srv.id.clone(), checked.clone());
            let stored = app.blocking(move |app| app.with_server(|db| db.set_mcp_tools(&for_db, &cache, &when))).await;
            if let Err(e) = stored {
                eprintln!("genie mcp: cannot cache the tools of {id}: {e}");
            }
            json!({
                "ok": true,
                "ms": ms(),
                "serverInfo": init["serverInfo"],
                "protocolVersion": init["protocolVersion"],
                "instructions": init["instructions"],
                "tools": tools,
                "checkedAt": checked,
            })
        }
        Ok(Err(e)) => json!({ "ok": false, "ms": ms(), "error": e }),
        Err(_) => json!({ "ok": false, "ms": ms(), "error": format!("no answer in {} s", CHECK_TIMEOUT.as_secs()) }),
    }))
}

#[derive(Deserialize, Default)]
struct CallsQuery {
    limit: Option<usize>,
}

/// The project's latest MCP calls, newest first.
async fn calls(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<CallsQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    if access.agent {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "agents cannot read the MCP calls log"));
    }
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    let list =
        app.blocking(move |app| app.with_tracker(&access.project, |t| events::latest_of(t.conn(), events::MCP_CALLED, limit))).await?;
    Ok(Json(json!(list)))
}
