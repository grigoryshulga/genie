//! The MCP gateway: agents reach the MCP connections of their role through genie.
//!
//! An agent's harness speaks MCP (Streamable HTTP, JSON responses) to
//! `/api/mcp-gateway/<server>` with its genie token. genie connects to the real
//! server — a process it starts, or an HTTP endpoint — with the secrets of its
//! own environment, lets through only the tools the role was granted and
//! records every tool call in the project's journal (`mcp.called`). The agent
//! never holds the secrets: its MCP config names only the gateway, and the
//! variables `mcp.json` refers to are left out of its environment.
//!
//! One connection per agent and server, opened on the agent's first request and
//! closed after `IDLE`; a server process runs in the agent's working directory.
//! A server's own requests (sampling, roots, elicitation) are answered with an
//! error, its notifications dropped.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, oneshot};

/// The MCP version genie asks servers for.
pub const PROTOCOL: &str = "2025-06-18";
/// How long one request may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// A connection nobody used this long is closed (its server process stops).
pub const IDLE: Duration = Duration::from_secs(600);

/// A server's answer: its result, or a JSON-RPC error object.
pub type Reply = Result<Value, Value>;

pub fn rpc_error(code: i64, message: impl Into<String>) -> Value {
    json!({ "code": code, "message": message.into() })
}

type Pending = Arc<StdMutex<HashMap<i64, oneshot::Sender<Value>>>>;

enum Transport {
    Stdio {
        stdin: Arc<Mutex<tokio::process::ChildStdin>>,
        pending: Pending,
        child: StdMutex<Option<tokio::process::Child>>,
    },
    Http {
        client: reqwest::Client,
        url: String,
        headers: Vec<(String, String)>,
        session: StdMutex<Option<String>>,
        version: StdMutex<Option<String>>,
    },
}

/// An initialized connection to a real MCP server.
pub struct Upstream {
    transport: Transport,
    next: AtomicI64,
    /// The server's answer to `initialize`.
    pub init: Value,
    /// The entry it was opened with: a changed configuration opens a new connection.
    config: Value,
    used: StdMutex<Instant>,
    /// The server process stopped, or the server ended the session.
    gone: Arc<AtomicBool>,
    /// The last lines the server process wrote to stderr (when kept, see `connect`).
    output: Arc<StdMutex<VecDeque<String>>>,
}

