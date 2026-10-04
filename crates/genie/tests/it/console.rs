//! The orchestrator console: a person's own session takes the orchestrator's
//! place. While it holds the console only its token takes the orchestrator's
//! mail; given back (or lapsed), the server's orchestrator carries on.

use crate::common;

use std::path::PathBuf;

use axum::http::StatusCode;
use common::{Harness, call};
use genie::config::Config;
use genie::state::App;
use genie_core::Role;
use genie_core::server_db::ProjectRole;
use serde_json::{Value, json};

fn person(h: &Harness, login: &str, role: ProjectRole) -> String {
    let u = h.app.with_server(|db| db.create_user(login, login, None, Some("password-1"), false)).unwrap();
    h.app.with_server(|db| db.set_membership("shop", u.id, role)).unwrap();
    h.app.with_server(|db| db.create_user_token(u.id, "cli")).unwrap()
}

async fn api(h: &Harness, method: &str, path: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut c = call(&h.remote, method, path).bearer(token).no_csrf();
    if let Some(b) = body {
        c = c.json(b);
    }
    let (s, v, _) = c.send().await;
    (s, v)
}

#[tokio::test]
async fn a_project_admin_takes_the_console_and_only_its_session_takes_the_orchestrators_mail() {
    let h = Harness::new();
    h.project("shop");
    let anna = person(&h, "anna", ProjectRole::Admin);
    let pm = person(&h, "pm", ProjectRole::Member);
    let (s, _) = api(&h, "POST", "/api/orchestrator/console", &pm, Some(json!({}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a member does not take the orchestrator's place");

    let (s, v) = api(&h, "POST", "/api/orchestrator/console", &anna, Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let token = v["token"].as_str().unwrap().to_string();
    let prompt = v["prompt"].as_str().unwrap();
    assert!(prompt.contains("## The console") && prompt.contains("@anna"), "{prompt}");
    assert!(prompt.contains("| `genie_team` spawn |"), "the orchestrator's own command table");
    assert_eq!(v["console"]["user"], "anna");
    let (s, _) = api(&h, "POST", "/api/orchestrator/console", &token, Some(json!({}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "an agent does not take the console");

    // Mail for the orchestrator: a person's new task lands in the inbox.
    api(&h, "POST", "/api/tasks", &pm, Some(json!({ "title": "CSV export" }))).await;
    let server = h
        .app
        .with_server(|db| db.create_agent_token("shop", Role::Orchestrator, "orchestrator", None, None, chrono::Duration::hours(1)))
        .unwrap();
    let (s, v) = api(&h, "POST", "/api/agent/inbox/lease", &server, Some(json!({ "seen": [] }))).await;
    assert_eq!(s, StatusCode::CONFLICT, "the server's orchestrator waits: {v}");
    assert!(v["error"].as_str().unwrap().contains("held by anna"));
    let (s, v) = api(&h, "POST", "/api/agent/inbox/lease", &token, Some(json!({ "seen": [] }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["text"].as_str().unwrap_or_default().contains("CSV export"), "the console's session gets it: {v}");

    // Someone else waits for it, or takes it over.
    let bob = person(&h, "bob", ProjectRole::Owner);
    let (s, v) = api(&h, "POST", "/api/orchestrator/console", &bob, Some(json!({}))).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    let (_, v) = api(&h, "GET", "/api/orchestrator/console", &pm, None).await;
    assert_eq!(v["console"]["user"], "anna", "everyone sees who is at the console");

    // Only its own token renews it.
    let (s, v) = api(&h, "POST", "/api/orchestrator/console/renew", &token, Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, _) = api(&h, "POST", "/api/orchestrator/console/renew", &server, Some(json!({}))).await;
    assert_eq!(s, StatusCode::CONFLICT);

    // Given back: its token stops working and the server's orchestrator takes the mail again.
    let (s, v) = api(&h, "DELETE", "/api/orchestrator/console", &token, None).await;
    assert_eq!((s, &v["released"]), (StatusCode::OK, &json!(true)), "{v}");
    let (s, _) = api(&h, "POST", "/api/orchestrator/console/renew", &token, Some(json!({}))).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "the console's token is revoked");
    let (s, v) = api(&h, "POST", "/api/agent/inbox/lease", &server, Some(json!({ "seen": [] }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // Taken over, then taken away by a project admin.
    let (s, v) = api(&h, "POST", "/api/orchestrator/console", &bob, Some(json!({ "force": true }))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v) = api(&h, "DELETE", "/api/orchestrator/console", &anna, None).await;
    assert_eq!((s, &v["released"]), (StatusCode::OK, &json!(true)), "{v}");
    let (_, v) = api(&h, "GET", "/api/orchestrator/console", &pm, None).await;
    assert!(v["console"].is_null(), "{v}");
}

/// `genie orchestrate` against a live server, with a stand-in for pi that
/// records what it was given and acts through `genie` like the model would.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn genie_orchestrate_runs_pi_at_the_console_and_gives_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let mut cfg = Config::load(dir.path()).unwrap();
    cfg.port = listener.local_addr().unwrap().port();
    cfg.runtime.enabled = false;
    let app = App::open(dir.path(), cfg, PathBuf::from("/nonexistent")).unwrap();
    app.create_project("shop", "Shop", None, None, None).unwrap();
    let u = app.with_server(|db| db.create_user("anna", "Anna", None, Some("password-1"), false)).unwrap();
    app.with_server(|db| db.set_membership("shop", u.id, ProjectRole::Owner)).unwrap();
    let anna = app.with_server(|db| db.create_user_token(u.id, "cli")).unwrap();
    let (stop, rx) = tokio::sync::oneshot::channel::<()>();
    let a = app.clone();
    tokio::spawn(async move {
        genie::serve_on(a, listener, async {
            let _ = rx.await;
        })
        .await
        .unwrap();
    });

    let rec = dir.path().join("rec");
    let pi = dir.path().join("pi");
    std::fs::write(
        &pi,
        format!(
            r#"#!/usr/bin/env bash
rec={rec}
{{ printf 'arg=%s\n' "$@"; echo "token=$GENIE_TOKEN"; echo "console=$GENIE_CONSOLE"; echo "role=$GENIE_AGENT_ROLE"; }} > "$rec"
prev=
for a in "$@"; do
  case "$prev" in
    --append-system-prompt) cat "$a" > "$rec.prompt" ;;
    -e) [ -f "$a" ] && echo "ext=$(head -c 40 "$a")" >> "$rec" ;;
  esac
  prev="$a"
done
"$GENIE_BIN" me show > "$rec.me" 2>&1
"$GENIE_BIN" task create "Found by the orchestrator" > "$rec.created" 2>&1
exit 3
"#,
            rec = rec.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_genie"))
        .arg("--data")
        .arg(dir.path())
        .args(["--project", "shop", "orchestrate", "--pi"])
        .arg(&pi)
        .args(["--", "--continue"])
        .env("GENIE_URL", &url)
        .env("GENIE_TOKEN", &anna)
        .env("GENIE_BIN", env!("CARGO_BIN_EXE_genie"))
        .output()
        .await
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(3), "pi's exit code comes back: {stderr}");
    assert!(stderr.contains("you hold the orchestrator console of shop") && stderr.contains("is given back"), "{stderr}");

    let rec_text = std::fs::read_to_string(&rec).unwrap();
    for want in [
        "arg=--append-system-prompt",
        "arg=-e",
        "arg=--continue",
        "console=1",
        "role=orchestrator",
        "ext=// genie-bus",
        "ext=// genie-console",
        "token=gna_",
    ] {
        assert!(rec_text.contains(want), "{want}:\n{rec_text}");
    }
    let prompt = std::fs::read_to_string(rec.with_extension("prompt")).unwrap();
    assert!(prompt.contains("## The console") && prompt.contains("@anna"), "the orchestrator's prompt: {prompt}");
    let me = std::fs::read_to_string(rec.with_extension("me")).unwrap();
    assert_eq!(me.trim(), "agent orchestrator (orchestrator) of project shop", "pi acts with the console's token");
    let created = std::fs::read_to_string(rec.with_extension("created")).unwrap();
    assert_eq!(created.trim(), "created G-1 — Found by the orchestrator · status draft", "the orchestrator's own tasks start as drafts");

    // Given back when pi ended: no console, and its token is dead.
    assert!(app.with_server(|db| db.console("shop")).unwrap().is_none());
    let console_token = rec_text.lines().find_map(|l| l.strip_prefix("token=")).unwrap();
    assert!(app.with_server(|db| db.resolve_token(console_token)).unwrap().is_none());
    let _ = stop.send(());
}
