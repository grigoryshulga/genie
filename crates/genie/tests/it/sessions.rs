//! Live agent sessions end to end: the real pi harness (`node_modules/.bin/pi`,
//! RPC mode, the genie-bus extension) against a scripted OpenAI-compatible model
//! served by the test. The model follows instructions found in the mail it gets:
//! `RUN: <command>` runs a shell command; a question carrying `ANSWER=<word>` is
//! answered with `genie mail reply`.
//!
//! Skipped (with a note) when pi is not installed, except on CI.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use genie::config::{Config, RoleModel};
use genie::runtime::AgentKey;
use genie::state::App;
use genie_core::team::{NewMember, NewTeam, SendMail};
use genie_core::{Activity, Actor, CreateInput, MemberState, Role};
use serde_json::{Value, json};

#[derive(Clone, Debug)]
struct Req {
    at: Instant,
    model: String,
    /// The system prompt.
    system: String,
    /// `(role, text)` of every other message in the request.
    messages: Vec<(String, String)>,
}

impl Req {
    fn last(&self) -> &(String, String) {
        self.messages.last().expect("a request has messages")
    }
    fn count(&self, needle: &str) -> usize {
        self.messages.iter().filter(|(_, t)| t.contains(needle)).count()
    }
}

type Log = Arc<Mutex<Vec<Req>>>;

fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// The scripted model: react to the last message only.
fn decide(messages: &[(String, String)]) -> Value {
    let (role, text) = messages.last().cloned().unwrap_or_default();
    if role == "tool" {
        return json!({ "content": "done" });
    }
    // The server-restart note carries no instruction: the agent continues its own work, so the
    // scripted model writes the follow-up file itself.
    if text.contains("The genie server restarted after a crash") {
        return json!({ "tool": "bash", "args": { "command": "echo second > a2" } });
    }
    if let Some(i) = text.rfind("RUN: ") {
        let cmd = text[i + 5..].lines().next().unwrap_or_default().trim().to_string();
        return json!({ "tool": "bash", "args": { "command": cmd } });
    }
    if let Some(i) = text.rfind("MCP: ") {
        let args: Value = serde_json::from_str(text[i + 5..].lines().next().unwrap_or_default().trim()).unwrap_or_default();
        return json!({ "tool": "mcp", "args": args });
    }
    if let (Some(r), Some(a)) = (text.find("genie mail reply "), text.find("ANSWER=")) {
        let id: String = text[r + 17..].chars().take_while(|c| c.is_ascii_digit()).collect();
        let answer: String = text[a + 7..].chars().take_while(|c| c.is_alphanumeric()).collect();
        return json!({ "tool": "bash", "args": { "command": format!("genie mail reply {id} {answer}") } });
    }
    json!({ "content": "ok" })
}

