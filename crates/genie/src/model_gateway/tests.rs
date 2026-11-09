use super::*;
use crate::config::Config;
use crate::runtime::{Kit, Mode};
use genie_core::{Actor, CreateInput, Role};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::test]
async fn native_pi_turns_and_manual_compaction_use_the_same_budget_gateway() {
    native_gateway("off").await;
}

#[tokio::test]
async fn sandboxed_native_pi_keeps_source_credentials_hidden_during_compaction() {
    native_gateway("bwrap").await;
}

async fn native_gateway(sandbox: &str) {
    let Some(pi) = std::env::var_os("GENIE_TEST_PI").map(PathBuf::from).or_else(|| crate::doctor::pi_program(&Config::default())) else {
        assert!(std::env::var_os("CI").is_none(), "install shipped Pi before running CI tests");
        eprintln!("skipped native model gateway: Pi is not installed");
        return;
    };
    if sandbox == "bwrap" && !crate::sandbox::works() {
        assert!(std::env::var_os("CI").is_none(), "bubblewrap is required for this CI test");
        eprintln!("skipped sandboxed model gateway: bubblewrap is unavailable");
        return;
    }
    let captured = Arc::new(std::sync::Mutex::new(Vec::<(String, Value)>::new()));
    let seen = captured.clone();
    let fake = Router::new().route(
        "/v1/chat/completions",
        post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push((headers[header::AUTHORIZATION].to_str().unwrap().to_string(), body));
                let base = json!({"id":"local-fake","object":"chat.completion.chunk","model":"model","created":0});
                let mut wire = String::new();
                for event in [
                    json!({"choices":[{"index":0,"delta":{"role":"assistant","content":"proof ".repeat(200)},"finish_reason":null}]}),
                    json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
                    json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":20,"total_tokens":30}}),
                ] {
                    let mut value = base.clone();
                    value.as_object_mut().unwrap().extend(event.as_object().unwrap().clone());
                    wire += &format!("data: {value}\n\n");
                }
                wire += "data: [DONE]\n\n";
                ([(header::CONTENT_TYPE, "text/event-stream")], wire)
            }
        }),
    );
    let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_url = format!("http://{}/v1", upstream.local_addr().unwrap());
    let upstream_server = tokio::spawn(async move {
        axum::serve(upstream, fake).await.unwrap();
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let key_dir = tempfile::Builder::new().prefix(".genie-model-key-test-").tempdir_in(std::env::var_os("HOME").unwrap()).unwrap();
    let key_file = key_dir.path().join("provider.key");
    std::fs::write(&key_file, "probe-only-upstream").unwrap();
    let source = dir.path().join("source-profile");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("models.json"),json!({"providers":{"probe":{"api":"openai-completions","baseUrl":upstream_url,"apiKey":"$PROBE_KEY","models":[{"id":"model","name":"model","reasoning":false,"input":["text"],"contextWindow":65536,"maxTokens":256,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},"compat":{"maxTokensField":"max_tokens"}}]}}}).to_string()).unwrap();
    std::fs::write(
        source.join("settings.json"),
        r#"{"packages":[],"extensions":[],"compaction":{"enabled":false,"keepRecentTokens":1,"reserveTokens":64}}"#,
    )
    .unwrap();
    let mut cfg = Config::load(dir.path()).unwrap();
    cfg.port = listener.local_addr().unwrap().port();
    cfg.vault.commit = Some(false);
    cfg.runtime.enabled = false;
    cfg.runtime.sandbox.mode = sandbox.into();
    cfg.runtime.turn_timeout_secs = 30;
    cfg.budgets.per_task = 10.0;
    cfg.runtime.session_command = vec![
        vec![pi.to_string_lossy().into_owned()],
        vec![
            "--mode".into(),
            "rpc".into(),
            "--model".into(),
            "{model}".into(),
            "--no-session".into(),
            "--no-tools".into(),
            "--no-skills".into(),
            "--no-context-files".into(),
            "--no-prompt-templates".into(),
            "--no-mcp".into(),
        ],
    ];
    cfg.runtime.env.insert("PI_CODING_AGENT_DIR".into(), source.to_string_lossy().into_owned());
    cfg.runtime.env.insert("PROBE_KEY".into(), "probe-only-upstream".into());
    cfg.runtime.env.insert("PROBE_KEY_FILE".into(), key_file.to_string_lossy().into_owned());
    cfg.runtime.env.insert("OPENAI_API_KEY".into(), "must-not-reach-the-agent".into());
    cfg.model_prices
        .insert("probe/model".into(), crate::config::ModelPrice { input: 1.0, output: 2.0, cache_read: Some(0.5), cache_write: Some(0.0) });
    let app = App::open(dir.path(), cfg, PathBuf::from("/nonexistent")).unwrap();
    app.create_project("shop", "", None, None, None).unwrap();
    let task = app
        .with_tracker("shop", |t| {
            t.create(&Actor::new("owner", Role::Human), CreateInput { title: "Bounded native calls".into(), ..Default::default() })
        })
        .unwrap()
        .id;
    let key = AgentKey::Orchestrator { project: "shop".into() };
    let cwd = dir.path().join("empty-workspace");
    std::fs::create_dir(&cwd).unwrap();
    let spec = AgentSpec {
        role: Role::Orchestrator,
        role_id: "project-manager".into(),
        readonly: String::new(),
        name: genie_core::team::ORCHESTRATOR.into(),
        team: None,
        task: Some(task.clone()),
        execution_stamp: Some(app.with_tracker("shop", |t| t.execution().stamp(&task)).unwrap()),
        job: None,
        cwd,
        session_id: "probe".into(),
        model: Some("probe/model".into()),
        thinking: None,
        prompt: "A local provider protocol check".into(),
        kit: Kit::default(),
        initiator: None,
    };
    let token = spec.token(&app, "shop", chrono::Duration::hours(1)).unwrap();
    let (mut command, _) =
        crate::runtime::agent_process(&app, &key, &spec, Mode::Session { extension: Path::new("/unused") }, &token, &Keys::Server)
            .await
            .unwrap();
    for name in ["PROBE_KEY", "OPENAI_API_KEY"] {
        assert!(command.as_std().get_envs().any(|(key, value)| key == name && value.is_none()), "{name} leaked into the agent command");
    }
    let profile_path = command.as_std().get_envs().find(|(key, _)| *key == "PI_CODING_AGENT_DIR").unwrap().1.unwrap().to_os_string();
    let profile_path = PathBuf::from(profile_path);
    let private = std::fs::read_to_string(profile_path.join("models.json")).unwrap();
    assert!(private.contains("$GENIE_TOKEN"));
    assert!(!private.contains("probe-only-upstream"));
    assert!(!private.contains(&upstream_url));
    if sandbox == "bwrap" {
        // Exercise the actual mount/environment plan with a process which
        // tries to read the source profile, not just inspect its arguments.
        let mut probe = tokio::process::Command::new(command.as_std().get_program());
        probe
            .args(command.as_std().get_args().take_while(|arg| *arg != "--"))
            .args([
                "--",
                "sh",
                "-c",
                r#"test ! -e "$1/models.json" && test -z "$PROBE_KEY" && test -z "$PROBE_KEY_FILE" && test -z "$OPENAI_API_KEY" && test -r "$2/models.json" && test ! -s "$3""#,
                "_",
            ])
            .arg(&source)
            .arg(&profile_path)
            .arg(&key_file)
            .current_dir(&spec.cwd);
        for (name, value) in command.as_std().get_envs() {
            match value {
                Some(value) => {
                    probe.env(name, value);
                }
                None => {
                    probe.env_remove(name);
                }
            }
        }
        assert!(probe.status().await.unwrap().success(), "the credential source must be inaccessible in the worker sandbox");
    }
    let server_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, crate::http::router(server_app).into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap();
    });
    command.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    let mut child = command.spawn().unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
    for n in 0..3 {
        input
            .write_all(
                format!("{}\n", json!({"id":format!("prompt-{n}"),"type":"prompt","message":format!("Local prompt {n}")})).as_bytes(),
            )
            .await
            .unwrap();
        input.flush().await.unwrap();
        rpc_until(&mut output, |value| value["type"] == "agent_end").await;
    }
    let before = captured.lock().unwrap().len();
    assert_eq!(before, 3);
    input.write_all(b"{\"id\":\"compact\",\"type\":\"compact\",\"customInstructions\":\"Keep proof\"}\n").await.unwrap();
    input.flush().await.unwrap();
    let result = rpc_until(&mut output, |value| value["id"] == "compact").await;
    assert_eq!(result["success"], true, "{result}");
    let seen = captured.lock().unwrap().clone();
    assert!(seen.len() > before, "manual compaction must reach the gateway too");
    assert!(seen.iter().all(|(auth, body)| auth == "Bearer probe-only-upstream"
        && body["model"] == "model"
        && body["max_tokens"].as_u64().is_some_and(|n| n > 0 && n <= 256)));
    let receipts = app.with_tracker("shop", |t| t.model_calls().list(Some(&task), 100)).unwrap();
    assert_eq!(receipts.len(), seen.len());
    assert!(receipts.iter().all(|receipt| receipt.state == genie_core::model_calls::CallState::Settled));
    assert!(receipts.iter().all(|receipt| receipt.tokens == Some(Tokens { input: 10, output: 20, ..Default::default() })));
    assert!(receipts.iter().any(|receipt| receipt.output_limit < 256), "compaction's smaller actual cap must be retained");
    assert_eq!(app.with_tracker("shop", |t| t.usage_of_task(&task)).unwrap().iter().map(|row| row.calls).sum::<u64>(), seen.len() as u64);
    child.kill().await.unwrap();
    child.wait().await.unwrap();
    server.abort();
    upstream_server.abort();
}

async fn rpc_until<R: tokio::io::AsyncBufRead + Unpin>(lines: &mut tokio::io::Lines<R>, predicate: impl Fn(&Value) -> bool) -> Value {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let line = lines.next_line().await.unwrap().expect("Pi ended before its RPC reply");
            if let Ok(value) = serde_json::from_str::<Value>(&line)
                && predicate(&value)
            {
                return value;
            }
        }
    })
    .await
    .expect("Pi RPC timed out")
}
