//! The MCP gateway: agents reach their role's MCP connections through genie,
//! which holds the secrets, lets through only the granted tools and records
//! every tool call in the project's journal. The servers here are real: a
//! process speaking MCP over stdio (`fixtures/fake-mcp-server.mjs`) and an HTTP
//! endpoint with sessions and event-stream answers.

use crate::common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use common::{Harness, call};
use genie_core::Role;
use serde_json::{Value, json};

const SECRET: &str = "tr4cker-s3cret";

/// The server's secret, in its environment only (set once for the whole test binary).
fn secret_env() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    // SAFETY: set before any test of this binary reads it, always to the same value.
    ONCE.call_once(|| unsafe { std::env::set_var("GENIE_TEST_TRACKER_TOKEN", SECRET) });
}

fn node() -> bool {
    let found = std::process::Command::new("node").arg("--version").output().is_ok_and(|o| o.status.success());
    if !found {
        assert!(std::env::var("CI").is_err(), "node is not installed");
        eprintln!("skipped: node is not installed");
    }
    found
}

fn fixture() -> String {
    format!("{}/tests/fixtures/fake-mcp-server.mjs", env!("CARGO_MANIFEST_DIR"))
}

fn write(h: &Harness, rel: &str, text: &str) {
    let p = h.dir.path().join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn token(h: &Harness, role: &str, name: &str, team: Option<&str>) -> String {
    h.app.with_server(|db| db.create_role_token("shop", Role::Reviewer, Some(role), name, team, None, chrono::Duration::hours(1))).unwrap()
}

/// One JSON-RPC request to the gateway: its HTTP status and body.
async fn rpc(h: &Harness, server: &str, token: &str, method: &str, params: Value) -> (StatusCode, Value) {
    let (s, v, _) = call(&h.router, "POST", &format!("/api/mcp-gateway/{server}"))
        .bearer(token)
        .no_csrf()
        .json(json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params }))
        .send()
        .await;
    (s, v)
}

/// A tool call's result (panics on a JSON-RPC error).
async fn tool(h: &Harness, server: &str, token: &str, name: &str, args: Value) -> Value {
    let (s, v) = rpc(h, server, token, "tools/call", json!({ "name": name, "arguments": args })).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v.get("error").is_none(), "{name}: {v}");
    v["result"].clone()
}

fn text(result: &Value) -> String {
    result["content"][0]["text"].as_str().unwrap_or_default().to_string()
}

