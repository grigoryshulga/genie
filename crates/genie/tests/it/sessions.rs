//! Live agent sessions end to end: the real pi harness (`pi` from PATH, Pi-Bolt on CI,
//! RPC mode, the genie-bus extension) against a scripted OpenAI-compatible model
//! served by the test. The model follows instructions found in the mail it gets:
//! `RUN: <command>` runs a shell command; `MCPCALL: {"tool": …, "args": …}` calls the
//! named tool and `CODEMODE: <script>` runs a codemode script; a question carrying
//! `ANSWER=<word>` is answered with `genie mail reply`.
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
use genie::config::Config;
use genie::runtime::AgentKey;
use genie::state::App;
use genie_core::server_db::ProjectRole;
use genie_core::team::{CHAT_MAIL, NewMember, NewTeam, ORCHESTRATOR, SendMail};
use genie_core::{Activity, Actor, CreateInput, MemberState, Role};
use serde_json::{Value, json};

#[derive(Clone, Debug)]
struct Req {
    at: Instant,
    model: String,
    /// The system prompt.
    system: String,
    /// The tools declared in the request.
    tools: Vec<String>,
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

#[derive(Clone)]
struct ReplyGate {
    needle: &'static str,
    release: Arc<tokio::sync::Semaphore>,
}

#[derive(Clone)]
struct Model {
    log: Log,
    reply_gate: Option<ReplyGate>,
}

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
    if let Some(i) = text.rfind("MCPCALL: ") {
        let call: Value = serde_json::from_str(text[i + 9..].lines().next().unwrap_or_default().trim()).unwrap_or_default();
        return json!({ "tool": call["tool"], "args": call["args"] });
    }
    if let Some(i) = text.rfind("CODEMODE: ") {
        let code = text[i + 10..].lines().next().unwrap_or_default().trim().to_string();
        return json!({ "tool": "codemode", "args": { "code": code } });
    }
    if let (Some(r), Some(a)) = (text.find("genie mail reply "), text.find("ANSWER=")) {
        let id: String = text[r + 17..].chars().take_while(|c| c.is_ascii_digit()).collect();
        let answer: String = text[a + 7..].chars().take_while(|c| c.is_alphanumeric()).collect();
        return json!({ "tool": "bash", "args": { "command": format!("genie mail reply {id} {answer}") } });
    }
    json!({ "content": "ok" })
}

async fn completions(State(state): State<Model>, body: Bytes) -> axum::response::Response {
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
    let tools: Vec<String> =
        r["tools"].as_array().into_iter().flatten().filter_map(|t| t["function"]["name"].as_str().map(str::to_string)).collect();
    // `PROVIDER-DOWN` anywhere in the conversation: the provider refuses every request.
    if messages.iter().any(|(_, t)| t.contains("PROVIDER-DOWN")) {
        let body = json!({ "error": { "message": "usage limit has been reached", "type": "invalid_request_error" } });
        return (axum::http::StatusCode::BAD_REQUEST, [("content-type", "application/json")], body.to_string()).into_response();
    }
    let n = {
        let mut l = state.log.lock().unwrap();
        l.push(Req { at: Instant::now(), model: model.clone(), system, tools, messages: messages.clone() });
        l.len()
    };
    if let Some(gate) = &state.reply_gate
        && messages.last().is_some_and(|(_, text)| text.contains(gate.needle))
    {
        // Hold a real provider response at a chosen boundary without a timed sleep.
        gate.release.acquire().await.unwrap().forget();
    }
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
    genie::doctor::pi_program(&Config::default())
}

struct Live {
    _dir: tempfile::TempDir,
    app: Arc<App>,
    log: Log,
    _stop: tokio::sync::oneshot::Sender<()>,
}

async fn live(pi: PathBuf, tweak: impl FnOnce(&mut Config)) -> Live {
    live_with_clock(pi, tweak, Arc::new(genie::clock::SystemClock)).await
}

async fn live_with_clock(pi: PathBuf, tweak: impl FnOnce(&mut Config), clock: Arc<dyn genie::clock::Clock>) -> Live {
    live_with_reply_gate(pi, tweak, clock, None).await
}

async fn live_with_reply_gate(
    pi: PathBuf,
    tweak: impl FnOnce(&mut Config),
    clock: Arc<dyn genie::clock::Clock>,
    reply_gate: Option<ReplyGate>,
) -> Live {
    let dir = tempfile::tempdir().unwrap();
    // The fake model.
    let log: Log = Arc::default();
    let llm = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let llm_port = llm.local_addr().unwrap().port();
    let router = Router::new().route("/v1/chat/completions", post(completions)).with_state(Model { log: log.clone(), reply_gate });
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
    cfg.vault.commit = Some(false);
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
    std::fs::create_dir_all(dir.path().join("agents")).unwrap();
    for role in ["executor", "reviewer", "orchestrator"] {
        // PD24: the model lives on the role; there are no class defaults.
        std::fs::write(dir.path().join(format!("agents/{role}.md")), format!("---\nmodel: fake/{role}\n---\n")).unwrap();
    }
    tweak(&mut cfg);
    let app = App::open_with_clock(dir.path(), cfg, PathBuf::from("/nonexistent"), clock).unwrap();
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

/// A member that has finished a run and is idle again. A fresh session is idle too, before its first wake-up.
async fn settled_run(app: &App, name: &str) {
    until(&format!("{name} to settle idle after a run"), 60, || {
        app.sessions.get(&member(name)).filter(|s| s.live().runs >= 1 && s.live().state == "idle").map(|_| ())
    })
    .await;
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
        tokio::time::sleep(Duration::from_millis(10)).await;
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
        assert!(std::env::var("CI").is_err(), "pi is not on PATH: install Pi-Bolt before `cargo test`");
        eprintln!("skipped: pi is not on PATH (install Pi-Bolt)");
        return;
    };
    let clock = Arc::new(crate::common::ManualClock::default());
    let l = live_with_clock(pi, |_| {}, clock.clone()).await;
    let (app, log) = (&l.app, &l.log);
    install_dump(app, log);

    // A busy agent gets mail at its next step boundary, not after its run.
    // Hold the real bash step until the mail is queued, rather than guessing how long it needs.
    let release = app.data.join("work/step-release");
    mail(app, "anna", "human", "bender", "RUN: while [ ! -e step-release ]; do sleep 0.01; done; echo slept", None);
    until("bender to run the command", 30, || session_state(app, "bender").filter(|s| s.1.as_deref() == Some("bash"))).await;
    let sent = Instant::now();
    mail(app, "yoda", "reviewer", "bender", "PING-mid: use CSV", None);
    assert!(request_with(log, "executor", "PING-mid").is_none(), "mail waits for the busy step's boundary");
    std::fs::write(release, "go").unwrap();
    let req = until("the mid-run mail in a model request", 15, || request_with(log, "executor", "PING-mid")).await;
    let waited = req.at - sent;
    eprintln!("latency: mail to a busy agent reached the model {waited:?} after sending and releasing its command");
    assert!(waited < Duration::from_secs(5), "delivered {waited:?} after sending and releasing the busy step");
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
    until("the crash backoff recorded", 20, || (app.attempts.failures(&member("bender")) == 1).then_some(())).await;
    mail(app, "yoda", "reviewer", "bender", "PING-after-crash", None);
    genie::runtime::tick(app).await.unwrap();
    assert!(session_state(app, "bender").is_none(), "the crashed agent waits for its retry deadline");
    clock.advance(Duration::from_secs(1));
    genie::runtime::tick(app).await.unwrap();
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
    // Without a session the chat still shows the conversation, read from its file.
    let saved = get("/agents/SHOP-1/bender/peek?deep=1".into()).await;
    assert!(saved["session"].is_null(), "{saved}");
    let conversation = saved["conversation"].as_array().cloned().unwrap_or_default();
    assert!(conversation.iter().any(|m| m["text"].as_str().unwrap_or_default().contains("yoda answered")), "{saved}");
    assert!(conversation.iter().any(|m| m["role"] == "custom:genie-mail"), "{saved}");
    mail(app, "yoda", "reviewer", "bender", "PING-paused", None);
    genie::runtime::tick(app).await.unwrap();
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
        assert!(std::env::var("CI").is_err(), "pi is not on PATH: install Pi-Bolt before `cargo test`");
        return;
    };
    let clock = Arc::new(crate::common::ManualClock::default());
    let reply = Arc::new(tokio::sync::Semaphore::new(0));
    let gate = ReplyGate { needle: "showed no sign of life", release: reply.clone() };
    let l = live_with_reply_gate(pi, |cfg| cfg.runtime.turn_timeout_secs = 3, clock.clone(), Some(gate)).await;
    let (app, log) = (&l.app, &l.log);

    // A step with no sign of life for turnTimeoutSecs is aborted and the orchestrator is told.
    mail(app, "anna", "human", "bender", "RUN: sleep 120", None);
    until("the silent command", 30, || session_state(app, "bender").filter(|s| s.1.as_deref() == Some("bash"))).await;
    // Drain startup/prompt responses before measuring silence: every RPC event is a heartbeat,
    // and seeing tool_execution_start alone does not mean its trailing responses were read.
    let session = app.sessions.get(&member("bender")).unwrap();
    let state = session.request(json!({ "type": "get_state" }), Duration::from_secs(5)).await.expect("the RPC barrier answers");
    assert_eq!(state["success"], true, "{state}");
    let stalled_turn = app
        .with_server(|db| db.turns("shop", Some("SHOP-1/bender"), 10))
        .unwrap()
        .into_iter()
        .find(|r| r.status == "running")
        .expect("the silent command has a running turn")
        .id;
    clock.advance(Duration::from_millis(2999));
    genie::runtime::tick(app).await.unwrap();
    assert!(orchestrator_mail(app, "no sign of life").is_none(), "the watchdog does not fire before its deadline");
    assert_eq!(session_state(app, "bender").unwrap().1.as_deref(), Some("bash"));
    clock.advance(Duration::from_millis(1));
    genie::runtime::tick(app).await.unwrap();
    until("the watchdog report", 20, || orchestrator_mail(app, "no sign of life")).await;
    until("the agent told why its step stopped", 20, || request_with(log, "executor", "showed no sign of life")).await;
    let runs = until("the watchdog continuation to start", 20, || {
        let runs = app.with_server(|db| db.turns("shop", Some("SHOP-1/bender"), 10)).unwrap();
        (runs.len() == 2 && runs.iter().any(|r| r.status == "running")).then_some(runs)
    })
    .await;
    // The notification starts another turn. Hold its provider response so this check
    // always exercises the CI interleaving: the abort is finished, its follow-up is not.
    let aborted = runs.iter().find(|r| r.id == stalled_turn).expect("the aborted turn remains recorded");
    assert_eq!(aborted.status, "succeeded", "an abort we asked for is not a failed run: {runs:?}");
    reply.add_permits(1);
    let runs = until("the watchdog continuation to finish", 20, || {
        let runs = app.with_server(|db| db.turns("shop", Some("SHOP-1/bender"), 10)).unwrap();
        // `idle` is set before the blocking finish_turn write; wait for both signals.
        (runs.len() == 2 && runs.iter().all(|r| r.status != "running") && session_state(app, "bender").is_some_and(|s| s.0 == "idle"))
            .then_some(runs)
    })
    .await;
    assert!(runs.iter().all(|r| r.status == "succeeded"), "the abort and its continuation both succeed: {runs:?}");

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

