//! A person's model keys: kept in their profile, never shown back, and given to
//! the agents started on their behalf — the LiteLLM key an agent owes, and the
//! keys of the providers pi supports, which override the server's environment.

use crate::common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use common::{Harness, call};
use genie::config::Config;
use genie::state::App;
use genie_core::work::NewJob;
use genie_core::{Actor, CreateInput, Role, Status};
use serde_json::json;

async fn login(h: &Harness, login: &str) -> String {
    h.app.with_server(|db| db.create_user(login, login, None, Some("password123"), false)).unwrap();
    let (st, _, cookies) =
        call(&h.remote, "POST", "/api/auth/login").json(json!({ "login": login, "password": "password123" })).send().await;
    assert_eq!(st, StatusCode::OK);
    cookies.iter().find(|c| c.starts_with("genie_session=")).unwrap().split(';').next().unwrap().to_string()
}

#[tokio::test]
async fn keys_are_set_in_the_profile_and_never_shown_back() {
    let h = Harness::new();
    let cookie = login(&h, "anna").await;
    let (st, v, _) = call(&h.remote, "GET", "/api/me/llm-keys").cookie(&cookie).send().await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(v["keys"].as_object().unwrap().is_empty());
    let providers: Vec<&str> = v["providers"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert!(providers.contains(&"litellm"), "{providers:?}");
    assert!(providers.contains(&"anthropic"), "pi's own providers are offered: {providers:?}");
    assert_eq!(v["providers"].as_array().unwrap()[0]["keyVar"], "LITELLM_API_KEY");

    let (st, v, _) =
        call(&h.remote, "PUT", "/api/me/llm-keys/litellm").cookie(&cookie).json(json!({ "key": "sk-anna-0123456789" })).send().await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["key"]["hint"], "…6789");
    let (st, v, _) =
        call(&h.remote, "PUT", "/api/me/llm-keys/anthropic").cookie(&cookie).json(json!({ "key": "sk-ant-0123456789" })).send().await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (_, v, _) = call(&h.remote, "GET", "/api/me/llm-keys").cookie(&cookie).send().await;
    assert_eq!(v["keys"]["litellm"]["hint"], "…6789");
    assert_eq!(v["keys"]["anthropic"]["hint"], "…6789");
    assert!(!v.to_string().contains("sk-anna"), "the keys themselves never leave the server: {v}");

    // Only providers with a key variable of their own; an empty key is a mistake.
    let (st, _, _) = call(&h.remote, "PUT", "/api/me/llm-keys/openai-codex").cookie(&cookie).json(json!({ "key": "x" })).send().await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _, _) = call(&h.remote, "PUT", "/api/me/llm-keys/litellm").cookie(&cookie).json(json!({ "key": "  " })).send().await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    for id in ["anthropic", "litellm"] {
        let (st, _, _) = call(&h.remote, "DELETE", &format!("/api/me/llm-keys/{id}")).cookie(&cookie).send().await;
        assert_eq!(st, StatusCode::OK);
    }
    let (_, v, _) = call(&h.remote, "GET", "/api/me/llm-keys").cookie(&cookie).send().await;
    assert!(v["keys"].as_object().unwrap().is_empty());
    // Somebody else's keys are nobody's business: there is no way to name another user.
    let (st, _, _) = call(&h.remote, "GET", "/api/me/llm-keys").send().await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

struct Live {
    dir: tempfile::TempDir,
    app: Arc<App>,
    _stop: tokio::sync::oneshot::Sender<()>,
}

/// A server whose agent only writes down the model keys it was given.
async fn live() -> Live {
    live_on("litellm/gpt-6-sol").await
}

