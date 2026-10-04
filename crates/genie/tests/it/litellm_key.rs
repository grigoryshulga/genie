//! A person's LiteLLM key: kept in their profile, never shown back, and given
//! to the agents started on their behalf; an agent whose person has none does
//! not start.

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
async fn the_key_is_set_in_the_profile_and_never_shown_back() {
    let h = Harness::new();
    let cookie = login(&h, "anna").await;
    let (st, v, _) = call(&h.remote, "GET", "/api/me/litellm-key").cookie(&cookie).send().await;
    assert_eq!(st, StatusCode::OK);
    assert!(v["key"].is_null());

    let (st, v, _) =
        call(&h.remote, "PUT", "/api/me/litellm-key").cookie(&cookie).json(json!({ "key": "sk-anna-0123456789" })).send().await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["key"]["hint"], "…6789");
    let (_, v, _) = call(&h.remote, "GET", "/api/me/litellm-key").cookie(&cookie).send().await;
    assert_eq!(v["key"]["hint"], "…6789");
    assert!(!v.to_string().contains("sk-anna"), "the key itself never leaves the server: {v}");

    let (st, _, _) = call(&h.remote, "PUT", "/api/me/litellm-key").cookie(&cookie).json(json!({ "key": "  " })).send().await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _, _) = call(&h.remote, "DELETE", "/api/me/litellm-key").cookie(&cookie).send().await;
    assert_eq!(st, StatusCode::OK);
    let (_, v, _) = call(&h.remote, "GET", "/api/me/litellm-key").cookie(&cookie).send().await;
    assert!(v["key"].is_null());
    // Somebody else's key is nobody's business: there is no way to name another user.
    let (st, _, _) = call(&h.remote, "GET", "/api/me/litellm-key").send().await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

struct Live {
    dir: tempfile::TempDir,
    app: Arc<App>,
    _stop: tokio::sync::oneshot::Sender<()>,
}

/// A server whose agent only writes down the LiteLLM key it was given.
async fn live() -> Live {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = Config::load(dir.path()).unwrap();
    cfg.runtime.sandbox.mode = "off".into();
    cfg.port = listener.local_addr().unwrap().port();
    let out = dir.path().join("keys");
    std::fs::create_dir_all(&out).unwrap();
    let script = format!("printf '%s' \"${{LITELLM_API_KEY-unset}}\" > '{}'/\"$GENIE_AGENT_NAME\"", out.display());
    cfg.runtime.command = vec![vec!["bash".into(), "-c".into(), script]];
    cfg.runtime.turn_timeout_secs = 30;
    // The server's own key must not reach agents once people have their own.
    cfg.runtime.env.insert("LITELLM_API_KEY".into(), "server-key".into());
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
    assert_eq!(key_file(l.dir.path(), "orchestrator").as_deref(), Some("sk-anna-0123456789"));

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
    assert_eq!(key_file(l.dir.path(), &name).as_deref(), Some("sk-anna-0123456789"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_whose_person_has_no_key_does_not_start_and_they_are_told() {
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