fn lock<T>(m: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Upstream {
    /// Connect to a server (a resolved `mcp.json` entry: `command` or `url`) and
    /// initialize it. `keep_output`: keep what a server process writes to stderr
    /// for the error messages (a check by an administrator; agents never see it).
    pub async fn connect(config: &Value, keep_output: bool) -> Result<Upstream, String> {
        let gone = Arc::new(AtomicBool::new(false));
        let output: Arc<StdMutex<VecDeque<String>>> = Arc::default();
        let transport = if let Some(url) = config["url"].as_str() {
            let mut headers: Vec<(String, String)> = config["headers"]
                .as_object()
                .map(|h| h.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect())
                .unwrap_or_default();
            if let Some(token) = config["bearerToken"].as_str() {
                headers.push(("Authorization".into(), format!("Bearer {token}")));
            }
            let client = reqwest::Client::builder().timeout(CALL_TIMEOUT).build().map_err(|e| e.to_string())?;
            Transport::Http { client, url: url.to_string(), headers, session: StdMutex::new(None), version: StdMutex::new(None) }
        } else if let Some(command) = config["command"].as_str() {
            let args: Vec<&str> = config["args"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            let mut cmd = tokio::process::Command::new(command);
            let stderr = if keep_output { Stdio::piped() } else { Stdio::null() };
            cmd.args(&args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(stderr).kill_on_drop(true);
            for (k, v) in config["env"].as_object().into_iter().flatten() {
                if let Some(v) = v.as_str() {
                    cmd.env(k, v);
                }
            }
            if let Some(cwd) = config["cwd"].as_str() {
                cmd.current_dir(cwd);
            }
            let mut child = cmd.spawn().map_err(|e| format!("cannot start {command}: {e}"))?;
            let stdin = Arc::new(Mutex::new(child.stdin.take().ok_or("the server has no stdin")?));
            let stdout = child.stdout.take().ok_or("the server has no stdout")?;
            if let Some(err) = child.stderr.take() {
                let out = output.clone();
                tokio::spawn(async move {
                    let mut lines = BufReader::new(err).lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let mut o = lock(&out);
                        if o.len() == 20 {
                            o.pop_front();
                        }
                        o.push_back(line.chars().take(300).collect());
                    }
                });
            }
            let pending: Pending = Arc::default();
            let (waiting, writer, stopped) = (pending.clone(), stdin.clone(), gone.clone());
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                    match (msg.get("method"), msg.get("id")) {
                        (None, Some(id)) => {
                            if let Some(tx) = id.as_i64().and_then(|id| lock(&waiting).remove(&id)) {
                                let _ = tx.send(msg);
                            }
                        }
                        (Some(_), Some(id)) => {
                            let reply =
                                json!({ "jsonrpc": "2.0", "id": id, "error": rpc_error(-32601, "not offered through the genie gateway") });
                            let mut w = writer.lock().await;
                            let _ = w.write_all(format!("{reply}\n").as_bytes()).await;
                            let _ = w.flush().await;
                        }
                        _ => {}
                    }
                }
                // The server stopped: whoever waits hears it.
                stopped.store(true, Ordering::Relaxed);
                lock(&waiting).clear();
            });
            Transport::Stdio { stdin, pending, child: StdMutex::new(Some(child)) }
        } else {
            return Err("the connection needs `command` or `url`".into());
        };
        let mut up = Upstream {
            transport,
            next: AtomicI64::new(1),
            init: Value::Null,
            config: config.clone(),
            used: StdMutex::new(Instant::now()),
            gone,
            output,
        };
        let params = json!({ "protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": { "name": "genie", "version": env!("CARGO_PKG_VERSION") } });
        let init = match up.request("initialize", params).await {
            Ok(v) => v,
            Err(e) => {
                if keep_output {
                    // Let the last words of a stopped server arrive.
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                up.close();
                return Err(up.explain(&format!("initialize: {}", e["message"].as_str().unwrap_or("failed"))));
            }
        };
        if let Transport::Http { version, .. } = &up.transport {
            *lock(version) = init["protocolVersion"].as_str().map(str::to_string);
        }
        up.notify("notifications/initialized", json!({})).await?;
        up.init = init;
        Ok(up)
    }

    /// Send a request and wait for its answer.
    pub async fn request(&self, method: &str, params: Value) -> Reply {
        *lock(&self.used) = Instant::now();
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let answer = match &self.transport {
            Transport::Stdio { stdin, pending, .. } => {
                let (tx, rx) = oneshot::channel();
                lock(pending).insert(id, tx);
                // The reader marks the server gone before it drops the waiting requests.
                if !self.alive() {
                    lock(pending).remove(&id);
                    return Err(rpc_error(-32000, "the MCP server has stopped"));
                }
                {
                    let mut w = stdin.lock().await;
                    if w.write_all(format!("{msg}\n").as_bytes()).await.is_err() || w.flush().await.is_err() {
                        lock(pending).remove(&id);
                        self.gone.store(true, Ordering::Relaxed);
                        return Err(rpc_error(-32000, "the MCP server has stopped"));
                    }
                }
                match tokio::time::timeout(CALL_TIMEOUT, rx).await {
                    Ok(Ok(m)) => m,
                    Ok(Err(_)) => return Err(rpc_error(-32000, "the MCP server has stopped")),
                    Err(_) => {
                        lock(pending).remove(&id);
                        return Err(rpc_error(-32000, "the MCP server did not answer in time"));
                    }
                }
            }
            Transport::Http { .. } => match self.http(&msg, Some(id)).await {
                Ok(Some(m)) => m,
                Ok(None) => return Err(rpc_error(-32603, "the MCP server gave no answer")),
                Err(e) => return Err(rpc_error(-32000, e)),
            },
        };
        match (answer.get("error"), answer.get("result")) {
            (Some(e), _) => Err(e.clone()),
            (None, Some(r)) => Ok(r.clone()),
            _ => Err(rpc_error(-32603, "the MCP server gave no answer")),
        }
    }

    /// Send a notification.
    pub async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        let msg = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        match &self.transport {
            Transport::Stdio { stdin, .. } => {
                let mut w = stdin.lock().await;
                w.write_all(format!("{msg}\n").as_bytes()).await.map_err(|e| e.to_string())?;
                w.flush().await.map_err(|e| e.to_string())
            }
            Transport::Http { .. } => self.http(&msg, None).await.map(|_| ()),
        }
    }

    /// POST one message over Streamable HTTP; the answer to `id`, from a JSON body
    /// or an event stream (`None` for a notification).
    async fn http(&self, msg: &Value, id: Option<i64>) -> Result<Option<Value>, String> {
        let Transport::Http { client, url, headers, session, version } = &self.transport else {
            return Err("not an HTTP connection".into());
        };
        let mut req = client
            .post(url)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(msg.to_string());
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        if let Some(s) = lock(session).clone() {
            req = req.header("mcp-session-id", s);
        }
        if let Some(v) = lock(version).clone() {
            req = req.header("mcp-protocol-version", v);
        }
        let had_session = lock(session).is_some();
        // Without the URL: it may hold a secret (`?token=${env:…}`), and the error reaches the agent.
        let res = req.send().await.map_err(|e| e.without_url().to_string())?;
        if let Some(s) = res.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            *lock(session) = Some(s.to_string());
        }
        let status = res.status();
        if status == reqwest::StatusCode::NOT_FOUND && had_session {
            // The server forgot the session: the next request opens a new connection.
            self.gone.store(true, Ordering::Relaxed);
            return Err("the MCP server ended the session; try again".into());
        }
        let stream = res.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|c| c.starts_with("text/event-stream"));
        let body = res.text().await.map_err(|e| e.without_url().to_string())?;
        if !status.is_success() {
            return Err(format!("HTTP {status}: {}", body.chars().take(300).collect::<String>()));
        }
        let Some(id) = id else { return Ok(None) };
        let is_answer = |m: &Value| m["id"].as_i64() == Some(id) && m.get("method").is_none();
        let messages: Vec<Value> = if stream {
            body.replace("\r\n", "\n")
                .split("\n\n")
                .filter_map(|event| {
                    let data: Vec<&str> = event.lines().filter_map(|l| l.strip_prefix("data:").map(str::trim_start)).collect();
                    serde_json::from_str::<Value>(&data.join("\n")).ok()
                })
                .collect()
        } else {
            match serde_json::from_str::<Value>(&body) {
                Ok(Value::Array(a)) => a,
                Ok(v) => vec![v],
                Err(_) => Vec::new(),
            }
        };
        Ok(messages.into_iter().find(is_answer))
    }

    /// Stop the server process (an HTTP connection just goes away).
    pub fn close(&self) {
        if let Transport::Stdio { child, .. } = &self.transport
            && let Some(mut c) = lock(child).take()
        {
            let _ = c.start_kill();
        }
    }

    fn idle(&self) -> Duration {
        lock(&self.used).elapsed()
    }

    /// Whether the connection still works (a stopped server is reconnected).
    pub fn alive(&self) -> bool {
        !self.gone.load(Ordering::Relaxed)
    }

    /// An error message with the last lines the server process wrote (if kept).
    pub fn explain(&self, message: &str) -> String {
        let out = lock(&self.output);
        if out.is_empty() {
            message.to_string()
        } else {
            format!("{message}\nthe server wrote:\n{}", out.iter().cloned().collect::<Vec<_>>().join("\n"))
        }
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.close();
    }
}

