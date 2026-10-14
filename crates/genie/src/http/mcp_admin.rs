//! The administration of MCP connections: the structured editor over `mcp.json`,
//! the environment values it keeps sealed in the server's database, and the usage
//! statistics of the gateway (see `crate::mcp_gateway`). Server administrators
//! only (decision PD14); the client side reads `/api/mcp` and the catalogue.
//!
//! The editor writes `mcp.json` through the same machinery as the raw file
//! editor: checked by the loader exactly as it will be read, refused when it
//! breaks something that works now, compared against the version the editor
//! started from, written atomically, recorded in the history.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use super::agent_config::{Target, file_state, save};
use super::ctx::Ctx;
use super::{ApiError, ApiResult};
use crate::agent_config::McpServer;
use crate::state::App;

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/mcp-admin", get(admin))
        .route("/mcp-admin/servers/{id}", put(put_server).delete(delete_server))
        .route("/mcp-admin/secrets/{name}", put(put_secret).delete(delete_secret))
        .route("/mcp-admin/usage", get(usage))
}

/// The cached listings of all connections: server → (tools, when checked).
pub(crate) fn tool_cache(app: &App) -> HashMap<String, (Vec<Value>, String)> {
    app.with_server(|db| db.mcp_tools())
        .map(|rows| {
            rows.into_iter()
                .map(|(server, tools, checked)| (server, (serde_json::from_str(&tools).unwrap_or_default(), checked)))
                .collect()
        })
        .unwrap_or_default()
}

/// The servers a project may use, as the client catalogue shows them: with the
/// tools of the last listing the connection gave, the closed ones left out —
/// the role editors choose from this.
pub(crate) fn servers_with_tools(app: &App, agents: &crate::agent_config::AgentConfig, project: Option<&str>) -> Vec<Value> {
    let cache = tool_cache(app);
    agents
        .mcp
        .values()
        .filter(|s| project.is_none_or(|p| s.available_in(p)))
        .map(|s| {
            let mut v = serde_json::to_value(s).unwrap_or(Value::Null);
            if let Some((tools, checked)) = cache.get(&s.id)
                && let Some(o) = v.as_object_mut()
            {
                let open: Vec<&Value> = tools
                    .iter()
                    .filter(|t| t["name"].as_str().is_some_and(|n| !crate::mcp_gateway::hidden(&s.hidden, n)))
                    .collect();
                o.insert("tools".into(), json!(open));
                o.insert("checkedAt".into(), json!(checked));
            }
            v
        })
        .collect()
}

/// The entry exactly as `mcp.json` holds it: genie's own fields back in place.
fn entry_of(s: &McpServer) -> Value {
    let mut entry = s.config.clone();
    if let Some(o) = entry.as_object_mut() {
        if !s.description.is_empty() {
            o.insert("description".into(), json!(s.description));
        }
        if let Some(p) = &s.projects {
            o.insert("projects".into(), json!(p));
        }
        if !s.gateway {
            o.insert("gateway".into(), json!(false));
        }
        if !s.hidden.is_empty() {
            o.insert("hiddenTools".into(), json!(s.hidden));
        }
    }
    entry
}

/// Everything the administration of MCP connections needs in one answer.
async fn admin(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    ctx.server_admin()?;
    let out = app
        .blocking(|app| {
            let agents = app.agents();
            let cache = tool_cache(app);
            let servers: Vec<Value> = agents
                .mcp
                .values()
                .map(|s| {
                    let (tools, checked) = cache.get(&s.id).cloned().unwrap_or((Vec::new(), String::new()));
                    json!({
                        "id": s.id,
                        "description": s.description,
                        "transport": s.transport,
                        "projects": s.projects,
                        "gateway": s.gateway,
                        "hiddenTools": s.hidden,
                        "envRefs": s.env_refs(),
                        "entry": entry_of(s),
                        "tools": tools,
                        "checkedAt": if checked.is_empty() { Value::Null } else { json!(checked) },
                    })
                })
                .collect();
            // Which connections refer to which environment variable.
            let mut used_by: HashMap<String, Vec<String>> = HashMap::new();
            for s in agents.mcp.values() {
                for name in s.env_refs() {
                    used_by.entry(name).or_default().push(s.id.clone());
                }
            }
            let secrets: Vec<Value> = app
                .with_server(|db| db.mcp_secret_infos())?
                .into_iter()
                .map(|i| {
                    json!({
                        "name": i.name,
                        "hint": i.hint,
                        "updated": i.updated,
                        "unreadable": i.unreadable,
                        "usedBy": used_by.remove(&i.name).unwrap_or_default(),
                    })
                })
                .collect();
            // Variables an entry refers to but nobody set a value for.
            let unset: Vec<String> = {
                let mut names = used_by.keys().cloned().collect::<Vec<_>>();
                names.sort();
                names
            };
            let (content, hash) = file_state(&app.data.join("mcp.json"));
            Ok(json!({
                "servers": servers,
                "secrets": secrets,
                "secretsUnset": unset,
                "file": { "path": "mcp.json", "content": content, "hash": hash },
                "problems": agents.problems.iter().filter(|p| p.item.starts_with("mcp")).collect::<Vec<_>>(),
            }))
        })
        .await?;
    Ok(Json(out))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerBody {
    /// The entry as it will stand in `mcpServers`: genie's fields included.
    entry: Value,
    base_hash: Option<String>,
}

/// What `mcp.json` holds now, or an empty root when there is no file yet.
fn mcp_root(app: &App) -> Result<(Value, String, bool), ApiError> {
    let (current, hash) = file_state(&app.data.join("mcp.json"));
    let existed = current.is_some();
    let root = match &current {
        Some(text) => serde_json::from_str::<Value>(text)
            .map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("mcp.json is broken ({e}); fix the file before editing connections")))?,
        None => json!({ "mcpServers": {} }),
    };
    Ok((root, hash, existed))
}