/// The same, with the `analyst` role on `model` (the jobs of the tests run on it).
async fn live_on(model: &str) -> Live {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    crate::common::give_roles_models(dir.path());
    let mut cfg = Config::load(dir.path()).unwrap();
    // PD24: the model lives on the role; there are no class defaults.
    std::fs::create_dir_all(dir.path().join("agents")).unwrap();
    std::fs::write(dir.path().join("agents/analyst.md"), format!("---\nmodel: {model}\nthinking: high\n---\n")).unwrap();
    cfg.runtime.sandbox.mode = "off".into();
    cfg.port = listener.local_addr().unwrap().port();
    let out = dir.path().join("keys");
    std::fs::create_dir_all(&out).unwrap();
    let script = format!(
        "printf 'litellm=%s anthropic=%s' \"${{LITELLM_API_KEY-unset}}\" \"${{ANTHROPIC_API_KEY-unset}}\" > '{}'/\"$GENIE_AGENT_NAME\"",
        out.display()
    );
    cfg.runtime.command = vec![vec!["bash".into(), "-c".into(), script]];
    cfg.runtime.turn_timeout_secs = 30;
    // The server's own keys must not reach agents once people have their own —
    // a person's key of a pi provider overrides the server's.
    cfg.runtime.env.insert("LITELLM_API_KEY".into(), "server-key".into());
    cfg.runtime.env.insert("ANTHROPIC_API_KEY".into(), "server-ant".into());
    let app = App::open(dir.path(), cfg, PathBuf::from("/nonexistent")).unwrap();
    app.create_project("shop", "Shop", None, None, None).unwrap();
    app.with_server(|db| {
        let anna = db.create_user("anna", "Anna", None, None, false)?;
        db.set_user_secret(anna.id, genie_core::secrets::LITELLM, "sk-anna-0123456789")?;
        db.create_user("bob", "Bob", None, None, false)?;
        Ok(())
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
    Live { dir, app, _stop: tx }
}

async fn wait_for(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for {what}");
}

fn key_file(dir: &Path, agent: &str) -> Option<String> {
    std::fs::read_to_string(dir.join("keys").join(agent)).ok()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agents_run_with_the_key_of_the_person_they_work_for() {
    let l = live().await;
    // Anna's task wakes the orchestrator: it answers her, with her key.
    l.app
        .with_tracker("shop", |t| {
            t.create(
                &Actor::new("anna", Role::Human),
                CreateInput { title: "CSV export".into(), status: Some(Status::Inbox), ..Default::default() },
            )
        })
        .unwrap();
    l.app.wake_runtime.notify_one();
    wait_for("the orchestrator's turn", Duration::from_secs(20), || key_file(l.dir.path(), "orchestrator").is_some()).await;
    assert_eq!(key_file(l.dir.path(), "orchestrator").as_deref(), Some("litellm=sk-anna-0123456789 anthropic=server-ant"));

    // A job she started runs with her key too.
    let job = l
        .app
        .with_server(|db| {
            db.create_job(NewJob {
                project: "shop".into(),
                role: "analyst".into(),
                goal: "look around".into(),
                workspace: "none".into(),
                initiator: Some("anna".into()),
                ..Default::default()
            })
        })
        .unwrap();
    l.app.wake_runtime.notify_one();
    let name = format!("job-{}", job.id);
    wait_for("the job's turn", Duration::from_secs(20), || key_file(l.dir.path(), &name).is_some()).await;
    // The analyst is on a litellm model: the server's anthropic key stays, the server's litellm key does not.
    assert_eq!(key_file(l.dir.path(), &name).as_deref(), Some("litellm=sk-anna-0123456789 anthropic=server-ant"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_persons_key_of_a_pi_provider_overrides_the_servers() {
    let l = live_on("anthropic/claude-opus-5-5").await;
    l.app
        .with_server(|db| {
            let anna = db.user_by_login("anna")?.unwrap();
            db.set_user_secret(anna.id, "anthropic", "sk-anna-ant-0123")
        })
        .unwrap();
    let job = |initiator: &str| {
        let l = l.app.clone();
        let initiator = initiator.to_string();
        l.with_server(move |db| {
            db.create_job(NewJob {
                project: "shop".into(),
                role: "analyst".into(),
                goal: "look around".into(),
                workspace: "none".into(),
                initiator: Some(initiator),
                ..Default::default()
            })
        })
        .unwrap()
    };
    // Anna's agent answers to her keys: both the ones she keeps and the provider's own.
    let anna = job("anna");
    l.app.wake_runtime.notify_one();
    let name = format!("job-{}", anna.id);
    wait_for("anna's turn", Duration::from_secs(20), || key_file(l.dir.path(), &name).is_some()).await;
    assert_eq!(key_file(l.dir.path(), &name).as_deref(), Some("litellm=sk-anna-0123456789 anthropic=sk-anna-ant-0123"));
    // Bob keeps no keys: a pi provider is not owed one, the agent runs on the server's.
    let bob = job("bob");
    l.app.wake_runtime.notify_one();
    let name = format!("job-{}", bob.id);
    wait_for("bob's turn", Duration::from_secs(20), || key_file(l.dir.path(), &name).is_some()).await;
    assert_eq!(key_file(l.dir.path(), &name).as_deref(), Some("litellm=unset anthropic=server-ant"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_whose_person_has_no_litellm_key_does_not_start_and_they_are_told() {
    let l = live().await;
    let job = l
        .app
        .with_server(|db| {
            db.create_job(NewJob {
                project: "shop".into(),
                role: "analyst".into(),
                goal: "look around".into(),
                workspace: "none".into(),
                initiator: Some("bob".into()),
                ..Default::default()
            })
        })
        .unwrap();
    l.app.wake_runtime.notify_one();
    let app = l.app.clone();
    wait_for("the job's attempt to fail", Duration::from_secs(20), || {
        app.with_server(|db| db.job(job.id)).unwrap().error.is_some_and(|e| e.contains("@bob, who has no LiteLLM key"))
    })
    .await;
    assert_eq!(key_file(l.dir.path(), &format!("job-{}", job.id)), None, "the agent never ran");
    let bob = l.app.with_server(|db| db.user_by_login("bob")).unwrap().unwrap();
    let told = l.app.with_server(|db| db.notifications(bob.id, false, 10)).unwrap();
    assert!(told.iter().any(|n| n.kind == "litellm-key"), "{told:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_orchestrator_does_not_work_for_git_host_but_for_the_person_of_the_task() {
    let dir = tempfile::tempdir().unwrap();
    crate::common::give_roles_models(dir.path());
    let cfg = Config::load(dir.path()).unwrap();
    let app = App::open(dir.path(), cfg, PathBuf::from("/nonexistent")).unwrap();
    app.create_project("shop", "Shop", None, None, None).unwrap();
    app.with_server(|db| db.create_user("anna", "Anna", None, None, false).map(|_| ())).unwrap();
    let task = app
        .with_tracker("shop", |t| {
            t.create(&Actor::new("anna", Role::Human), CreateInput { title: "CSV export".into(), ..Default::default() })
        })
        .unwrap();
    // `git-host` reports a merge as a human, but has no account to hold a key.
    let report =
        genie_core::team::Mail { from: "git-host".into(), from_role: "human".into(), task: Some(task.id.clone()), ..Default::default() };
    assert_eq!(genie::llm_key::mail_initiator(&app, "shop", std::slice::from_ref(&report)).as_deref(), Some("anna"));
    // A person with an account who wrote still comes first.
    let anna = genie_core::team::Mail { from: "anna".into(), from_role: "human".into(), ..Default::default() };
    assert_eq!(genie::llm_key::mail_initiator(&app, "shop", &[report, anna]).as_deref(), Some("anna"));
}