type Slot = Arc<Mutex<Option<Arc<Upstream>>>>;

/// How many agents' working directories are remembered.
const PLACES: usize = 1000;

/// The agents' connections, by agent and server.
#[derive(Default)]
pub struct Gateway {
    conns: StdMutex<HashMap<String, Slot>>,
    /// Where each agent works (by `agent_key`), and since when.
    places: StdMutex<HashMap<String, (PathBuf, Instant)>>,
}

/// An agent in connection keys: `<project>/<team or ->/<name>`.
pub fn agent_key(project: &str, team: Option<&str>, name: &str) -> String {
    format!("{project}/{}/{name}", team.unwrap_or("-"))
}

impl Gateway {
    /// Remember where an agent works: a server process started for it runs there.
    pub fn place(&self, agent: &str, cwd: &Path) {
        let mut places = lock(&self.places);
        if places.len() >= PLACES && !places.contains_key(agent) {
            let mut by_age: Vec<(String, Instant)> = places.iter().map(|(k, (_, at))| (k.clone(), *at)).collect();
            by_age.sort_by_key(|(_, at)| *at);
            for (k, _) in by_age.into_iter().take(PLACES / 2) {
                places.remove(&k);
            }
        }
        places.insert(agent.to_string(), (cwd.to_path_buf(), Instant::now()));
    }