/// The tool result `model` got right after `needle` was sent.
async fn tool_result_of(log: &Log, model: &str, needle: &str) -> String {
    until(&format!("the tool result after {needle}"), 30, || {
        let l = log.lock().unwrap();
        let sent = l.iter().position(|r| r.model == model && r.last().1.contains(needle))?;
        l[sent + 1..].iter().find(|r| r.model == model && r.last().0 == "tool").map(|r| r.last().1.clone())
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_role_gets_its_skills_and_the_guard_keeps_it_within_its_grants() {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is not on PATH: install Pi-Bolt before `cargo test`");
        return;
    };
    let pi_path = pi.to_string_lossy().into_owned();
    let l = live(pi, |cfg| {
        // The default session command (guard, skills, MCP, trust) on the test's pi.
        let mut cmd = genie::config::RuntimeConfig::default().session_command;
        cmd[0][0] = pi_path;
        cfg.runtime.session_command = cmd;
    })
    .await;
    let (app, log) = (&l.app, &l.log);
    install_dump(app, log);
    let data = app.data.clone();
    let server = format!("{}/tests/fixtures/fake-mcp-server.mjs", env!("CARGO_MANIFEST_DIR"));
    let stdio = |extra: Value| {
        let mut e = json!({ "command": "node", "args": [server] });
        e.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        e
    };

    // In this server the executor role has a skill, a denied command and MCP connections: some tools
    // of `tracker`, all of `wide`. The reviewer (no edit, no write) gets `cm`, which the admin left to
    // codemode.
    write(
        &data.join("agents/executor.md"),
        "---\nmodel: fake/executor\nskills: [owasp]\ndenyCommands: [\"git push*\"]\nmcp: [\"tracker:get_*\", \"tracker:list_issues\", wide]\n---\n",
    );
    write(&data.join("agents/reviewer.md"), "---\nmodel: fake/reviewer\ndenyCommands: [\"git push*\"]\nmcp: [cm]\n---\n");
    write(&data.join("skills/owasp/SKILL.md"), "---\nname: owasp\ndescription: OWASP checks.\n---\nx\n");
    write(
        &data.join("pi-agent/skills/user-wide/SKILL.md"),
        "---\nname: user-wide\ndescription: Installed for the machine's user.\n---\nx\n",
    );
    write(&data.join("work/.agents/skills/house-style/SKILL.md"), "---\nname: house-style\ndescription: The repository's style.\n---\nx\n");
    write(
        &data.join("mcp.json"),
        &json!({ "mcpServers": {
            "tracker": stdio(json!({ "description": "The issue tracker" })),
            "wide": stdio(json!({ "description": "The whole tracker" })),
            "cm": stdio(json!({ "exposure": "codemode" })),
            "secret": { "url": "https://secret.example/mcp" }
        } })
        .to_string(),
    );
    // What the machine's user and the repository bring along: MCP servers of their own, an extension
    // of the repository — which pi's trust file even trusts. None of it may reach an agent.
    let work = data.join("work").canonicalize().unwrap();
    write(&data.join("pi-agent/mcp.json"), &json!({ "mcpServers": { "userwide": stdio(json!({})) } }).to_string());
    write(&work.join(".pi/mcp.json"), &json!({ "mcpServers": { "repo": stdio(json!({})) } }).to_string());
    write(
        &work.join(".pi/extensions/evil.ts"),
        "export default (pi: any) => pi.registerTool({ name: \"evil_tool\", label: \"evil\", description: \"x\", parameters: { type: \"object\", properties: {} }, async execute() { return { content: [{ type: \"text\", text: \"EVIL\" }], details: {} }; } });\n",
    );
    write(&data.join("pi-agent/trust.json"), &json!({ work.to_string_lossy(): true }).to_string());
    app.reload_agents();

    // Its skills and the repository's, and no others; its MCP connections in the prompt —
    // from the first request of the fresh session on.
    mail(app, "anna", "human", "bender", "RUN: echo ready", None);
    let req = until("bender's first request", 60, || request_with(log, "executor", "echo ready")).await;
    assert!(req.system.contains("You are the **executor** of a focus team"), "the role's prompt:\n{}", req.system);
    assert!(req.system.contains("<name>owasp</name>"), "the role's skill:\n{}", req.system);
    assert!(req.system.contains("<name>house-style</name>"), "the repository's skill");
    assert!(!req.system.contains("user-wide"), "no other skills");
    assert!(req.system.contains("## MCP connections") && req.system.contains("`tracker`"), "{}", req.system);
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

    // The role's connections only: the granted tools of `tracker` are declared and nothing else of it,
    // `wide` is found with tool search, and nothing of the machine user's or the repository's —
    // neither MCP servers nor the repository's extension — and no codemode.
    let has = |n: &str| req.tools.iter().any(|t| t == n);
    let any = |p: &str| req.tools.iter().any(|t| t.contains(p));
    assert!(has("mcp__tracker__get_issue") && has("mcp__tracker__list_issues"), "{:?}", req.tools);
    assert!(!has("mcp__tracker__delete_issue") && !has("mcp__tracker__whoami"), "ungranted tools are never exposed: {:?}", req.tools);
    assert!(has("tool_search") && !any("mcp__wide__"), "a whole connection is searched: {:?}", req.tools);
    assert!(!any("userwide") && !any("mcp__repo") && !any("secret") && !has("evil_tool"), "{:?}", req.tools);
    assert!(!has("codemode"), "codemode is for connections the admin set to it: {:?}", req.tools);

    // A denied command is blocked, the rest of the shell works.
    mail(app, "anna", "human", "bender", "RUN: echo one && git push origin main", None);
    let out = tool_result(log, "git push origin main").await;
    assert!(out.contains("may not run `git push*`"), "{out}");

    // MCP through the gateway: a granted tool passes.
    mail(app, "anna", "human", "bender", r#"MCPCALL: {"tool": "mcp__tracker__get_issue", "args": {"id": 7}}"#, None);
    assert!(tool_result(log, r#""id": 7"#).await.contains(r#"get_issue {"id":7}"#), "a granted tool");
    // Tool search declares the tools of `wide`, which are then called like any.
    mail(app, "anna", "human", "bender", r#"MCPCALL: {"tool": "tool_search", "args": {"query": "whoami"}}"#, None);
    assert!(tool_result(log, "whoami").await.contains("mcp__wide__whoami"));
    mail(app, "anna", "human", "bender", r#"MCPCALL: {"tool": "mcp__wide__whoami", "args": {}}"#, None);
    assert!(tool_result(log, "mcp__wide__whoami").await.contains("pid"), "a tool found by search");
    // A tool pi does not know is no MCP call.
    mail(app, "anna", "human", "bender", r#"MCPCALL: {"tool": "mcp__userwide__whoami", "args": {}}"#, None);
    let out = tool_result(log, "mcp__userwide__whoami").await;
    assert!(!out.contains("pid") && !out.contains("EVIL"), "{out}");

    // A rule changed while the agent runs applies at its next tool call, without a restart: one
    // tool and a whole connection taken away, a command denied.
    let pid = session_state(app, "bender").unwrap().2;
    write(
        &data.join("agents/executor.md"),
        "---\nmodel: fake/executor\nskills: [owasp]\ndenyCommands: [\"git push*\", \"curl *\"]\nmcp: [\"tracker:get_*\"]\n---\n",
    );
    app.reload_agents();
    mail(app, "anna", "human", "bender", "RUN: curl https://example.com", None);
    let out = tool_result(log, "curl https://example.com").await;
    assert!(out.contains("may not run `curl *`"), "{out}");
    mail(app, "anna", "human", "bender", r#"MCPCALL: {"tool": "mcp__tracker__list_issues", "args": {}}"#, None);
    let out = tool_result(log, "mcp__tracker__list_issues").await;
    assert!(out.contains("may use only these tools of tracker: get_*") && !out.contains("list_issues {"), "{out}");
    mail(app, "anna", "human", "bender", r#"MCPCALL: {"tool": "mcp__wide__whoami", "args": {"again": true}}"#, None);
    let out = tool_result(log, "again\": true").await;
    assert!(out.contains("MCP connection wide is not granted") && !out.contains("pid"), "{out}");
    assert_eq!(session_state(app, "bender").unwrap().2, pid, "the same session");

    // A read-only role with codemode: a script reaches its connection, and nothing else it must not
    // — no edit or write (not there at all), no denied command (the guard sees the script's calls),
    // no connection of another role, no server of the machine's user.
    let script = "const run = async (f) => { try { await f(); return 'ok'; } catch (e) { return 'refused: ' + String(e && e.message || e).slice(0, 160); } }; \
        text('whoami=' + await run(() => tools.mcp__cm__whoami({}))); \
        text('write=' + await run(() => tools.write({ path: 'x.txt', content: 'y' }))); \
        text('edit=' + await run(() => tools.edit({ path: 'x.txt', edits: [] }))); \
        text('push=' + await run(() => tools.bash({ command: 'git push origin main' }))); \
        text('echo=' + await run(() => tools.bash({ command: 'echo from-codemode' }))); \
        text('tracker=' + await run(() => tools.mcp__tracker__whoami({}))); \
        text('userwide=' + await run(() => tools.mcp__userwide__whoami({})));";
    mail(app, "anna", "human", "yoda", &format!("CODEMODE: {script}"), None);
    let out = tool_result_of(log, "reviewer", "whoami=").await;
    let line = |k: &str| out.lines().find(|l| l.starts_with(&format!("{k}="))).unwrap_or_default().to_string();
    assert_eq!(line("whoami"), "whoami=ok", "{out}");
    assert!(line("write").contains("refused"), "{out}");
    assert!(line("edit").contains("refused"), "{out}");
    assert!(line("push").contains("may not run `git push*`"), "{out}");
    assert_eq!(line("echo"), "echo=ok", "{out}");
    assert!(line("tracker").contains("refused") && line("userwide").contains("refused"), "{out}");
    let req = until("yoda's request", 10, || request_with(log, "reviewer", "whoami=")).await;
    assert!(req.tools.iter().any(|t| t == "codemode") && !req.tools.iter().any(|t| t == "write" || t == "edit"), "{:?}", req.tools);
    assert!(!req.tools.iter().any(|t| t.starts_with("mcp__")), "the connection stays behind codemode: {:?}", req.tools);
}

/// pi runs under a V8 heap cap added to the operator's own `NODE_OPTIONS`; what pi runs does not inherit it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_runs_under_a_heap_cap_its_commands_do_not_inherit() {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is not on PATH: install Pi-Bolt before `cargo test`");
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
        assert!(std::env::var("CI").is_err(), "pi is not on PATH: install Pi-Bolt before `cargo test`");
        eprintln!("skipped: pi is not on PATH (install Pi-Bolt)");
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
        .env("GENIE_BUS_POLL_MS", "20")
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
        assert!(std::env::var("CI").is_err(), "pi is not on PATH: install Pi-Bolt before `cargo test`");
        return;
    };
    let clock = Arc::new(crate::common::ManualClock::default());
    let l = live_with_clock(pi, |cfg| cfg.runtime.max_attempts = 2, clock.clone()).await;
    let app = &l.app;

    mail(app, "anna", "human", "bender", "PROVIDER-DOWN please work", None);
    until("the first failed run", 20, || (app.attempts.failures(&member("bender")) == 1).then_some(())).await;
    clock.advance(Duration::from_millis(999));
    genie::runtime::tick(app).await.unwrap();
    assert_eq!(app.attempts.failures(&member("bender")), 1, "no retry before its deadline");
    clock.advance(Duration::from_millis(1));
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
        assert!(std::env::var("CI").is_err(), "pi is not on PATH: install Pi-Bolt before `cargo test`");
        eprintln!("skipped: pi is not on PATH (install Pi-Bolt)");
        return;
    };
    let clock = Arc::new(crate::common::ManualClock::default());
    let l = live_with_clock(pi, |_| {}, clock.clone()).await;
    let (app, log) = (&l.app, &l.log);
    install_dump(app, log);
    let work = app.data.join("work");

    // A first step runs and its letter is acknowledged while the turn runs.
    mail(app, "anna", "human", "bender", "RUN: echo first > a1", None);
    // Idle with its letter taken is not enough: a fresh session is idle before its first wake-up.
    until("the step and its letter to settle", 60, || {
        let idle = session_state(app, "bender").is_some_and(|s| s.0 == "idle");
        let empty = app.with_tracker("shop", |t| t.bus().pending(Some("SHOP-1"), "bender")).unwrap().is_empty();
        (idle && empty && work.join("a1").exists()).then_some(())
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
    let req = until("the restart note in a model request", 30, || {
        // The simulated shutdown may have recorded a crash backoff before recovery finishes.
        if let Some(wait) = app.attempts.wait_for(&member("bender")) {
            clock.advance(wait);
            app.wake_runtime.notify_one();
        }
        request_with(log, "executor", "The genie server restarted after a crash")
    })
    .await;
    eprintln!("latency: after a server crash mid-turn, the agent was told to continue after {:?}", req.at - crashed);
    assert_eq!(req.count("RUN: echo first"), 1, "the acknowledged letter is not injected twice");
    until("the agent to continue its own work", 60, || work.join("a2").exists().then_some(())).await;
    eprintln!("a server crash mid-turn: the agent resumed by itself and wrote a2");
}

/// A person of the project with a personal token: how people call the API.
fn person_token(app: &App) -> String {
    let u = app.with_server(|db| db.create_user("ines", "ines", None, None, false)).unwrap();
    app.with_server(|db| db.set_membership("shop", u.id, ProjectRole::Member)).unwrap();
    app.with_server(|db| db.create_user_token(u.id, "test")).unwrap()
}

fn orchestrator() -> AgentKey {
    AgentKey::Orchestrator { project: "shop".into() }
}

/// The state of the live session of `key`, if one runs.
fn live_state(app: &App, key: &AgentKey) -> Option<String> {
    app.sessions.get(key).map(|s| s.live().state)
}

/// A project whose manager runs without chat: not `manual`, so it is wanted all the time.
fn automatic(app: &App) {
    app.with_server(|db| db.set_autonomy("shop", "assisted")).unwrap();
}

/// A person warmed the manual project's manager just now: that keeps it wanted (`POST /api/chat/warm`).
/// Unlike `automatic`, the team's task is not adopted, so no task manager takes a session slot.
fn warmed_chat(app: &App) {
    assert!(app.sessions.warm("shop", app.clock.now(), Duration::from_secs(30)));
}

/// `POST /api/chat/warm` as a person: the status and the body.
async fn warm(app: &App, token: &str) -> (reqwest::StatusCode, Value) {
    let res =
        reqwest::Client::new().post(format!("http://127.0.0.1:{}/api/chat/warm", app.cfg.port)).bearer_auth(token).send().await.unwrap();
    let status = res.status();
    (status, res.json().await.unwrap())
}

/// The row of `agent` on the board (`GET /api/agents`), as a person sees it.
async fn board_row(app: &App, token: &str, agent: &str) -> Value {
    let board: Value = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{}/api/agents", app.cfg.port))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    board["agents"].as_array().unwrap().iter().find(|a| a["agent"] == agent).cloned().unwrap_or_else(|| panic!("no {agent}: {board}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_warmed_manager_settles_idle_without_a_model_call_and_answers_its_first_mail_at_once() {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is not on PATH: install Pi-Bolt before `cargo test`");
        return;
    };
    let l = live(pi, |_| {}).await;
    let app = &l.app;
    let token = person_token(app);

    // Opening the chat warms the manual project's manager: it starts, and waits for its first mail.
    let (status, body) = warm(app, &token).await;
    assert_eq!((status, body["started"].as_bool()), (reqwest::StatusCode::OK, Some(true)), "{body}");
    until("the warmed manager to be idle", 30, || (live_state(app, &orchestrator()).as_deref() == Some("idle")).then_some(())).await;
    assert!(l.log.lock().unwrap().is_empty(), "a warm-up asks the model nothing");
    let row = board_row(app, &token, ORCHESTRATOR).await;
    assert!(row["session"]["startup"]["readyMs"].is_u64(), "{row}");
    assert!(row["session"]["startup"]["firstReplyMs"].is_null(), "no reply yet: {row}");

    app.with_tracker("shop", |t| {
        t.bus().notify_manager(ORCHESTRATOR, "ines", "human", CHAT_MAIL, "Project chat message #1 from @ines:\n\n@pm status?", None)
    })
    .unwrap();
    app.wake_runtime.notify_one();
    until("the manager's first reply", 60, || app.sessions.get(&orchestrator()).and_then(|s| s.live().startup.first_reply_ms)).await;
    let row = board_row(app, &token, ORCHESTRATOR).await;
    assert!(row["session"]["startup"]["firstReplyMs"].is_u64(), "{row}");
    assert!(!l.log.lock().unwrap().is_empty(), "the mail was answered by the model");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_managers_idle_window_is_longer_than_a_members() {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is not on PATH: install Pi-Bolt before `cargo test`");
        return;
    };
    let clock = Arc::new(crate::common::ManualClock::default());
    let l = live_with_clock(pi, |_| {}, clock.clone()).await;
    let app = &l.app;
    automatic(app);
    assert!(genie::sessions::prewarm(app, &orchestrator()).await.unwrap());
    mail(app, "anna", "human", "bender", "hello", None);
    settled_run(app, "bender").await;
    until("the manager to settle idle", 30, || (live_state(app, &orchestrator()).as_deref() == Some("idle")).then_some(())).await;

    // Past idleStopSecs (300 s) and inside managerIdleStopSecs (3600 s).
    clock.advance(Duration::from_secs(301));
    genie::runtime::tick(app).await.unwrap();
    assert!(session_state(app, "bender").is_none_or(|s| s.0 == "stopping"), "the member stops after idleStopSecs");
    assert_eq!(live_state(app, &orchestrator()).as_deref(), Some("idle"), "the manager stays for managerIdleStopSecs");

    clock.advance(Duration::from_secs(3300));
    genie::runtime::tick(app).await.unwrap();
    assert!(live_state(app, &orchestrator()).is_none_or(|s| s == "stopping"), "the manager stops after managerIdleStopSecs");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn making_room_stops_a_member_before_the_warm_manager() {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is not on PATH: install Pi-Bolt before `cargo test`");
        return;
    };
    let l = live(pi, |cfg| cfg.runtime.max_sessions = 2).await;
    let app = &l.app;
    warmed_chat(app);
    // The manager has been idle the longest of the two: the old rule would have stopped it.
    assert!(genie::sessions::prewarm(app, &orchestrator()).await.unwrap());
    until("the manager to settle idle", 30, || (live_state(app, &orchestrator()).as_deref() == Some("idle")).then_some(())).await;
    mail(app, "anna", "human", "bender", "hello", None);
    settled_run(app, "bender").await;

    mail(app, "anna", "human", "yoda", "EVICT-ME hello", None);
    until("bender to make room", 30, || session_state(app, "bender").is_none_or(|s| s.0 == "stopping").then_some(())).await;
    assert_eq!(live_state(app, &orchestrator()).as_deref(), Some("idle"), "the manager keeps its session");
    until("yoda to answer once it has its slot", 60, || request_with(&l.log, "reviewer", "EVICT-ME").map(|_| ())).await;
    assert_eq!(live_state(app, &orchestrator()).as_deref(), Some("idle"), "the manager was never the one evicted");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_warm_up_never_takes_a_session_slot_from_another_agent() {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is not on PATH: install Pi-Bolt before `cargo test`");
        return;
    };
    let l = live(pi, |cfg| cfg.runtime.max_sessions = 1).await;
    let app = &l.app;
    warmed_chat(app);
    assert!(genie::sessions::prewarm(app, &orchestrator()).await.unwrap());
    until("the manager to settle idle", 30, || (live_state(app, &orchestrator()).as_deref() == Some("idle")).then_some(())).await;
    assert!(!genie::sessions::prewarm(app, &member("bender")).await.unwrap(), "the only slot is taken, and a warm-up does not evict");
    assert_eq!(live_state(app, &orchestrator()).as_deref(), Some("idle"));
    assert!(session_state(app, "bender").is_none());
}

/// The manager of a manual project has finished a chat answer: one of its turns succeeded.
fn answered(app: &App) -> bool {
    app.with_server(|db| db.turns("shop", Some("orchestrator"), 10)).unwrap().iter().any(|t| t.status == "succeeded")
}

/// A manual project's manager takes the chat it is asked and nothing else: a system note waits, queued
/// and untouched, while the model answers the chat (the live path: `deliver` and the session's lease).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_manual_managers_session_takes_chat_and_leaves_the_rest_queued() {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is not on PATH: install Pi-Bolt before `cargo test`");
        eprintln!("skipped: pi is not on PATH (install Pi-Bolt)");
        return;
    };
    let l = live(pi, |_| {}).await;
    let app = &l.app;
    app.with_tracker("shop", |t| {
        t.bus().notify_manager(ORCHESTRATOR, "genie", "system", "system", "AUTOMATION-NOTE: a note nobody asked for", None)?;
        t.bus().notify_manager(ORCHESTRATOR, "anna", "human", CHAT_MAIL, "CHAT-QUESTION: is the export done?", None)
    })
    .unwrap();
    app.wake_runtime.notify_one();

    until("the manager to answer the chat", 60, || request_with(&l.log, "orchestrator", "CHAT-QUESTION")).await;
    until("the manager's answer to be recorded", 60, || answered(app).then_some(())).await;
    let requests = l.log.lock().unwrap().clone();
    assert!(requests.iter().all(|r| r.count("AUTOMATION-NOTE") == 0), "the note never reached the model");
    // The chat was taken; the note waits, undelivered (the harness's inbox notice waits too).
    let queued = app.with_tracker("shop", |t| t.bus().pending(None, ORCHESTRATOR)).unwrap();
    assert!(queued.iter().all(|m| m.kind != CHAT_MAIL), "every chat was taken: {queued:?}");
    let note = queued.iter().find(|m| m.text.starts_with("AUTOMATION-NOTE")).expect("the note waits in the queue");
    assert!(note.delivered_at.is_none(), "the note is still queued");
}

/// The manager of a manual project is interrupted by a server crash in the middle of its chat answer.
/// Its continuation is chat from genie: the new session starts for it without anyone writing, and the
/// question it had already been given is not injected again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_manual_manager_interrupted_by_a_restart_finishes_its_chat_answer() {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is not on PATH: install Pi-Bolt before `cargo test`");
        eprintln!("skipped: pi is not on PATH (install Pi-Bolt)");
        return;
    };
    let l = live(pi, |_| {}).await;
    let app = &l.app;
    app.with_tracker("shop", |t| {
        t.bus().notify_manager(ORCHESTRATOR, "anna", "human", CHAT_MAIL, "CHAT-QUESTION: is the export done?", None)
    })
    .unwrap();
    app.wake_runtime.notify_one();
    until("the manager to receive the question", 60, || request_with(&l.log, "orchestrator", "CHAT-QUESTION")).await;
    until("the manager's answer to be recorded", 60, || answered(app).then_some(())).await;

    // The server dies: the session goes with it, and the turn it ran stays `running` in the database.
    genie::sessions::stop_all(app);
    until("the manager's session to end", 30, || app.sessions.get(&orchestrator()).is_none().then_some(())).await;
    let turn = app
        .with_server(|db| {
            let t = db.start_turn("shop", ORCHESTRATOR, None, None, None)?;
            Ok::<_, genie_core::GenieError>(t)
        })
        .unwrap();

    let restarted = Instant::now();
    genie::runtime::recover(app).unwrap();
    genie::sessions::recover(app);
    assert_eq!(app.with_server(|db| db.turn(turn)).unwrap().status, "interrupted");

    let req = until("the restart note in the manager's model request", 30, || {
        request_with(&l.log, "orchestrator", "The genie server restarted after a crash")
    })
    .await;
    eprintln!("latency: a manual manager resumed its chat answer {:?} after the restart", req.at - restarted);
    assert_eq!(req.count("CHAT-QUESTION"), 1, "the question is in the resumed conversation once, not injected again");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_taskless_project_manager_reaching_its_hard_limit_stops_its_live_session() {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is required in CI");
        return;
    };
    let clock = Arc::new(crate::common::ManualClock::default());
    let l = live_with_clock(
        pi,
        |cfg| {
            cfg.runtime.turn_timeout_secs = 30;
            cfg.runtime.run_timeout_secs = 3;
        },
        clock.clone(),
    )
    .await;
    let app = &l.app;
    app.with_tracker("shop", |t| t.bus().notify_manager(ORCHESTRATOR, "anna", "human", CHAT_MAIL, "RUN: sleep 120", None)).unwrap();
    app.wake_runtime.notify_one();
    let session = until("project manager's long tool started", 20, || {
        app.sessions.get(&orchestrator()).filter(|s| s.live().tool.as_ref().is_some_and(|tool| tool["name"] == "bash"))
    })
    .await;
    assert!(session.task.is_none(), "exercise the taskless hard-limit path");
    // The active session is responsive: its hard deadline, rather than silence,
    // must trigger the stop and terminalize this turn.
    clock.advance(Duration::from_millis(2999));
    assert!(session.request(json!({"type":"get_state"}), Duration::from_secs(5)).await.is_some());
    clock.advance(Duration::from_millis(1));
    genie::runtime::tick(app).await.unwrap();
    until("project manager's limited turn interrupted", 10, || {
        app.with_server(|db| db.turns("shop", Some(ORCHESTRATOR), 10))
            .unwrap()
            .iter()
            .any(|turn| turn.pid == Some(i64::from(session.pid)) && turn.status == "interrupted")
            .then_some(())
    })
    .await;
    until("limited project-manager process removed", 10, || {
        app.sessions.get(&orchestrator()).is_none_or(|next| next.pid != session.pid).then_some(())
    })
    .await;
    assert!(app.with_tracker("shop", |t| t.execution().blocking("SHOP-1")).unwrap().is_none());
    genie::sessions::stop_all(app);
}

async fn owned_tool_pid(work: &std::path::Path, file: &str) -> String {
    until("owned tool's host PID", 10, || {
        let inside = std::fs::read_to_string(work.join(file)).ok()?;
        // bubblewrap uses a PID namespace: match the shell's namespace PID
        // and this test's unique workspace to identify the host process.
        std::fs::read_dir("/proc").ok()?.flatten().find_map(|entry| {
            let path = entry.path();
            if std::fs::read_link(path.join("cwd")).ok()? != work {
                return None;
            }
            let status = std::fs::read_to_string(path.join("status")).ok()?;
            let pids = status.lines().find_map(|line| line.strip_prefix("NSpid:"))?;
            (pids.split_whitespace().last()? == inside.trim()).then(|| entry.file_name().to_string_lossy().into_owned())
        })
    })
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_live_hard_limit_stops_work_and_allows_independent_work_while_its_pause_write_fails() {
    hard_limit_storage_failure(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_live_hard_limit_keeps_its_fence_after_exit_until_its_server_intent_can_be_stored() {
    hard_limit_storage_failure(true).await;
}

async fn hard_limit_storage_failure(server: bool) {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is required in CI");
        return;
    };
    let clock = Arc::new(crate::common::ManualClock::default());
    let l = live_with_clock(
        pi,
        |cfg| {
            cfg.runtime.max_concurrent = 1;
            cfg.runtime.max_sessions = 1;
            cfg.runtime.turn_timeout_secs = 30;
            cfg.runtime.run_timeout_secs = 3;
        },
        clock.clone(),
    )
    .await;
    let app = &l.app;
    let fault = format!(
        "CREATE TRIGGER fail_time_pause BEFORE INSERT ON {} BEGIN SELECT RAISE(ABORT,'pause storage fault'); END;",
        if server { "execution_pauses" } else { "task_execution" },
    );
    if server {
        app.with_server(|db| db.conn().execute_batch(&fault).map_err(Into::into)).unwrap();
    } else {
        app.with_tracker("shop", |t| t.conn().execute_batch(&fault).map_err(Into::into)).unwrap();
    }
    mail(app, "anna", "human", "bender", "RUN: echo $BASHPID > limit-tool.pid; echo kept > partial.txt; sleep 120", None);
    until("limited session's long tool", 20, || session_state(app, "bender").filter(|s| s.1.as_deref() == Some("bash"))).await;
    let old = app.sessions.get(&member("bender")).unwrap();
    let work = l._dir.path().join("work");
    let tool = owned_tool_pid(&work, "limit-tool.pid").await;
    clock.advance(Duration::from_secs(3));
    genie::runtime::tick(app).await.unwrap();
    until("limited owned tool stopped despite storage failure", 5, || {
        let running = std::fs::read_to_string(format!("/proc/{tool}/stat"))
            .is_ok_and(|stat| !stat.split(") ").nth(1).is_some_and(|tail| tail.starts_with('Z')));
        (!running).then_some(())
    })
    .await;
    assert_eq!(
        app.with_server(|db| Ok(db.conn().query_row(
            "SELECT COUNT(*) FROM api_tokens WHERE project='shop' AND member='bender' AND revoked=0",
            [],
            |r| r.get::<_, i64>(0)
        )?))
        .unwrap(),
        0,
        "the interrupted worker has no remaining authority"
    );
    assert!(genie::tasks::resume_execution(app, &genie::tasks::Caller::person("shop", "anna"), "SHOP-1").is_err());
    if server {
        assert_eq!(
            app.with_tracker("shop", |t| t.execution().blocking("SHOP-1")).unwrap().unwrap().reason,
            "time",
            "the project DB preserves the pause across a restart while its server intent write is unavailable"
        );
        assert!(app.with_server(|db| db.pending_execution_pauses(None)).unwrap().is_empty());
        assert_eq!(app.sessions.get(&member("bender")).unwrap().pid, old.pid, "the stopped session retains the memory fence");
    } else {
        assert!(app.with_tracker("shop", |t| t.execution().blocking("SHOP-1")).unwrap().is_none());
        assert_eq!(app.with_server(|db| db.pending_execution_pauses(None)).unwrap().len(), 1);
    }
    let other = app
        .with_tracker("shop", |t| {
            let task = t.create(
                &Actor::new("anna", Role::Human),
                CreateInput { title: "Independent work".into(), quiet: true, ..Default::default() },
            )?;
            t.bus().create(
                "anna",
                "human",
                NewTeam {
                    id: task.id.clone(),
                    task: task.id.clone(),
                    cwd: work.to_string_lossy().into_owned(),
                    members: vec![NewMember { name: "independent".into(), role: "executor".into(), ..Default::default() }],
                    ..Default::default()
                },
            )?;
            t.bus().send(SendMail {
                team: &task.id,
                from: "anna",
                from_role: "human",
                to: "independent",
                text: "Independent hello",
                kind: "owner",
                ..Default::default()
            })?;
            Ok(task.id)
        })
        .unwrap();
    genie::runtime::tick(app).await.unwrap();
    until("independent task uses the freed run slot", 20, || {
        app.with_server(|db| db.turns("shop", Some(&format!("{other}/independent")), 10))
            .unwrap()
            .iter()
            .any(|turn| turn.status == "succeeded")
            .then_some(())
    })
    .await;
    assert!(
        app.sessions.get(&member("bender")).is_none_or(|session| session.pid == old.pid),
        "the failed pause cannot restart the old task"
    );
    if server {
        app.with_server(|db| db.conn().execute_batch("DROP TRIGGER fail_time_pause").map_err(Into::into)).unwrap();
        clock.advance(Duration::from_secs(1));
    } else {
        app.with_tracker("shop", |t| t.conn().execute_batch("DROP TRIGGER fail_time_pause").map_err(Into::into)).unwrap();
    }
    genie::runtime::tick(app).await.unwrap();
    until("pause persisted after storage recovery", 10, || {
        app.with_tracker("shop", |t| t.execution().blocking("SHOP-1")).unwrap().filter(|state| state.reason == "time")
    })
    .await;
    until("intent acknowledged after storage recovery", 10, || {
        app.with_server(|db| db.pending_execution_pauses(None)).unwrap().is_empty().then_some(())
    })
    .await;
    assert_eq!(std::fs::read_to_string(work.join("partial.txt")).unwrap(), "kept\n");
    genie::tasks::resume_execution(app, &genie::tasks::Caller::person("shop", "anna"), "SHOP-1").unwrap();
    genie::runtime::tick(app).await.unwrap();
    until("explicit resume starts a new live worker", 20, || app.sessions.get(&member("bender")).filter(|session| session.pid != old.pid))
        .await;
    genie::sessions::stop_all(app);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hard_run_limit_pauses_productive_work_and_explicit_resume_keeps_progress() {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is required in CI");
        return;
    };
    let clock = Arc::new(crate::common::ManualClock::default());
    let l = live_with_clock(
        pi,
        |cfg| {
            cfg.runtime.turn_timeout_secs = 30;
            cfg.runtime.run_timeout_secs = 3;
        },
        clock.clone(),
    )
    .await;
    let app = &l.app;
    mail(app, "anna", "human", "bender", "RUN: echo $BASHPID > tool.pid; echo kept > partial.txt; sleep 120", None);
    until("long tool started", 20, || session_state(app, "bender").filter(|s| s.1.as_deref() == Some("bash"))).await;
    let s = app.sessions.get(&member("bender")).unwrap();
    let work = l._dir.path().join("work");
    let tool_pid = owned_tool_pid(&work, "tool.pid").await;
    clock.advance(Duration::from_millis(2999));
    // This is recent activity, so the silence watchdog cannot justify the stop.
    assert!(s.request(json!({"type":"get_state"}), Duration::from_secs(5)).await.is_some());
    genie::runtime::tick(app).await.unwrap();
    assert!(app.with_tracker("shop", |t| t.execution().blocking("SHOP-1")).unwrap().is_none());
    clock.advance(Duration::from_millis(1));
    genie::runtime::tick(app).await.unwrap();
    until("hard deadline stopped session", 10, || app.sessions.get(&member("bender")).is_none().then_some(())).await;
    until("owned shell tool stopped", 5, || {
        let running = std::fs::read_to_string(format!("/proc/{tool_pid}/stat"))
            .is_ok_and(|stat| !stat.split(") ").nth(1).is_some_and(|tail| tail.starts_with('Z')));
        (!running).then_some(())
    })
    .await;
    let paused = app.with_tracker("shop", |t| t.execution().blocking("SHOP-1")).unwrap().unwrap();
    assert_eq!(paused.reason, "time");
    assert_eq!(std::fs::read_to_string(l._dir.path().join("work/partial.txt")).unwrap(), "kept\n");
    until("interrupted turn recorded", 10, || {
        app.with_server(|db| db.turns("shop", Some("SHOP-1/bender"), 10)).unwrap().iter().any(|t| t.status == "interrupted").then_some(())
    })
    .await;
    genie::runtime::tick(app).await.unwrap();
    assert!(app.sessions.get(&member("bender")).is_none(), "a pending continuation cannot clear the fence");
    genie::tasks::resume_execution(app, &genie::tasks::Caller::person("shop", "anna"), "SHOP-1").unwrap();
    genie::runtime::tick(app).await.unwrap();
    until("explicit resume starts a fresh process", 20, || app.sessions.get(&member("bender")).filter(|next| next.pid != s.pid)).await;
    genie::sessions::stop_all(app);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn task_stop_releases_live_capacity_when_a_tool_inherits_stdout() {
    inherited_stdout_stop(ToolStop::Owner, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_live_deadline_releases_capacity_when_a_tool_inherits_stdout() {
    inherited_stdout_stop(ToolStop::Deadline, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn task_stop_releases_live_capacity_when_a_tool_clears_its_environment() {
    inherited_stdout_stop(ToolStop::Owner, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_live_deadline_releases_capacity_when_a_tool_clears_its_environment() {
    inherited_stdout_stop(ToolStop::Deadline, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parent_exit_cleans_an_environment_cleared_tool_before_draining_stdout() {
    inherited_stdout_stop(ToolStop::ParentExit, true).await;
}

#[derive(Clone, Copy)]
enum ToolStop {
    Owner,
    Deadline,
    ParentExit,
}

async fn inherited_stdout_stop(stop: ToolStop, clear_env: bool) {
    let clock = Arc::new(crate::common::ManualClock::default());
    let l = live_with_clock(
        PathBuf::from("/unused"),
        |cfg| {
            cfg.runtime.enabled = false;
            cfg.runtime.sandbox.mode = "off".into();
            cfg.runtime.max_concurrent = 1;
            cfg.runtime.run_timeout_secs = if matches!(stop, ToolStop::Deadline) { 3 } else { 0 };
            cfg.runtime.session_command = vec![vec![
                "bash".into(),
                "-c".into(),
                r#"while IFS= read -r line; do
                  case "$line" in
                    *'"get_state"'*) printf '%s\n' '{"type":"response","success":true}' ;;
                    *'"prompt"'*)
                      printf '%s\n' '{"type":"agent_start"}'
                      echo kept > partial.txt
                      if [ "$GENIE_TEST_CLEAR_TOOL_ENV" = "true" ]; then
                        env -i /bin/sleep 120 &
                      else
                        sleep 120 &
                      fi
                      echo "$!" > inherited-stdout-tool.pid
                      printf '%s\n' '{"type":"tool_execution_start","toolName":"bash","args":{"command":"sleep 120"}}'
                      if [ "$GENIE_TEST_PARENT_EXIT" = "true" ]; then
                        while [ ! -f exit-parent ]; do sleep 0.02; done
                        exit 0
                      fi
                      ;;
                  esac
                done"#
                    .into(),
            ]];
            cfg.runtime.command = vec![vec!["bash".into(), "-c".into(), r#""$GENIE_BIN" job output '{"ok":true}'"#.into()]];
            cfg.runtime.env.insert("GENIE_BIN".into(), env!("CARGO_BIN_EXE_genie").into());
            cfg.runtime.env.insert("GENIE_TEST_CLEAR_TOOL_ENV".into(), clear_env.to_string());
            cfg.runtime.env.insert("GENIE_TEST_PARENT_EXIT".into(), matches!(stop, ToolStop::ParentExit).to_string());
        },
        clock.clone(),
    )
    .await;
    let app = &l.app;
    mail(app, "anna", "human", "bender", "Start an owned tool with inherited stdout", None);
    genie::runtime::tick(app).await.unwrap();
    until("scripted live harness ready", 5, || app.sessions.get(&member("bender")).filter(|s| s.live().startup.ready_ms.is_some())).await;
    genie::runtime::tick(app).await.unwrap();
    let work = l._dir.path().join("work");
    let tool = until("stdout-inheriting tool and registered run", 5, || {
        let pid = std::fs::read_to_string(work.join("inherited-stdout-tool.pid")).ok()?.trim().parse::<i64>().ok()?;
        let turn = app.with_server(|db| db.turns("shop", Some("SHOP-1/bender"), 10)).unwrap();
        turn.iter().any(|t| t.status == "running" && t.pid.is_some()).then_some(pid)
    })
    .await;
    let session = app.sessions.get(&member("bender")).unwrap();
    if clear_env {
        until("tool exec cleared its environment", 5, || {
            std::fs::read(format!("/proc/{tool}/environ")).is_ok_and(|env| env.is_empty()).then_some(())
        })
        .await;
    }
    let stat = std::fs::read_to_string(format!("/proc/{tool}/stat")).unwrap();
    let group = stat.rsplit_once(") ").unwrap().1.split_whitespace().nth(2).unwrap().parse::<u32>().unwrap();
    assert_eq!(group, session.pid, "the tool stays in the harness's owned group");
    assert_eq!(app.slots.available_permits(), 0);
    let next = app
        .with_tracker("shop", |t| {
            t.create(&Actor::new("anna", Role::Human), CreateInput { title: "Independent job".into(), quiet: true, ..Default::default() })
        })
        .unwrap();
    let job = app
        .with_server(|db| {
            db.create_job(genie_core::work::NewJob {
                project: "shop".into(),
                task: Some(next.id.clone()),
                role: "executor".into(),
                goal: "Independent work after live stop".into(),
                workspace: "none".into(),
                ..Default::default()
            })
        })
        .unwrap();
    match stop {
        ToolStop::Deadline => {
            clock.advance(Duration::from_secs(3));
            genie::runtime::tick(app).await.unwrap();
        }
        ToolStop::Owner => {
            genie::tasks::pause_execution(app, &genie::tasks::Caller::person("shop", "anna"), "SHOP-1", "owner", "Stop inherited stdout")
                .unwrap();
        }
        ToolStop::ParentExit => std::fs::write(work.join("exit-parent"), "exit").unwrap(),
    }
    until("parent and stdout-inheriting tool stopped and capacity released", 5, || {
        (!process_running(i64::from(session.pid))
            && !process_running(tool)
            && app.sessions.get(&member("bender")).is_none()
            && app.slots.available_permits() == 1)
            .then_some(())
    })
    .await;
    let pause = app.with_tracker("shop", |t| t.execution().blocking("SHOP-1")).unwrap();
    match stop {
        ToolStop::Owner => assert_eq!(pause.unwrap().reason, "owner"),
        ToolStop::Deadline => assert_eq!(pause.unwrap().reason, "time"),
        ToolStop::ParentExit => assert!(pause.is_none()),
    }
    assert_eq!(std::fs::read_to_string(work.join("partial.txt")).unwrap(), "kept\n");
    genie::runtime::tick(app).await.unwrap();
    until("independent job completes on the only run slot", 5, || {
        app.with_server(|db| db.job(job.id)).unwrap().status.eq("succeeded").then_some(())
    })
    .await;
}

fn process_running(pid: i64) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| !s.split(") ").nth(1).is_some_and(|s| s.starts_with('Z')))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_deleted_job_does_not_abort_daily_budget_termination_of_other_work() {
    daily_budget_termination(BudgetFault::DeletedTask).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_pause_write_does_not_abort_daily_budget_termination_of_other_work() {
    daily_budget_termination(BudgetFault::ProjectWrite).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_budget_intent_write_preserves_the_project_pause_and_job_across_restart() {
    daily_budget_termination(BudgetFault::ServerIntent).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admission_refusal_preserves_the_last_job_attempt_during_a_pause_write_fault() {
    budget_start_refusal(false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admission_refusal_uses_a_project_fallback_when_the_intent_write_fails() {
    budget_start_refusal(true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unpriced_startup_retains_a_retryable_budget_pause_despite_a_project_write_fault() {
    budget_start_refusal(false, true).await;
}

async fn budget_start_refusal(server_fault: bool, unpriced: bool) {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is required in CI");
        return;
    };
    let l = live(pi.clone(), |cfg| {
        cfg.runtime.enabled = false;
        cfg.runtime.max_concurrent = 1;
        cfg.runtime.max_attempts = 1;
        // No committed spend: the next full-context reservation cannot fit.
        cfg.budgets.per_task = 0.05;
        if !unpriced {
            cfg.model_prices.insert(
                "fake/executor".into(),
                genie::config::ModelPrice { input: 1.0, output: 1.0, cache_read: Some(0.0), cache_write: Some(0.0) },
            );
        }
        cfg.runtime.command = vec![
            vec![pi.to_string_lossy().into_owned(), "--print".into(), "--model".into(), "{model}".into()],
            vec![
                "--no-session".into(),
                "--no-skills".into(),
                "--no-context-files".into(),
                "--no-prompt-templates".into(),
                "--no-mcp".into(),
            ],
            vec!["{message}".into()],
        ];
    })
    .await;
    let app = &l.app;
    let job = app
        .with_server(|db| {
            db.create_job(genie_core::work::NewJob {
                project: "shop".into(),
                task: Some("SHOP-1".into()),
                role: "executor".into(),
                goal: "RUN: test -f retained-budget.txt && genie job output '{\"ok\":true}'".into(),
                workspace: "none".into(),
                ..Default::default()
            })
        })
        .unwrap();
    let workspace = app.data.join(format!("workspaces/shop/job-{}", job.id));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("retained-budget.txt"), "kept\n").unwrap();
    if server_fault {
        app.with_server(|db| db.conn().execute_batch("CREATE TRIGGER fail_budget_pause BEFORE INSERT ON execution_pauses WHEN NEW.reason='budget' BEGIN SELECT RAISE(ABORT,'budget pause fault'); END;").map_err(Into::into)).unwrap();
    } else {
        app.with_tracker("shop", |t| {
            t.conn()
                .execute_batch(
                    "CREATE TRIGGER fail_budget_pause BEFORE INSERT ON task_execution BEGIN SELECT RAISE(ABORT,'budget pause fault'); END;",
                )
                .map_err(Into::into)
        })
        .unwrap();
    }
    genie::runtime::tick(app).await.unwrap();
    until("admission refusal retains the job, pause and capacity", 20, || {
        let retained = app.with_server(|db| db.job(job.id)).unwrap();
        let fenced = if server_fault {
            app.with_tracker("shop", |t| t.execution().blocking("SHOP-1")).unwrap().is_some()
        } else {
            !app.with_server(|db| db.pending_execution_pauses(Some("shop"))).unwrap().is_empty()
        };
        (fenced && retained.status == "queued" && retained.attempts == 0 && app.slots.available_permits() == 1).then_some(())
    })
    .await;
    assert_eq!(app.attempts.failures(&AgentKey::Job { project: "shop".into(), job: job.id }), 0);
    assert!(
        app.with_server(|db| db.turns("shop", None, 10))
            .unwrap()
            .iter()
            .filter(|turn| turn.job == Some(job.id))
            .all(|turn| turn.status == "interrupted")
    );
    assert!(l.log.lock().unwrap().is_empty(), "no provider request may precede the required human resume");
    assert!(app.with_tracker("shop", |t| t.model_calls().list(None, 10)).unwrap().is_empty());
    assert!(genie::budget::check(app, "shop", Some("SHOP-1")).unwrap().is_none(), "the committed budget is not exhausted");
    assert!(genie::tasks::resume_execution(app, &genie::tasks::Caller::person("shop", "anna"), "SHOP-1").is_err());
    resume_budget_job_after_restart(&l, job.id, server_fault).await;
}

enum BudgetFault {
    DeletedTask,
    ProjectWrite,
    ServerIntent,
}

async fn daily_budget_termination(fault: BudgetFault) {
    let deleted = matches!(fault, BudgetFault::DeletedTask);
    let server_fault = matches!(fault, BudgetFault::ServerIntent);
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is required in CI");
        return;
    };
    let l = live(pi.clone(), |cfg| {
        cfg.runtime.enabled = false;
        cfg.runtime.max_concurrent = 3;
        cfg.budgets.per_day = 1.0;
        cfg.runtime.max_attempts = 1;
        cfg.model_prices.insert(
            "fake/executor".into(),
            genie::config::ModelPrice { input: 1.0, output: 1.0, cache_read: Some(0.0), cache_write: Some(0.0) },
        );
        cfg.runtime.command = vec![
            vec![pi.to_string_lossy().into_owned(), "--print".into(), "--model".into(), "{model}".into()],
            vec![
                "--no-session".into(),
                "--no-skills".into(),
                "--no-context-files".into(),
                "--no-prompt-templates".into(),
                "--no-mcp".into(),
            ],
            vec!["{message}".into()],
        ];
    })
    .await;
    let app = &l.app;
    let task = app
        .with_tracker("shop", |t| {
            t.create(
                &Actor::new("anna", Role::Human),
                CreateInput { title: "Surviving budgeted job".into(), quiet: true, ..Default::default() },
            )
        })
        .unwrap();
    let jobs = app
        .with_server(|db| {
            ["SHOP-1", task.id.as_str()]
                .into_iter()
                .map(|task| {
                    db.create_job(genie_core::work::NewJob {
                        project: "shop".into(),
                        task: Some(task.into()),
                        role: "executor".into(),
                        goal: "RUN: if [ -f retained-budget.txt ]; then genie job output '{\"ok\":true}'; else echo kept > retained-budget.txt; sleep 120; fi".into(),
                        workspace: "none".into(),
                        ..Default::default()
                    })
                })
                .collect::<genie_core::Result<Vec<_>>>()
        })
        .unwrap();
    genie::runtime::tick(app).await.unwrap();
    let pids = until("both budgeted one-shot job processes", 20, || {
        let turns = app.with_server(|db| db.turns("shop", None, 10)).unwrap();
        assert!(turns.iter().all(|t| t.status != "failed"), "{turns:?}");
        let pids: Vec<_> = turns.iter().filter(|t| t.job.is_some()).filter_map(|t| t.pid).collect();
        (pids.len() == 2 && l.log.lock().unwrap().len() >= 2).then_some(pids)
    })
    .await;
    until("both budgeted jobs retained progress", 10, || {
        jobs.iter().all(|job| app.data.join(format!("workspaces/shop/job-{}/retained-budget.txt", job.id)).exists()).then_some(())
    })
    .await;
    if deleted {
        genie::tasks::delete(app, &genie::tasks::Caller::person("shop", "anna"), "SHOP-1", false).unwrap();
        let obsolete = app.with_server(|db| db.job(jobs[0].id)).unwrap();
        assert_eq!(obsolete.status, "cancelled");
        assert_eq!(obsolete.task.as_deref(), Some("SHOP-1"));
    } else if server_fault {
        app.with_server(|db| {
            db.conn().execute_batch("CREATE TRIGGER fail_budget_pause BEFORE INSERT ON execution_pauses WHEN NEW.reason='budget' AND NEW.task='SHOP-1' BEGIN SELECT RAISE(ABORT,'budget pause fault'); END;").map_err(Into::into)
        }).unwrap();
    } else {
        app.with_tracker("shop", |t| {
            t.conn().execute_batch(
                "CREATE TRIGGER fail_budget_pause BEFORE INSERT ON task_execution WHEN NEW.task='SHOP-1' BEGIN SELECT RAISE(ABORT,'budget pause fault'); END;",
            ).map_err(Into::into)
        })
        .unwrap();
    }
    app.with_tracker("shop", |t| {
        t.record_usage("legacy", Some(&task.id), "fake/executor", &genie_core::usage::Tokens { input: 1_000_000, ..Default::default() })
    })
    .unwrap();
    let result = genie::budget::enforce(app, "shop", Some(&task.id));
    if deleted {
        result.unwrap();
    } else {
        assert!(result.unwrap_err().to_string().contains("budget pause fault"));
    }
    until("all one-shot workers stopped despite the obsolete or unwritable scope", 5, || {
        (app.sched.snapshot(3, app.slots.available_permits()).running == 0 && pids.iter().all(|pid| !process_running(*pid))).then_some(())
    })
    .await;
    assert_eq!(app.with_tracker("shop", |t| t.execution().blocking(&task.id)).unwrap().unwrap().reason, "budget");
    assert_eq!(app.slots.available_permits(), 3);
    assert_eq!(app.with_server(|db| db.job(jobs[1].id)).unwrap().status, "queued");
    let tokens: i64 = app
        .with_server(|db| {
            Ok(db
                .conn()
                .query_row("SELECT COUNT(*) FROM api_tokens WHERE project=?1 AND kind='agent' AND revoked=0", ["shop"], |r| r.get(0))?)
        })
        .unwrap();
    assert_eq!(tokens, 0);
    if !deleted {
        let retained = app.with_server(|db| db.job(jobs[0].id)).unwrap();
        assert_eq!((retained.status.as_str(), retained.attempts), ("queued", 0), "a budget stop cannot consume the last failure attempt");
        let pending = app.with_server(|db| db.pending_execution_pauses(Some("shop"))).unwrap();
        assert_eq!(pending.len(), if server_fault { 0 } else { 1 });
        if let Some(pending) = pending.first() {
            assert_eq!((pending.task.as_str(), pending.reason.as_str()), ("SHOP-1", "budget"));
        }
        assert!(genie::tasks::resume_execution(app, &genie::tasks::Caller::person("shop", "anna"), "SHOP-1").is_err());

        resume_budget_job_after_restart(&l, jobs[0].id, server_fault).await;
    }
}

async fn resume_budget_job_after_restart(l: &Live, job: i64, server_fault: bool) {
    let app = &l.app;
    // A fresh runtime has no in-memory cancellation/fence state. Its raised
    // allowance cannot implicitly resume the retained job through the fault.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = app.cfg.clone();
    cfg.port = listener.local_addr().unwrap().port();
    cfg.budgets.per_day = 10.0;
    cfg.budgets.per_task = 10.0;
    cfg.model_prices.insert(
        "fake/executor".into(),
        genie::config::ModelPrice { input: 1.0, output: 1.0, cache_read: Some(0.0), cache_write: Some(0.0) },
    );
    let recovered = App::open(l._dir.path(), cfg, PathBuf::from("/nonexistent")).unwrap();
    let router = genie::http::router(recovered.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap();
    });
    genie::runtime::recover(&recovered).unwrap();
    genie::runtime::recover(&recovered).unwrap();
    genie::runtime::tick(&recovered).await.unwrap();
    if !server_fault {
        assert!(genie::tasks::resume_execution(&recovered, &genie::tasks::Caller::person("shop", "anna"), "SHOP-1").is_err());
    }
    assert_eq!(recovered.with_server(|db| db.job(job)).unwrap().attempts, 0);
    if server_fault {
        recovered.with_server(|db| db.conn().execute_batch("DROP TRIGGER fail_budget_pause").map_err(Into::into)).unwrap();
    } else {
        recovered.with_tracker("shop", |t| t.conn().execute_batch("DROP TRIGGER fail_budget_pause").map_err(Into::into)).unwrap();
    }
    genie::runtime::tick(&recovered).await.unwrap();
    let paused = recovered.with_tracker("shop", |t| t.execution().blocking("SHOP-1")).unwrap().unwrap();
    assert_eq!(paused.reason, "budget");
    assert!(recovered.with_server(|db| db.pending_execution_pauses(None)).unwrap().is_empty());
    let interactions: Value =
        reqwest::get(format!("http://127.0.0.1:{}/api/interactions?project=shop", recovered.cfg.port)).await.unwrap().json().await.unwrap();
    assert!(
        interactions.as_array().unwrap().iter().any(|item| item["kind"] == "execution_pause" && item["task"] == "SHOP-1"),
        "{interactions}"
    );
    genie::tasks::resume_execution(&recovered, &genie::tasks::Caller::person("shop", "anna"), "SHOP-1").unwrap();
    genie::runtime::tick(&recovered).await.unwrap();
    until("retained last-attempt job completes after persistence recovery and explicit resume", 20, || {
        recovered.with_server(|db| db.job(job)).unwrap().status.eq("succeeded").then_some(())
    })
    .await;
    let finished = recovered.with_server(|db| db.job(job)).unwrap();
    assert_eq!(finished.attempts, 1);
    assert_eq!(finished.output, Some(json!({"ok":true})));
    assert_eq!(std::fs::read_to_string(recovered.data.join(format!("workspaces/shop/job-{}/retained-budget.txt", job))).unwrap(), "kept\n");
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sessions_share_active_run_capacity_and_idle_sessions_release_it() {
    let Some(pi) = pi_bin() else {
        assert!(std::env::var("CI").is_err(), "pi is required in CI");
        return;
    };
    let l = live(pi, |cfg| {
        cfg.runtime.max_concurrent = 1;
        cfg.runtime.max_sessions = 4;
    })
    .await;
    let app = &l.app;
    mail(app, "anna", "human", "bender", "RUN: sleep 120", None);
    until("first active session", 20, || session_state(app, "bender").filter(|s| s.1.as_deref() == Some("bash"))).await;
    mail(app, "anna", "human", "yoda", "Wait for capacity", None);
    genie::runtime::tick(app).await.unwrap();
    assert_eq!(app.slots.available_permits(), 0);
    assert!(
        app.sessions.get(&member("yoda")).is_none_or(|s| s.live().runs == 0),
        "a second process cannot start a model run without a slot"
    );
    genie::sessions::halt(app, &member("bender"), "test frees capacity");
    until("next session starts", 20, || app.sessions.get(&member("yoda")).filter(|s| s.live().runs > 0)).await;
    until("settled session returns slot", 20, || (app.slots.available_permits() == 1).then_some(())).await;
    assert!(app.sessions.get(&member("yoda")).is_some(), "an idle warm process does not consume active-run capacity");
    genie::sessions::stop_all(app);
}