/// The project's MCP calls, newest first, as the web gets them.
async fn calls(h: &Harness) -> Vec<Value> {
    let (s, v, _) = call(&h.router, "GET", "/api/mcp/calls?project=shop").send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v.as_array().unwrap().clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_reaches_only_its_granted_tools_and_every_call_is_recorded() {
    if !node() {
        return;
    }
    secret_env();
    let h = Harness::new();
    h.project("shop");
    let mcp = json!({ "mcpServers": {
        "tracker": { "description": "Issues", "command": "node", "args": [fixture()], "env": { "TRACKER_TOKEN": "${env:GENIE_TEST_TRACKER_TOKEN}" } },
        "broken": { "command": "node", "args": [fixture(), "--fail"] }
    }});
    write(&h, "mcp.json", &mcp.to_string());
    write(
        &h,
        "agents/auditor.md",
        "---\nbase: reviewer\nmcp: [\"tracker:get_*\", \"tracker:whoami\", \"tracker:ask_client\", \"tracker:fail\"]\n---\nx\n",
    );
    write(&h, "agents/lead.md", "---\nbase: reviewer\nmcp: [tracker]\n---\nx\n");
    write(&h, "agents/plain.md", "---\nbase: reviewer\n---\nx\n");
    let cfg = h.app.reload_agents();
    assert_eq!(cfg.errors().count(), 0, "{:?}", cfg.problems);
    let yoda = token(&h, "auditor", "yoda", Some("SHOP-1"));
    let lead = token(&h, "lead", "lead", Some("SHOP-1"));
    let plain = token(&h, "plain", "chaos", Some("SHOP-1"));
    let place = h.dir.path().join("worktrees/SHOP-1");
    std::fs::create_dir_all(&place).unwrap();
    h.app.mcp.place(&genie::mcp_gateway::agent_key("shop", Some("SHOP-1"), "yoda"), &place);

    // The handshake: the server's own answer, less what the gateway does not pass on.
    let (s, v) = rpc(
        &h,
        "tracker",
        &yoda,
        "initialize",
        json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "pi" } }),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["id"], 7);
    assert_eq!(v["result"]["serverInfo"]["name"], "fake-tracker");
    assert_eq!(v["result"]["instructions"], "Read issues with get_issue.");
    assert_eq!(v["result"]["capabilities"], json!({ "tools": {} }), "no notifications; some tools only, so no resources either");
    let (s, _, _) = call(&h.router, "POST", "/api/mcp-gateway/tracker")
        .bearer(&yoda)
        .no_csrf()
        .json(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
        .send()
        .await;
    assert_eq!(s, StatusCode::ACCEPTED);

    // Only the granted tools are listed and callable.
    let (_, v) = rpc(&h, "tracker", &yoda, "tools/list", json!({})).await;
    let names: Vec<&str> = v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["get_issue", "whoami", "ask_client", "fail"]);
    assert_eq!(text(&tool(&h, "tracker", &yoda, "get_issue", json!({ "id": "7" })).await), r#"get_issue {"id":"7"}"#);
    let (_, v) = rpc(&h, "tracker", &yoda, "tools/call", json!({ "name": "delete_issue", "arguments": {} })).await;
    assert_eq!(v["error"]["code"], -32602);
    assert!(v["error"]["message"].as_str().unwrap().contains("not granted to role auditor"), "{v}");
    let (_, v) = rpc(&h, "tracker", &yoda, "resources/list", json!({})).await;
    assert!(v["error"]["message"].as_str().unwrap().contains("only some tools"), "{v}");

    // The server has the secret and runs where the agent works; the agent never sees the secret.
    let me: Value = serde_json::from_str(&text(&tool(&h, "tracker", &yoda, "whoami", json!({})).await)).unwrap();
    assert_eq!(me["token"], SECRET);
    assert_eq!(PathBuf::from(me["cwd"].as_str().unwrap()).canonicalize().unwrap(), place.canonicalize().unwrap());
    let again: Value = serde_json::from_str(&text(&tool(&h, "tracker", &yoda, "whoami", json!({})).await)).unwrap();
    assert_eq!(again["pid"], me["pid"], "one connection per agent and server");
    let theirs: Value = serde_json::from_str(&text(&tool(&h, "tracker", &lead, "whoami", json!({})).await)).unwrap();
    assert_ne!(theirs["pid"], me["pid"], "another agent has its own");
    let (_, v) = rpc(&h, "tracker", &lead, "resources/list", json!({})).await;
    assert_eq!(v["result"]["resources"][0]["name"], "readme", "a grant of the whole connection has its resources");

    // The server's own requests are refused; a tool's error is its result.
    assert_eq!(text(&tool(&h, "tracker", &yoda, "ask_client", json!({})).await), "the client said: not offered through the genie gateway");
    let failed = tool(&h, "tracker", &yoda, "fail", json!({})).await;
    assert_eq!(failed["isError"], true);

    // A batch gets its answers in one response; a notification in it gets none.
    let (s, v, _) = call(&h.router, "POST", "/api/mcp-gateway/tracker")
        .bearer(&yoda)
        .no_csrf()
        .json(json!([
            { "jsonrpc": "2.0", "id": "a", "method": "ping" },
            { "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": 1 } },
            { "jsonrpc": "2.0", "id": "b", "method": "sampling/createMessage" }
        ]))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v[0], json!({ "jsonrpc": "2.0", "id": "a", "result": {} }));
    assert_eq!(v[1]["error"]["code"], -32601);
    assert_eq!(v.as_array().unwrap().len(), 2);
    let (s, v, _) =
        call(&h.router, "POST", "/api/mcp-gateway/tracker").bearer(&yoda).no_csrf().header("content-type", "application/json").send().await;
    assert_eq!((s, v["error"]["code"].clone()), (StatusCode::BAD_REQUEST, json!(-32700)));
    let pings: Vec<Value> = (0..51).map(|i| json!({ "jsonrpc": "2.0", "id": i, "method": "ping" })).collect();
    let (s, v, _) = call(&h.router, "POST", "/api/mcp-gateway/tracker").bearer(&yoda).no_csrf().json(json!(pings)).send().await;
    assert_eq!((s, v["error"]["code"].clone()), (StatusCode::BAD_REQUEST, json!(-32600)), "a batch has at most 50 messages");

    // Who may not: a role without the connection, a person, nobody; no stream to open.
    let (s, v) = rpc(&h, "tracker", &plain, "tools/list", json!({})).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert!(v["error"].as_str().unwrap().contains("role plain has no access to the MCP connection tracker"), "{v}");
    let (s, _, _) =
        call(&h.router, "POST", "/api/mcp-gateway/tracker").json(json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" })).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN, "people check connections instead");
    let (s, _, _) =
        call(&h.remote, "POST", "/api/mcp-gateway/tracker").json(json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" })).send().await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _, _) = call(&h.router, "GET", "/api/mcp-gateway/tracker").bearer(&yoda).send().await;
    assert_eq!(s, StatusCode::METHOD_NOT_ALLOWED);

    // Every tool call is in the project's journal, the refused ones too.
    let log = calls(&h).await;
    let of = |tool: &str| log.iter().filter(|e| e["payload"]["tool"] == tool).cloned().collect::<Vec<_>>();
    let get = &of("get_issue")[0];
    assert_eq!(
        (get["type"].as_str(), get["actor"].as_str(), get["actorRole"].as_str()),
        (Some("mcp.called"), Some("yoda"), Some("reviewer"))
    );
    assert_eq!(get["subject"], "SHOP-1");
    assert_eq!(get["payload"]["server"], "tracker");
    assert_eq!(get["payload"]["role"], "auditor");
    assert_eq!(get["payload"]["ok"], true);
    assert_eq!(get["payload"]["args"], r#"{"id":"7"}"#);
    let refused = &of("delete_issue")[0]["payload"];
    assert_eq!((refused["ok"].clone(), refused["refused"].clone()), (json!(false), json!(true)));
    assert_eq!(of("fail")[0]["payload"]["error"], "the tracker is down");
    assert_eq!(of("whoami").len(), 3);
    assert_eq!(log[0]["payload"]["tool"], "fail", "newest first");
    let (s, _, _) = call(&h.router, "GET", "/api/mcp/calls").bearer(&yoda).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN, "agents do not read the log");

    // A revoked tool stops working at once.
    write(&h, "agents/auditor.md", "---\nbase: reviewer\nmcp: [\"tracker:whoami\"]\n---\nx\n");
    h.app.reload_agents();
    let (_, v) = rpc(&h, "tracker", &yoda, "tools/call", json!({ "name": "get_issue", "arguments": {} })).await;
    assert_eq!(v["error"]["code"], -32602, "{v}");

    // Idle connections close; the next call opens a new one.
    h.app.mcp.close_idle(Duration::ZERO);
    assert_eq!(h.app.mcp.open(), 0);
    let fresh: Value = serde_json::from_str(&text(&tool(&h, "tracker", &yoda, "whoami", json!({})).await)).unwrap();
    assert_ne!(fresh["pid"], me["pid"]);

    // Administrators check a connection: its tools, or why it does not start.
    let (s, v, _) = call(&h.router, "POST", "/api/mcp/tracker/check").send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["serverInfo"]["name"], "fake-tracker");
    assert_eq!(v["tools"].as_array().unwrap().len(), 6);
    let (_, v, _) = call(&h.router, "POST", "/api/mcp/broken/check").send().await;
    assert_eq!(v["ok"], false);
    assert!(v["error"].as_str().unwrap().contains("cannot log in: TRACKER_TOKEN is not set"), "{v}");
    let (s, _, _) = call(&h.router, "POST", "/api/mcp/tracker/check").bearer(&yoda).no_csrf().send().await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

/// An MCP server over Streamable HTTP: it wants the secret, hands out a session
/// and answers some requests with an event stream.
#[derive(Default)]
struct Web {
    /// What each request carried: method, authorization, session, protocol version.
    seen: Mutex<Vec<[Option<String>; 4]>>,
    sessions: AtomicUsize,
    /// Forget the session at the next request (as a restarted server would).
    forget: AtomicBool,
}

async fn web(State(w): State<Arc<Web>>, headers: HeaderMap, body: String) -> Response {
    let m: Value = serde_json::from_str(&body).unwrap();
    let h = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
    let method = m["method"].as_str().unwrap_or_default().to_string();
    w.seen.lock().unwrap().push([Some(method.clone()), h("authorization"), h("mcp-session-id"), h("mcp-protocol-version")]);
    if h("authorization") != Some(format!("Bearer {SECRET}")) {
        return (StatusCode::UNAUTHORIZED, "who are you").into_response();
    }
    let stream = |messages: Vec<Value>| {
        let body: String = messages.iter().map(|m| format!("event: message\ndata: {m}\n\n")).collect();
        Response::builder().header("content-type", "text/event-stream").body(axum::body::Body::from(body)).unwrap()
    };
    if method == "initialize" {
        let n = w.sessions.fetch_add(1, Ordering::SeqCst) + 1;
        let mut r = stream(vec![json!({ "jsonrpc": "2.0", "id": m["id"], "result": {
            "protocolVersion": "2025-06-18", "capabilities": { "tools": {} }, "serverInfo": { "name": "fake-web", "version": "2" }
        }})]);
        r.headers_mut().insert("mcp-session-id", format!("s-{n}").parse().unwrap());
        return r;
    }
    let current = format!("s-{}", w.sessions.load(Ordering::SeqCst));
    if h("mcp-session-id") != Some(current) || w.forget.swap(false, Ordering::SeqCst) {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    }
    if m.get("id").is_none() {
        return StatusCode::ACCEPTED.into_response();
    }
    match method.as_str() {
        "tools/list" => axum::Json(json!({ "jsonrpc": "2.0", "id": m["id"], "result": { "tools": [
            { "name": "search_docs", "inputSchema": { "type": "object" } },
            { "name": "drop_docs", "inputSchema": { "type": "object" } }
        ]}}))
        .into_response(),
        _ => stream(vec![
            json!({ "jsonrpc": "2.0", "method": "notifications/progress", "params": { "progressToken": 1, "progress": 1 } }),
            json!({ "jsonrpc": "2.0", "id": m["id"], "result": { "content": [{ "type": "text", "text": format!("found {}", m["params"]["arguments"]["q"]) }] } }),
        ]),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_http_server_is_reached_with_the_servers_secret_and_its_session() {
    secret_env();
    let w = Arc::new(Web::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let router = axum::Router::new().route("/mcp", axum::routing::post(web)).with_state(w.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let h = Harness::new();
    h.project("shop");
    let mcp = json!({ "mcpServers": {
        "web": { "type": "http", "url": format!("http://127.0.0.1:{port}/mcp"), "headers": { "Authorization": "Bearer ${env:GENIE_TEST_TRACKER_TOKEN}" } },
        "down": { "type": "http", "url": "http://127.0.0.1:1/mcp?key=${env:GENIE_TEST_TRACKER_TOKEN}" }
    }});
    write(&h, "mcp.json", &mcp.to_string());
    write(&h, "agents/reader.md", "---\nbase: analyst\nmcp: [\"web:search_*\", down]\n---\nx\n");
    h.app.reload_agents();
    let t = token(&h, "reader", "ann", None);

    let (_, v) = rpc(&h, "web", &t, "tools/list", json!({})).await;
    assert_eq!(v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].clone()).collect::<Vec<_>>(), vec![json!("search_docs")]);
    assert_eq!(
        text(&tool(&h, "web", &t, "search_docs", json!({ "q": "gateway" })).await),
        r#"found "gateway""#,
        "the answer in an event stream"
    );
    {
        let seen = w.seen.lock().unwrap();
        let methods: Vec<&str> = seen.iter().map(|s| s[0].as_deref().unwrap()).collect();
        assert_eq!(methods, vec!["initialize", "notifications/initialized", "tools/list", "tools/call"]);
        assert!(seen.iter().all(|s| s[1].as_deref() == Some(&*format!("Bearer {SECRET}"))), "the server's secret on every request");
        assert!(seen[1..].iter().all(|s| s[2].as_deref() == Some("s-1") && s[3].as_deref() == Some("2025-06-18")), "{seen:?}");
    }

    // The server forgot the session: this call fails, the next opens a new one.
    w.forget.store(true, Ordering::SeqCst);
    let (_, v) = rpc(&h, "web", &t, "tools/call", json!({ "name": "search_docs", "arguments": { "q": "x" } })).await;
    assert!(v["error"]["message"].as_str().unwrap().contains("ended the session"), "{v}");
    assert_eq!(text(&tool(&h, "web", &t, "search_docs", json!({ "q": "y" })).await), r#"found "y""#);
    assert_eq!(w.sessions.load(Ordering::SeqCst), 2);

    // A server that cannot be reached: the agent hears so, without the secret in its URL.
    let (_, v) = rpc(&h, "down", &t, "tools/list", json!({})).await;
    let message = v["error"]["message"].as_str().unwrap();
    assert!(message.starts_with("cannot connect to the MCP server down"), "{v}");
    assert!(!message.contains(SECRET), "{message}");
}