/// A name pi accepts for a connection, and one the loader would not merge with another.
fn valid_mcp_id(id: &str, servers: &serde_json::Map<String, Value>) -> Result<(), ApiError> {
    let bad = || ApiError::bad("a connection's name is letters, digits, `_` and `-`");
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(bad());
    }
    let merged = |s: &str| s.replace('-', "_");
    if servers.keys().any(|other| other != id && merged(other) == merged(id)) {
        return Err(ApiError::bad(format!(
            "pi takes names that differ only in `-` and `_` for one connection; `{id}` would repeat one that is already there"
        )));
    }
    Ok(())
}

/// Add or replace one connection of `mcp.json` (the entry as it will stand there).
async fn put_server(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Json(b): Json<ServerBody>) -> ApiResult<Json<Value>> {
    let user = ctx.server_admin()?.login.clone();
    let out = app.blocking(move |app| Ok(put_server_sync(app, &user, &id, b))).await?;
    Ok(Json(out?))
}

fn put_server_sync(app: &App, user: &str, id: &str, b: ServerBody) -> Result<Value, ApiError> {
    if !b.entry.is_object() {
        return Err(ApiError::bad("pass the connection as `entry`, an object"));
    }
    let (mut root, hash, existed) = mcp_root(app)?;
    let servers = root["mcpServers"]
        .as_object_mut()
        .ok_or_else(|| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "mcp.json has no {\"mcpServers\": {…}} object; fix the file first"))?;
    valid_mcp_id(id, servers)?;
    servers.insert(id.to_string(), b.entry);
    // A structured edit always builds on the file as it is now.
    let base = b.base_hash.filter(|_| existed).unwrap_or(hash);
    let content = serde_json::to_string_pretty(&root).map_err(|e| ApiError::from(crate::state::AppError::Internal(e.to_string())))?;
    save(app, user, &Target::mcp(app), Some(content), Some(&base))
}

/// Remove one connection of `mcp.json` and forget its cached tool listing.
async fn delete_server(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Query(q): Query<DeleteQuery>) -> ApiResult<Json<Value>> {
    let user = ctx.server_admin()?.login.clone();
    let out = app.blocking(move |app| Ok(delete_server_sync(app, &user, &id, &q.base_hash))).await?;
    Ok(Json(out?))
}

fn delete_server_sync(app: &App, user: &str, id: &str, base_hash: &Option<String>) -> Result<Value, ApiError> {
    let (mut root, hash, _) = mcp_root(app)?;
    let servers = root["mcpServers"]
        .as_object_mut()
        .ok_or_else(|| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "mcp.json has no {\"mcpServers\": {…}} object; fix the file first"))?;
    if servers.remove(id).is_none() {
        return Err(ApiError::new(StatusCode::NOT_FOUND, format!("MCP connection {id} not found")));
    }
    let base = base_hash.clone().unwrap_or(hash);
    let content = serde_json::to_string_pretty(&root).map_err(|e| ApiError::from(crate::state::AppError::Internal(e.to_string())))?;
    let out = save(app, user, &Target::mcp(app), Some(content), Some(&base))?;
    if let Err(e) = app.with_server(|db| db.forget_mcp_tools(id)) {
        eprintln!("genie mcp: cannot forget the cached tools of {id}: {e}");
    }
    Ok(out)
}

#[derive(Deserialize, Default)]
struct DeleteQuery {
    base_hash: Option<String>,
}

#[derive(Deserialize)]
struct SecretBody {
    value: String,
}

/// Set the value of an environment variable MCP connections refer to (a blank
/// value removes it); it is sealed in the server's database and never read back
/// except to open a connection.
async fn put_secret(State(app): State<Arc<App>>, ctx: Ctx, Path(name): Path<String>, Json(b): Json<SecretBody>) -> ApiResult<Json<Value>> {
    ctx.server_admin()?;
    app.blocking(move |app| app.with_server(|db| db.set_mcp_secret(&name, &b.value))).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_secret(State(app): State<Arc<App>>, ctx: Ctx, Path(name): Path<String>) -> ApiResult<Json<Value>> {
    ctx.server_admin()?;
    app.blocking(move |app| app.with_server(|db| db.delete_mcp_secret(&name))).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize, Default)]
struct UsageQuery {
    days: Option<u64>,
}

/// The gateway's tool calls of the last `days` days: per day, project, server
/// and tool, with what each call moved both ways. The web adds these up.
async fn usage(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<UsageQuery>) -> ApiResult<Json<Value>> {
    ctx.server_admin()?;
    let days = q.days.unwrap_or(30).clamp(1, 365);
    let rows = app.blocking(move |app| app.with_server(|db| db.mcp_usage(days))).await?;
    Ok(Json(json!({ "days": days, "rows": rows })))
}