async fn completions(State(log): State<Log>, body: Bytes) -> axum::response::Response {
    let r: Value = serde_json::from_slice(&body).unwrap_or_default();
    let all = r["messages"].as_array().cloned().unwrap_or_default();
    let is_system = |m: &Value| m["role"] == "system" || m["role"] == "developer";
    let system = all.iter().filter(|m| is_system(m)).map(|m| text_of(&m["content"])).collect::<Vec<_>>().join("\n");
    let messages: Vec<(String, String)> = all
        .iter()
        .filter(|m| !is_system(m))
        .map(|m| (m["role"].as_str().unwrap_or_default().to_string(), text_of(&m["content"])))
        .collect();
    let model = r["model"].as_str().unwrap_or_default().to_string();
    // `PROVIDER-DOWN` anywhere in the conversation: the provider refuses every request.
    if messages.iter().any(|(_, t)| t.contains("PROVIDER-DOWN")) {
        let body = json!({ "error": { "message": "usage limit has been reached", "type": "invalid_request_error" } });
        return (axum::http::StatusCode::BAD_REQUEST, [("content-type", "application/json")], body.to_string()).into_response();
    }
    let n = {
        let mut l = log.lock().unwrap();
        l.push(Req { at: Instant::now(), model: model.clone(), system, messages: messages.clone() });
        l.len()
    };
    let out = decide(&messages);
    let chunk = |delta: Value, finish: Value| {
        format!(
            "data: {}\n\n",
            json!({ "id": format!("c{n}"), "object": "chat.completion.chunk", "created": 0, "model": model, "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }] })
        )
    };
    let mut sse = chunk(json!({ "role": "assistant" }), Value::Null);
    if let Some(tool) = out["tool"].as_str() {
        sse.push_str(&chunk(
            json!({ "tool_calls": [{ "index": 0, "id": format!("call_{n}"), "type": "function", "function": { "name": tool, "arguments": out["args"].to_string() } }] }),
            Value::Null,
        ));
        sse.push_str(&chunk(json!({}), json!("tool_calls")));
    } else {
        sse.push_str(&chunk(json!({ "content": out["content"] }), Value::Null));
        sse.push_str(&chunk(json!({}), json!("stop")));
    }
    sse.push_str(&format!(
        "data: {}\n\n",
        json!({ "id": format!("c{n}"), "object": "chat.completion.chunk", "created": 0, "model": model, "choices": [], "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 } })
    ));
    sse.push_str("data: [DONE]\n\n");
    ([("content-type", "text/event-stream")], sse).into_response()
}

fn pi_bin() -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../node_modules/.bin/pi");
    p.exists().then_some(p)
}

struct Live {
    _dir: tempfile::TempDir,
    app: Arc<App>,
    log: Log,
    _stop: tokio::sync::oneshot::Sender<()>,
}

async fn live(pi: PathBuf, tweak: impl FnOnce(&mut Config)) -> Live {
    let dir = tempfile::tempdir().unwrap();
    // The fake model.
    let log: Log = Arc::default();
    let llm = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let llm_port = llm.local_addr().unwrap().port();
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(log.clone());
    tokio::spawn(async move { axum::serve(llm, router).await.unwrap() });
    let agent_dir = dir.path().join("pi-agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    let models: Vec<Value> =
        ["executor", "reviewer", "orchestrator"].iter().map(|m| json!({ "id": m, "contextWindow": 100000, "maxTokens": 4000 })).collect();
    std::fs::write(
        agent_dir.join("models.json"),
        json!({ "providers": { "fake": { "baseUrl": format!("http://127.0.0.1:{llm_port}/v1"), "api": "openai-completions", "apiKey": "x", "models": models } } })
            .to_string(),
    )
    .unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = Config::load(dir.path()).unwrap();
    // Real pi in the agent sandbox: GENIE_TEST_SANDBOX=bwrap (the fake model's files are in the data directory's pi dir).
    cfg.runtime.sandbox.mode = std::env::var("GENIE_TEST_SANDBOX").unwrap_or_else(|_| "off".into());
    cfg.port = listener.local_addr().unwrap().port();
    let g = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    cfg.runtime.mode = "sessions".into();
    cfg.runtime.session_command = vec![
        vec![pi.to_string_lossy().into_owned(), "--mode".into(), "rpc".into()],
        g(&["--session-dir", "{sessionDir}"]),
        g(&["--session-id", "{sessionId}"]),
        g(&["--model", "{model}"]),
        g(&["--append-system-prompt", "{promptFile}"]),
        g(&["--exclude-tools", "{readonlyTools}"]),
        g(&["-e", "{extension}"]),
        g(&["-e", "{guard}"]),
        g(&["--no-skills"]),
    ];
    let genie_dir = PathBuf::from(env!("CARGO_BIN_EXE_genie")).parent().unwrap().to_string_lossy().into_owned();
    for (k, v) in [
        ("PI_CODING_AGENT_DIR", agent_dir.to_string_lossy().into_owned()),
        ("PI_OFFLINE", "1".into()),
        ("PI_SKIP_VERSION_CHECK", "1".into()),
        ("PI_TELEMETRY", "0".into()),
        ("GENIE_BUS_DEBUG", "1".into()),
        ("PATH", format!("{genie_dir}:{}", std::env::var("PATH").unwrap_or_default())),
    ] {
        cfg.runtime.env.insert(k.into(), v);
    }
    for role in ["executor", "reviewer", "orchestrator"] {
        cfg.role_models.insert(role.into(), RoleModel { model: Some(format!("fake/{role}")), thinking: None });
    }
    tweak(&mut cfg);
    let app = App::open(dir.path(), cfg, PathBuf::from("/nonexistent")).unwrap();
    app.create_project("shop", "Shop", None, None, Some("SHOP")).unwrap();
    app.with_server(|db| db.set_autonomy("shop", "manual")).unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    app.with_tracker("shop", |t| {
        t.create(&Actor::new("anna", Role::Human), CreateInput { title: "Export".into(), ..Default::default() })?;
        let m = |n: &str, r: &str| NewMember { name: n.into(), role: r.into(), ..Default::default() };
        t.bus().create(
            "anna",
            "human",
            NewTeam {
                id: "SHOP-1".into(),
                task: "SHOP-1".into(),
                cwd: work.to_string_lossy().into_owned(),
                members: vec![m("bender", "executor"), m("yoda", "reviewer")],
                ..Default::default()
            },
        )
    })
    .unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let a = app.clone();
    tokio::spawn(async move {
        genie::serve_on(a, listener, async {
            let _ = rx.await;
        })
        .await
        .unwrap();
    });
    genie::runtime::start(&app);
    Live { _dir: dir, app, log, _stop: tx }
}

fn mail(app: &App, from: &str, from_role: &str, to: &str, text: &str, level: Option<&str>) {
    let kind = if from_role == "human" { "owner" } else { "message" };
    app.with_tracker("shop", |t| t.bus().send(SendMail { team: "SHOP-1", from, from_role, to, text, level, kind, ..Default::default() }))
        .unwrap();
    app.wake_runtime.notify_one();
}

fn member(name: &str) -> AgentKey {
    AgentKey::Member { project: "shop".into(), team: "SHOP-1".into(), member: name.into() }
}

async fn until<T>(what: &str, secs: u64, mut f: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(secs) {
        if let Some(v) = f() {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out after {secs}s waiting for {what}\n{}", DUMP.lock().unwrap().as_ref().map(|d| d()).unwrap_or_default());
}

/// What the test prints when a wait times out: sessions, their activity and the model's requests.
static DUMP: Mutex<Option<Box<dyn Fn() -> String + Send>>> = Mutex::new(None);

fn install_dump(app: &Arc<App>, log: &Log) {
    let (app, log) = (app.clone(), log.clone());
    *DUMP.lock().unwrap() = Some(Box::new(move || {
        let mut out = Vec::new();
        for s in app.sessions.all() {
            let l = s.live();
            out.push(format!(
                "session {} {} tool={:?}\n  {}",
                s.key.label(),
                l.state,
                l.tool,
                l.recent.iter().cloned().collect::<Vec<_>>().join("\n  ")
            ));
            let dir = app.data.join("runtime").join("shop").join(s.key.label().replace('/', "_"));
            out.push(format!("stderr: {}", std::fs::read_to_string(dir.join("stderr.log")).unwrap_or_default()));
        }
        for r in log.lock().unwrap().iter().rev().take(6).collect::<Vec<_>>().into_iter().rev() {
            out.push(format!("request {} last={:?}", r.model, r.last()));
        }
        let pending = app.with_tracker("shop", |t| t.bus().pending(Some("SHOP-1"), "bender")).unwrap_or_default();
        out.push(format!("bender pending: {:?}", pending.iter().map(|m| (&m.text, &m.level)).collect::<Vec<_>>()));
        let rows: Vec<String> = app
            .with_tracker("shop", |t| {
                let mut stmt =
                    t.conn().prepare("SELECT id, text, delivery, delivered_at FROM mail WHERE recipient = 'bender' ORDER BY id")?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok(format!(
                            "#{} {:?} delivery={:?} delivered={:?}",
                            r.get::<_, i64>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, Option<i64>>(2)?,
                            r.get::<_, Option<String>>(3)?
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .unwrap_or_default();
        out.push(rows.join("\n"));
        out.join("\n")
    }));
}

fn request_with(log: &Log, model: &str, needle: &str) -> Option<Req> {
    log.lock().unwrap().iter().find(|r| r.model == model && r.last().1.contains(needle)).cloned()
}

fn session_state(app: &App, name: &str) -> Option<(String, Option<String>, u32)> {
    app.sessions.get(&member(name)).map(|s| {
        let l = s.live();
        (l.state, l.tool.and_then(|t| t["name"].as_str().map(str::to_string)), l.pid)
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mail_reaches_live_agents_between_steps_and_on_interrupt() {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is not installed: run `npm ci` before `cargo test`");
        eprintln!("skipped: pi is not installed (npm ci)");
        return;
    };
    let l = live(pi, |_| {}).await;
    let (app, log) = (&l.app, &l.log);
    install_dump(app, log);

    // A busy agent gets mail at its next step boundary, not after its run.
    mail(app, "anna", "human", "bender", "RUN: sleep 3; echo slept", None);
    until("bender to run the command", 30, || session_state(app, "bender").filter(|s| s.1.as_deref() == Some("bash"))).await;
    let sent = Instant::now();
    mail(app, "yoda", "reviewer", "bender", "PING-mid: use CSV", None);
    let req = until("the mid-run mail in a model request", 15, || request_with(log, "executor", "PING-mid")).await;
    let waited = req.at - sent;
    eprintln!("latency: mail to a busy agent reached the model {waited:?} after sending (a 3s command was running)");
    assert!(waited < Duration::from_secs(5), "delivered {waited:?} after sending, while a 3s command ran");
    assert!(req.messages.iter().any(|(r, t)| r == "tool" && t.contains("slept")), "delivered right after the step's tool call");
    let runs = app.with_server(|db| db.turns("shop", Some("SHOP-1/bender"), 10)).unwrap();
    assert_eq!(runs.len(), 1, "the mail joined the running run instead of waiting for a new one: {runs:?}");
    until("the mail acknowledged", 10, || {
        app.with_tracker("shop", |t| t.bus().pending(Some("SHOP-1"), "bender")).unwrap().is_empty().then_some(())
    })
    .await;

    // An idle agent is woken at once.
    until("bender idle", 20, || session_state(app, "bender").filter(|s| s.0 == "idle")).await;
    let sent = Instant::now();
    mail(app, "yoda", "reviewer", "bender", "PING-idle", None);
    let req = until("the wake-up request", 10, || request_with(log, "executor", "PING-idle")).await;
    eprintln!("latency: an idle agent saw its mail after {:?}", req.at - sent);
    assert!(req.at - sent < Duration::from_secs(3), "an idle session is woken within seconds: {:?}", req.at - sent);

    // An interrupt stops a long command.
    until("bender idle again", 20, || session_state(app, "bender").filter(|s| s.0 == "idle")).await;
    mail(app, "anna", "human", "bender", "RUN: sleep 60", None);
    until("the long command", 20, || session_state(app, "bender").filter(|s| s.1.as_deref() == Some("bash"))).await;
    let sent = Instant::now();
    mail(app, "orchestrator", "orchestrator", "bender", "STOP-NOW: switch to the report", Some("interrupt"));
    let req = until("the interrupt request", 20, || request_with(log, "executor", "STOP-NOW")).await;
    eprintln!("latency: an interrupt stopped a 60s command and reached the model after {:?}", req.at - sent);
    assert!(req.at - sent < Duration::from_secs(8), "the interrupt stopped a 60s command: {:?}", req.at - sent);
    assert!(req.last().1.contains("INTERRUPT"), "{}", req.last().1);

    // A crashed session restarts with the same conversation; nothing is injected twice.
    until("bender idle before the crash", 20, || session_state(app, "bender").filter(|s| s.0 == "idle")).await;
    let pid = session_state(app, "bender").unwrap().2;
    std::process::Command::new("kill").arg("-KILL").arg(pid.to_string()).status().unwrap();
    until("the crash noticed", 20, || session_state(app, "bender").is_none_or(|s| s.2 != pid).then_some(())).await;
    mail(app, "yoda", "reviewer", "bender", "PING-after-crash", None);
    let sent = Instant::now();
    let req = until("mail after the crash", 60, || request_with(log, "executor", "PING-after-crash")).await;
    eprintln!("latency: after a crash, a restarted session saw new mail after {:?}", req.at - sent);
    assert_eq!(req.count("PING-mid"), 1, "the resumed conversation holds earlier mail exactly once");
    assert_eq!(req.count("PING-idle"), 1);

    // Ask and wait: bender asks yoda; yoda answers; bender's command returns the answer.
    until("bender idle before asking", 30, || session_state(app, "bender").filter(|s| s.0 == "idle")).await;
    let sent = Instant::now();
    mail(app, "anna", "human", "bender", "RUN: genie mail ask yoda 'Which export format? ANSWER=CSV'", None);
    let req = until("the answer in bender's context", 60, || {
        log.lock().unwrap().iter().find(|r| r.model == "executor" && r.last().0 == "tool" && r.last().1.contains("yoda answered")).cloned()
    })
    .await;
    eprintln!("latency: an ask was answered by a teammate (whose session had to start) after {:?}", req.at - sent);
    assert!(req.last().1.contains("CSV"), "{}", req.last().1);
    let pending = app.with_tracker("shop", |t| t.bus().pending(Some("SHOP-1"), "bender")).unwrap();
    assert!(!pending.iter().any(|m| m.text == "CSV"), "the answer went to the waiting asker, not into its mailbox");

    // The board and peek over HTTP (local mode: the loopback is trusted).
    let base = format!("http://127.0.0.1:{}/api", app.cfg.port);
    let http = reqwest::Client::new();
    let get = |path: String| {
        let (http, base) = (http.clone(), base.clone());
        async move { http.get(format!("{base}{path}")).send().await.unwrap().json::<Value>().await.unwrap() }
    };
    let board = get("/agents".into()).await;
    let bender = board["agents"].as_array().unwrap().iter().find(|a| a["agent"] == "SHOP-1/bender").cloned().unwrap();
    assert!(bender["session"]["state"].is_string(), "{bender}");
    assert!(!bender["session"]["recent"].as_array().unwrap().is_empty());
    until("bender idle before peeking", 20, || session_state(app, "bender").filter(|s| s.0 == "idle")).await;
    let peek = get("/agents/SHOP-1/bender/peek?deep=1".into()).await;
    let conversation = peek["conversation"].as_array().cloned().unwrap_or_default();
    assert!(conversation.iter().any(|m| m["text"].as_str().unwrap_or_default().contains("yoda answered")), "{peek}");
    // The web chat reads structured parts: delivered mail names its messages, tool calls come with a result.
    assert!(
        conversation.iter().any(|m| m["role"] == "custom:genie-mail" && m["mailIds"].as_array().is_some_and(|ids| !ids.is_empty())),
        "{peek}"
    );
    assert!(conversation.iter().all(|m| m["parts"].is_array() && m["at"].is_string()), "{peek}");
    let few = get("/agents/SHOP-1/bender/peek?deep=1&limit=2".into()).await;
    assert_eq!(few["conversation"].as_array().map(Vec::len), Some(2), "{few}");

    // Only the orchestrator and people interrupt.
    let token = app
        .with_server(|db| db.create_agent_token("shop", Role::Reviewer, "yoda", Some("SHOP-1"), None, chrono::Duration::hours(1)))
        .unwrap();
    let res = http
        .post(format!("{base}/teams/SHOP-1/mail"))
        .bearer_auth(&token)
        .json(&json!({ "to": "bender", "text": "stop", "level": "interrupt" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 403);

    // A paused agent's session stops and its mail waits; resumed, it gets the mail.
    let post = |path: &str| http.post(format!("{base}{path}")).header("x-genie", "1").json(&json!({})).send();
    assert!(post("/agents/SHOP-1/bender/pause").await.unwrap().status().is_success());
    until("the paused session to stop", 20, || session_state(app, "bender").is_none().then_some(())).await;
    mail(app, "yoda", "reviewer", "bender", "PING-paused", None);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(session_state(app, "bender").is_none(), "a paused agent is not started");
    assert!(request_with(log, "executor", "PING-paused").is_none());
    assert!(post("/agents/SHOP-1/bender/resume").await.unwrap().status().is_success());
    until("mail after resuming", 30, || request_with(log, "executor", "PING-paused")).await;

    // The guard reported every model response: the chat's tokens, on its task.
    let rows = until("usage reported", 10, || {
        let rows = app.with_tracker("shop", |t| t.usage_of_agent("SHOP-1/bender")).unwrap();
        (!rows.is_empty()).then_some(rows)
    })
    .await;
    assert!(rows.iter().all(|r| r.task.as_deref() == Some("SHOP-1") && r.model.ends_with("executor")), "{rows:?}");
    let calls: u64 = rows.iter().map(|r| r.calls).sum();
    assert_eq!(rows.iter().map(|r| r.tokens.input).sum::<u64>(), 10 * calls, "10 prompt tokens a response: {rows:?}");
    let v: Value = http.get(format!("{base}/agents/SHOP-1/bender/usage")).send().await.unwrap().json().await.unwrap();
    assert_eq!(v["spend"]["tokens"]["output"].as_u64(), v["spend"]["calls"].as_u64().map(|n| 5 * n), "{v}");
    assert_eq!(v["spend"]["unpricedTokens"].as_u64(), v["spend"]["calls"].as_u64().map(|n| 15 * n), "no price set: {v}");
}

fn orchestrator_mail(app: &App, needle: &str) -> Option<()> {
    let mail = app.with_tracker("shop", |t| t.bus().pending(None, "orchestrator")).unwrap();
    mail.iter().any(|m| m.text.contains(needle)).then_some(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn watchdogs_stop_silent_steps_and_report_loops() {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is not installed: run `npm ci` before `cargo test`");
        return;
    };
    let l = live(pi, |cfg| cfg.runtime.turn_timeout_secs = 3).await;
    let (app, log) = (&l.app, &l.log);

    // A step with no sign of life for turnTimeoutSecs is aborted and the orchestrator is told.
    mail(app, "anna", "human", "bender", "RUN: sleep 120", None);
    until("the silent command", 30, || session_state(app, "bender").filter(|s| s.1.as_deref() == Some("bash"))).await;
    until("the watchdog report", 20, || orchestrator_mail(app, "no sign of life")).await;
    until("the stuck step to be aborted", 20, || session_state(app, "bender").filter(|s| s.0 == "idle")).await;
    until("the agent told why its step stopped", 20, || request_with(log, "executor", "showed no sign of life")).await;
    let runs = app.with_server(|db| db.turns("shop", Some("SHOP-1/bender"), 10)).unwrap();
    assert!(runs.iter().all(|r| r.status == "succeeded"), "an abort we asked for is not a failed run: {runs:?}");

    // The same call five times in a row: the orchestrator hears the agent may loop.
    for i in 0..5 {
        mail(app, "anna", "human", "bender", "RUN: echo again", None);
        until("the repeated call", 30, || {
            let n = log
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.model == "executor" && r.last().0 == "tool" && r.last().1.contains("again"))
                .count();
            (n > i).then_some(())
        })
        .await;
        until("bender idle", 20, || session_state(app, "bender").filter(|s| s.0 == "idle")).await;
    }
    until("the loop report", 10, || orchestrator_mail(app, "stuck in a loop")).await;
}

fn write(path: &std::path::Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The tool result the model got right after `needle` was sent.
async fn tool_result(log: &Log, needle: &str) -> String {
    until(&format!("the tool result after {needle}"), 30, || {
        let l = log.lock().unwrap();
        let sent = l.iter().position(|r| r.model == "executor" && r.last().1.contains(needle))?;
        l[sent + 1..].iter().find(|r| r.model == "executor" && r.last().0 == "tool").map(|r| r.last().1.clone())
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_role_gets_its_skills_and_the_guard_keeps_it_within_its_grants() {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is not installed: run `npm ci` before `cargo test`");
        return;
    };
    let fake_mcp = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-mcp.ts");
    let pi_path = pi.to_string_lossy().into_owned();
    let l = live(pi, |cfg| {
        // The default session command (guard, skills, MCP config) on the test's pi, plus a
        // stand-in for pi-mcp-adapter's `mcp` tool. Without the adapter pi still accepts --mcp-config.
        let mut cmd = genie::config::RuntimeConfig::default().session_command;
        cmd[0][0] = pi_path;
        cmd.push(vec!["-e".into(), fake_mcp.to_string_lossy().into_owned()]);
        cfg.runtime.session_command = cmd;
    })
    .await;
    let (app, log) = (&l.app, &l.log);
    install_dump(app, log);
    let data = app.data.clone();

    // In this server the executor role has a skill, a denied command and one MCP connection.
    write(&data.join("agents/executor.md"), "---\nskills: [owasp]\ndenyCommands: [\"git push*\"]\nmcp: [docs]\n---\n");
    write(&data.join("skills/owasp/SKILL.md"), "---\nname: owasp\ndescription: OWASP checks.\n---\nx\n");
    write(
        &data.join("pi-agent/skills/user-wide/SKILL.md"),
        "---\nname: user-wide\ndescription: Installed for the machine's user.\n---\nx\n",
    );
    write(&data.join("work/.agents/skills/house-style/SKILL.md"), "---\nname: house-style\ndescription: The repository's style.\n---\nx\n");
    write(
        &data.join("mcp.json"),
        &json!({ "mcpServers": { "docs": { "command": "docs-mcp" }, "secret": { "url": "https://secret.example/mcp" } } }).to_string(),
    );
    app.reload_agents();

    // Its skills and the repository's, and no others; its MCP connection in the prompt —
    // from the first request of the fresh session on.
    mail(app, "anna", "human", "bender", "RUN: echo ready", None);
    let req = until("bender's first request", 60, || request_with(log, "executor", "echo ready")).await;
    assert!(req.system.contains("You are the **executor** of a focus team"), "the role's prompt:\n{}", req.system);
    assert!(req.system.contains("<name>owasp</name>"), "the role's skill:\n{}", req.system);
    assert!(req.system.contains("<name>house-style</name>"), "the repository's skill");
    assert!(!req.system.contains("user-wide"), "no other skills");
    assert!(req.system.contains("## MCP connections") && req.system.contains("`docs`"), "{}", req.system);
    // The command table of the role, from the catalog of operations.
    assert!(
        req.system.contains(
            "| `genie_task` status | `genie task status <STATUS> [--task …] [--note …]` (your role may set: in_progress, review) |"
        ),
        "the command table:\n{}",
        req.system
    );
    // (`genie pr open --title` is the executor's own: it hands its work over.)
    assert!(!req.system.contains("genie agent ") && !req.system.contains("genie task create"), "the catalog's commands, those of the role");

    // A denied command is blocked, the rest of the shell works.
    mail(app, "anna", "human", "bender", "RUN: echo one && git push origin main", None);
    let out = tool_result(log, "git push origin main").await;
    assert!(out.contains("may not run `git push*`"), "{out}");

    // MCP: the granted connection passes, another one and installing servers do not.
    mail(app, "anna", "human", "bender", r#"MCP: {"server": "docs", "tool": "search", "args": {"q": "export"}}"#, None);
    assert!(tool_result(log, r#""tool": "search""#).await.contains("FAKE-MCP"), "a granted connection");
    mail(app, "anna", "human", "bender", r#"MCP: {"server": "secret", "tool": "dump"}"#, None);
    let out = tool_result(log, r#""tool": "dump""#).await;
    assert!(out.contains("secret is not granted") && !out.contains("FAKE-MCP"), "{out}");
    mail(app, "anna", "human", "bender", r#"MCP: {"action": "install", "url": "https://evil.example/mcp"}"#, None);
    assert!(tool_result(log, "evil.example").await.contains("do not install MCP servers"));

    // A rule changed while the agent runs applies at its next tool call, without a restart.
    let pid = session_state(app, "bender").unwrap().2;
    write(&data.join("agents/executor.md"), "---\nskills: [owasp]\ndenyCommands: [\"git push*\", \"curl *\"]\nmcp: [docs]\n---\n");
    app.reload_agents();
    mail(app, "anna", "human", "bender", "RUN: curl https://example.com", None);
    let out = tool_result(log, "curl https://example.com").await;
    assert!(out.contains("may not run `curl *`"), "{out}");
    assert_eq!(session_state(app, "bender").unwrap().2, pid, "the same session");
}

/// pi runs under a V8 heap cap added to the operator's own `NODE_OPTIONS`; what pi runs does not inherit it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_runs_under_a_heap_cap_its_commands_do_not_inherit() {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is not installed: run `npm ci` before `cargo test`");
        return;
    };
    let l = live(pi, |cfg| {
        cfg.runtime.env.insert("NODE_OPTIONS".into(), "--no-warnings".into());
    })
    .await;
    let (app, log) = (&l.app, &l.log);
    mail(app, "anna", "human", "bender", "RUN: echo CAP=[$NODE_OPTIONS][$GENIE_NODE_HEAP_MB]", None);
    let out = tool_result(log, "CAP=").await;
    assert!(out.contains("CAP=[--no-warnings][]"), "the command sees the operator's options only: {out}");
    let pid = session_state(app, "bender").unwrap().2;
    let environ = std::fs::read(format!("/proc/{pid}/environ")).unwrap();
    let environ = String::from_utf8_lossy(&environ);
    assert!(environ.contains("NODE_OPTIONS=--no-warnings --max-old-space-size=2048"), "pi itself got the cap");
    // What a bare session weighs, for sizing `nodeHeapMb` and `maxSessions` (docs/platform/docker.md).
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    eprintln!("pi session: {}", status.lines().filter(|l| l.starts_with("VmRSS") || l.starts_with("VmHWM")).collect::<Vec<_>>().join(" "));
}

/// The orchestrator console: `genie orchestrate` runs pi (here in RPC mode, so
/// the test sees its UI requests) with genie-bus in console mode — mail for the
/// orchestrator reaches the idle session without anyone waking it — and the
/// console extension's card of teams; when pi ends, the console is given back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn at_the_console_mail_reaches_the_idle_session_by_itself() {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is not installed: run `npm ci` before `cargo test`");
        eprintln!("skipped: pi is not installed (npm ci)");
        return;
    };
    let l = live(pi.clone(), |_| {}).await;
    let (app, log) = (&l.app, &l.log);
    install_dump(app, log);
    let u = app.with_server(|db| db.create_user("anna", "Anna", None, Some("password-1"), false)).unwrap();
    app.with_server(|db| db.set_membership("shop", u.id, genie_core::server_db::ProjectRole::Owner)).unwrap();
    let anna = app.with_server(|db| db.create_user_token(u.id, "cli")).unwrap();

    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_genie"))
        .arg("--data")
        .arg(&app.data)
        .args(["--project", "shop", "orchestrate", "--pi"])
        .arg(&pi)
        .args(["--", "--mode", "rpc", "--no-skills"])
        .env("GENIE_URL", format!("http://127.0.0.1:{}", app.cfg.port))
        .env("GENIE_TOKEN", &anna)
        .env("PI_CODING_AGENT_DIR", l._dir.path().join("pi-agent"))
        .env("PI_OFFLINE", "1")
        .env("PI_SKIP_VERSION_CHECK", "1")
        .env("PI_TELEMETRY", "0")
        .env("GENIE_BUS_POLL_MS", "300")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let out: Arc<Mutex<Vec<String>>> = Arc::default();
    let (stdout, lines) = (child.stdout.take().unwrap(), out.clone());
    tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut r = tokio::io::BufReader::new(stdout).lines();
        while let Ok(Some(line)) = r.next_line().await {
            lines.lock().unwrap().push(line);
        }
    });
    until("the console taken", 30, || app.with_server(|db| db.console("shop")).unwrap()).await;

    // A person's new task tells the orchestrator; nobody sends the session a command.
    app.with_tracker("shop", |t| {
        t.create(
            &Actor::new("pm", Role::Human),
            CreateInput { title: "CSV export".into(), status: Some(genie_core::Status::Inbox), ..Default::default() },
        )
    })
    .unwrap();
    let req = until("the mail in a request of the console's model", 30, || {
        log.lock().unwrap().iter().find(|r| r.model == "orchestrator" && r.count("CSV export") > 0).cloned()
    })
    .await;
    assert!(req.system.contains("## The console") && req.system.contains("@anna"), "the console's prompt:\n{}", req.system);

    // The card of the project's teams, drawn by the console extension.
    let card = until("the card of teams", 30, || {
        out.lock().unwrap().iter().find(|l| l.contains("setWidget") && l.contains("genie · shop")).cloned()
    })
    .await;
    assert!(card.contains("console: @anna") && card.contains("SHOP-1: bender (executor)"), "{card}");

    // `/genie` shows the task board of the command line above the editor.
    {
        use tokio::io::AsyncWriteExt;
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(format!("{}\n", json!({ "type": "prompt", "message": "/genie" })).as_bytes()).await.unwrap();
        stdin.flush().await.unwrap();
    }
    let board =
        until("the task board", 30, || out.lock().unwrap().iter().find(|l| l.contains("setWidget") && l.contains("genie-board")).cloned())
            .await;
    assert!(board.contains("CSV export"), "{board}");

    // pi ends: the console is given back.
    drop(child.stdin.take());
    let status = tokio::time::timeout(Duration::from_secs(30), child.wait()).await.expect("pi ends").unwrap();
    assert!(status.success(), "{status:?}");
    assert!(app.with_server(|db| db.console("shop")).unwrap().is_none(), "the console is given back");
}

/// A provider that refuses every request (a usage limit): after `maxAttempts` the agent stays
/// in `error` — also once its session is stopped — with the reason on the board, and the
/// orchestrator is told; a restart lets it work again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_whose_provider_refuses_stays_in_error_with_the_reason() {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is not installed: run `npm ci` before `cargo test`");
        return;
    };
    let l = live(pi, |cfg| cfg.runtime.max_attempts = 2).await;
    let app = &l.app;

    mail(app, "anna", "human", "bender", "PROVIDER-DOWN please work", None);
    until("the orchestrator hears the agent gave up", 60, || orchestrator_mail(app, "failed 2 runs in a row")).await;
    until("the session to be stopped", 30, || app.sessions.get(&member("bender")).is_none().then_some(())).await;

    let bender = || {
        let team = app.with_tracker("shop", |t| t.bus().get("SHOP-1")).unwrap();
        team.members.into_iter().find(|m| m.name == "bender").unwrap()
    };
    let m = bender();
    assert_eq!(m.state, MemberState::Error, "a stopped session does not clear the error: {m:?}");
    assert!(m.status.contains("usage limit has been reached"), "the board says why: {}", m.status);

    genie::runtime::restart_member(app, "shop", "SHOP-1", "bender").unwrap();
    assert_eq!(bender().state, MemberState::Active, "a restart lets the agent work again");
}

/// G-132: the server is killed mid-step, so the session process goes with it. Its mail was
/// acknowledged (it is in the conversation), nothing is pending — without a note the agent would
/// sit there for ever. `runtime::recover` must tell it to continue, and the note must be what
/// starts the new run: no letter from a person, no duplicate of the acknowledged one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_server_crash_mid_turn_does_not_leave_the_agent_stuck() {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is not installed: run `npm ci` before `cargo test`");
        eprintln!("skipped: pi is not installed (npm ci)");
        return;
    };
    let l = live(pi, |_| {}).await;
    let (app, log) = (&l.app, &l.log);
    install_dump(app, log);
    let work = app.data.join("work");

    // A first step runs and its letter is acknowledged while the turn runs.
    mail(app, "anna", "human", "bender", "RUN: echo first > a1", None);
    until("the step and its letter to settle", 60, || {
        let idle = session_state(app, "bender").is_some_and(|s| s.0 == "idle");
        let empty = app.with_tracker("shop", |t| t.bus().pending(Some("SHOP-1"), "bender")).unwrap().is_empty();
        (idle && empty).then_some(())
    })
    .await;
    assert!(work.join("a1").exists(), "the first step ran");

    // The server dies: its process table goes, the session with it; the DB keeps the turn
    // `running` and the letter `delivered`, and the member stays `working`.
    let pid = session_state(app, "bender").map(|s| s.2);
    let activity = || {
        app.with_tracker("shop", |t| Ok(t.bus().get("SHOP-1")?.members.into_iter().find(|m| m.name == "bender").unwrap().activity)).unwrap()
    };
    genie::sessions::stop_all(app);
    // The shutdown path writes the activity too: wait for it, so the state fabricated below is the
    // last word — exactly what a kill -9 leaves.
    until("the session and its shutdown to settle", 30, || {
        (app.sessions.get(&member("bender")).is_none() && activity() == Activity::Idle).then_some(())
    })
    .await;
    let turn = app
        .with_server(|db| {
            let t = db.start_turn("shop", "SHOP-1/bender", Some("SHOP-1"), Some("bender"), None)?;
            if let Some(pid) = pid {
                db.set_turn_pid(t, pid)?;
            }
            Ok::<_, genie_core::GenieError>(t)
        })
        .unwrap();
    app.with_tracker("shop", |t| t.bus().member_working("SHOP-1", "bender", json!({ "kind": "session" }))).unwrap();

    // A restart: the turn is interrupted, the stray process is gone, and the agent is told to go on.
    let crashed = Instant::now();
    genie::runtime::recover(app).unwrap();
    genie::sessions::recover(app);
    assert_eq!(app.with_server(|db| db.turn(turn)).unwrap().status, "interrupted");
    assert_eq!(activity(), Activity::Idle, "the board must not show a busy agent without a process");

    // Nobody writes to the agent: the note alone starts the run, and the letter it had
    // acknowledged is in the resumed conversation exactly once.
    let req =
        until("the restart note in a model request", 120, || request_with(log, "executor", "The genie server restarted after a crash"))
            .await;
    eprintln!("latency: after a server crash mid-turn, the agent was told to continue after {:?}", req.at - crashed);
    assert_eq!(req.count("RUN: echo first"), 1, "the acknowledged letter is not injected twice");
    until("the agent to continue its own work", 60, || work.join("a2").exists().then_some(())).await;
    eprintln!("a server crash mid-turn: the agent resumed by itself and wrote a2");
}