    /// Where an agent works, if it was started by this server.
    pub fn place_of(&self, agent: &str) -> Option<PathBuf> {
        lock(&self.places).get(agent).map(|(p, _)| p.clone())
    }

    /// The connection `key`, opened on first use (one at a time per key) and
    /// opened again when its server stopped or its configuration changed.
    pub async fn get(&self, key: &str, config: &Value) -> Result<Arc<Upstream>, String> {
        let slot = lock(&self.conns).entry(key.to_string()).or_default().clone();
        let mut s = slot.lock().await;
        if let Some(up) = s.as_ref() {
            if up.alive() && up.config == *config {
                return Ok(up.clone());
            }
            up.close();
            *s = None;
        }
        let up = Arc::new(Upstream::connect(config, false).await?);
        *s = Some(up.clone());
        Ok(up)
    }

    /// How many connections are open.
    pub fn open(&self) -> usize {
        lock(&self.conns).values().filter(|slot| slot.try_lock().map_or(true, |s| s.is_some())).count()
    }

    /// Close connections unused for `idle` (and forget failed ones).
    pub fn close_idle(&self, idle: Duration) {
        lock(&self.conns).retain(|_, slot| match slot.try_lock() {
            Ok(s) => match s.as_ref() {
                Some(up) if up.idle() >= idle || !up.alive() => {
                    up.close();
                    false
                }
                Some(_) => true,
                None => false,
            },
            Err(_) => true,
        });
    }
}

/// Whether a tool is within a grant (`None`: every tool of the connection).
pub fn allowed(patterns: Option<&[String]>, tool: &str) -> bool {
    patterns.is_none_or(|ps| ps.iter().any(|p| glob(p, tool)))
}

/// Whether the administrator's `hiddenTools` patterns close a tool for everyone.
pub fn hidden(patterns: &[String], tool: &str) -> bool {
    patterns.iter().any(|p| glob(p, tool))
}

/// `*` matches any text, `?` one character.
fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut i, mut j, mut star, mut mark) = (0, 0, None, 0);
    while j < t.len() {
        if i < p.len() && (p[i] == '?' || p[i] == t[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some(i);
            mark = j;
            i += 1;
        } else if let Some(s) = star {
            i = s + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    p[i..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_patterns_are_globs() {
        let ps = vec!["get_*".to_string(), "list_commits".to_string(), "search_?".to_string()];
        assert!(allowed(Some(&ps), "get_issue"));
        assert!(allowed(Some(&ps), "list_commits"));
        assert!(allowed(Some(&ps), "search_x"));
        assert!(!allowed(Some(&ps), "search_xy"));
        assert!(!allowed(Some(&ps), "delete_repo"));
        assert!(!allowed(Some(&ps), "list_commits_all"));
        assert!(allowed(None, "anything"));
        assert!(glob("*", "") && glob("a*b*c", "aXbYYc") && !glob("a*b", "aXc"));
    }

    #[test]
    fn hidden_tools_close_whatever_a_role_grants() {
        let closed = vec!["dangerous_*".to_string(), "drop_table".to_string()];
        assert!(hidden(&closed, "dangerous_drop"));
        assert!(hidden(&closed, "drop_table"));
        assert!(!hidden(&closed, "drop_table_all"));
        assert!(!hidden(&closed, "get_issue"));
        assert!(!hidden(&[], "anything"));
        // The whole connection granted, one tool still closed.
        assert!(allowed(None, "dangerous_drop") && hidden(&closed, "dangerous_drop"));
    }
}
