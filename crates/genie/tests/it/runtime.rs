//! The whole agent loop over real HTTP and processes: inbox → orchestrator →
//! team → review → done, with a scripted agent standing in for the model.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use genie::config::Config;
use genie::state::App;
use genie_core::team::{NewMember, NewTeam, SendMail};
use genie_core::work::NewJob;
use genie_core::{Activity, Actor, CreateInput, Role, Status, TeamState};
use serde_json::json;

struct Live {
    _dir: tempfile::TempDir,
    app: Arc<App>,
    _stop: tokio::sync::oneshot::Sender<()>,
}

async fn live(env: &[(&str, &str)]) -> Live {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut cfg = Config::load(dir.path()).unwrap();
    // Test files live in the data directory, which a sandboxed agent does not see.
    cfg.runtime.sandbox.mode = "off".into();
    cfg.port = port;
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-agent.sh");
    cfg.runtime.command = vec![vec!["bash".into(), script.to_string_lossy().into_owned()], vec!["{message}".into()]];
    cfg.runtime.turn_timeout_secs = 60;
    cfg.runtime.env.insert("GENIE_BIN".into(), env!("CARGO_BIN_EXE_genie").into());
    cfg.runtime.env.insert("GENIE_MARK".into(), dir.path().join("failed-once").to_string_lossy().into_owned());
    for (k, v) in env {
        cfg.runtime.env.insert(k.to_string(), v.to_string());
    }
    let app = App::open(dir.path(), cfg, PathBuf::from("/nonexistent")).unwrap();
    app.create_project("shop", "Shop", None, None, None).unwrap();
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
    Live { _dir: dir, app, _stop: tx }
}

async fn wait_for(app: &Arc<App>, what: &str, timeout: Duration, mut done: impl FnMut(&App) -> bool) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if done(app) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let turns = app.with_server(|db| db.turns("shop", None, 50)).unwrap();
    let log: Vec<String> =
        turns.iter().map(|t| format!("{} {} {:?}\n{}", t.agent, t.status, t.error, t.log.clone().unwrap_or_default())).collect();
    panic!("timed out waiting for {what}\n{}", log.join("\n---\n"));
}

fn status(app: &App) -> Status {
    app.with_tracker("shop", |t| Ok(t.get("G-1")?.status)).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inbox_to_done_through_the_orchestrator_and_a_team() {
    let l = live(&[]).await;
    l.app
        .with_tracker("shop", |t| {
            t.create(
                &Actor::new("anna", Role::Human),
                CreateInput { title: "CSV export".into(), status: Some(Status::Inbox), ..Default::default() },
            )
        })
        .unwrap();
    l.app.wake_runtime.notify_one();
    wait_for(&l.app, "task done", Duration::from_secs(90), |app| status(app) == Status::Done).await;
    let (task, team) = l.app.with_tracker("shop", |t| Ok((t.get("G-1")?, t.bus().get("G-1")?))).unwrap();
    assert!(task.acceptance.iter().all(|a| a.done));
    assert!(task.artifacts.iter().any(|a| a.name == "review.md"));
    assert_eq!(task.merge_strategy, "merge by orchestrator");
    // The closed task's team is stopped by the server, not by an agent.
    wait_for(&l.app, "team stopped", Duration::from_secs(10), |app| {
        app.with_tracker("shop", |t| Ok(t.bus().get("G-1")?.state == TeamState::Stopped)).unwrap()
    })
    .await;
    assert_eq!(team.template.as_deref(), Some("pair"));
    // The orchestrator's last turn (the one that accepted the task) may still be finishing.
    wait_for(&l.app, "the last turns to finish", Duration::from_secs(20), |app| {
        app.with_server(|db| db.turns("shop", None, 100)).unwrap().iter().all(|t| t.status != "running")
    })
    .await;
    let turns = l.app.with_server(|db| db.turns("shop", None, 100)).unwrap();
    assert!(turns.iter().all(|t| t.status == "succeeded" || t.status == "skipped"), "{turns:#?}");
    let unread: i64 = l
        .app
        .with_tracker("shop", |t| {
            Ok(t.conn()
                .query_row("SELECT COUNT(*) FROM mail WHERE delivered_at IS NULL AND recipient <> 'orchestrator'", [], |r| r.get(0))?)
        })
        .unwrap();
    assert_eq!(unread, 0, "every message was delivered exactly through a successful turn");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_crashed_turn_gives_its_mail_back_and_is_retried() {
    let l = live(&[("FAIL_ONCE", "1")]).await;
    l.app
        .with_tracker("shop", |t| {
            t.create(
                &Actor::new("anna", Role::Human),
                CreateInput { title: "CSV export".into(), status: Some(Status::Inbox), ..Default::default() },
            )
        })
        .unwrap();
    l.app.wake_runtime.notify_one();
    wait_for(&l.app, "task done after a retry", Duration::from_secs(120), |app| status(app) == Status::Done).await;
    let turns = l.app.with_server(|db| db.turns("shop", None, 100)).unwrap();
    let failed: Vec<_> = turns.iter().filter(|t| t.status == "failed").collect();
    assert_eq!(failed.len(), 1, "exactly the simulated crash failed");
    assert!(failed[0].log.as_deref().unwrap_or_default().contains("simulated crash"));
    let retried = turns.iter().filter(|t| t.agent == failed[0].agent && t.status == "succeeded").count();
    assert!(retried >= 1, "the same agent ran again and succeeded");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_servers_orchestrator_waits_while_a_person_holds_the_console() {
    let l = live(&[]).await;
    let ttl = chrono::Duration::seconds(120);
    let (_, token) = l.app.with_server(|db| db.take_console("shop", "anna", None, ttl, false)).unwrap();
    l.app
        .with_tracker("shop", |t| {
            t.create(
                &Actor::new("anna", Role::Human),
                CreateInput { title: "CSV export".into(), status: Some(Status::Inbox), ..Default::default() },
            )
        })
        .unwrap();
    l.app.wake_runtime.notify_one();
    tokio::time::sleep(Duration::from_secs(3)).await;
    let turns = l.app.with_server(|db| db.turns("shop", Some("orchestrator"), 10)).unwrap();
    assert!(turns.is_empty(), "no server orchestrator while the console is held: {turns:?}");
    assert_eq!(status(&l.app), Status::Inbox);

    // Given back: the server's orchestrator takes the mail that came meanwhile.
    l.app.with_server(|db| db.release_console("shop", Some(&token))).unwrap();
    l.app.wake_runtime.notify_one();
    wait_for(&l.app, "the orchestrator refines the task", Duration::from_secs(60), |app| status(app) != Status::Inbox).await;
}

#[test]
fn a_server_restart_tells_the_interrupted_agents_to_continue() {
    // What a killed server leaves in live-session mode: a letter the agent already acknowledged
    // (delivered) inside a turn that was still running, so `release_all_leases` has nothing to
    // offer again and the scheduler sees no pending mail to start a session for.
    let dir = tempfile::tempdir().unwrap();
    let app = app_with_team("sessions", dir.path());
    app.with_tracker("shop", |t| {
        t.bus().send(SendMail {
            team: "G-1",
            from: "anna",
            from_role: "human",
            to: "bender",
            text: "ACKED: write a1",
            kind: "owner",
            ..Default::default()
        })?;
        let d = t.bus().lease_delivery(Some("G-1"), "bender", &[], 10_000)?.expect("the letter is leased");
        t.bus().ack_delivery(d.id, "bender")?;
        t.bus().member_working("G-1", "bender", serde_json::json!({ "kind": "session" }))?;
        Ok(())
    })
    .unwrap();
    let turn = app.with_server(|db| db.start_turn("shop", "G-1/bender", Some("G-1"), Some("bender"), None)).unwrap();
    let pending = || app.with_tracker("shop", |t| t.bus().pending(Some("G-1"), "bender")).unwrap();
    let activity = || {
        app.with_tracker("shop", |t| Ok(t.bus().get("G-1")?.members.into_iter().find(|m| m.name == "bender").unwrap().activity)).unwrap()
    };

    genie::runtime::recover(&app).unwrap();

    assert_eq!(app.with_server(|db| db.turn(turn)).unwrap().status, "interrupted");
    let notes: Vec<_> = pending().into_iter().filter(|m| m.kind == "system").collect();
    assert_eq!(notes.len(), 1, "one note per stranded agent: {:?}", pending());
    assert!(notes[0].text.contains("server restarted"), "{}", notes[0].text);
    assert!(!pending().iter().any(|m| m.text.starts_with("ACKED")), "the acknowledged letter is not offered twice");
    let delivered: i64 = app
        .with_tracker("shop", |t| {
            Ok(t.conn().query_row("SELECT COUNT(*) FROM mail WHERE text LIKE 'ACKED%' AND delivered_at IS NOT NULL", [], |r| r.get(0))?)
        })
        .unwrap();
    assert_eq!(delivered, 1, "the letter the agent saw is still delivered, not lost");
    assert_eq!(activity(), Activity::Idle, "the board does not show a busy agent without a process");

    // A server restarted in a loop adds no second note while the first one is unread.
    app.with_server(|db| db.start_turn("shop", "G-1/bender", Some("G-1"), Some("bender"), None)).unwrap();
    genie::runtime::recover(&app).unwrap();
    assert_eq!(pending().iter().filter(|m| m.kind == "system").count(), 1, "an unread note is enough: {:?}", pending());

    // An interrupted job is skipped: `requeue_running_jobs` re-runs it.
    let job = app
        .with_server(|db| {
            db.create_job(NewJob {
                project: "shop".into(),
                role: "executor".into(),
                goal: "audit the export".into(),
                inputs: json!({}),
                workspace: "none".into(),
                ..Default::default()
            })
        })
        .unwrap();
    app.with_server(|db| db.start_job(job.id)).unwrap();
    app.with_server(|db| db.start_turn("shop", &format!("job/{}", job.id), None, None, Some(job.id))).unwrap();
    genie::runtime::recover(&app).unwrap();
    let to_orchestrator = app.with_tracker("shop", |t| t.bus().pending(None, "orchestrator")).unwrap();
    assert!(to_orchestrator.iter().all(|m| m.kind != "system"), "a job turn gets no note: {to_orchestrator:?}");

    // `turns` mode: the interrupted turn released its lease, so its mail runs again — no note.
    let dir = tempfile::tempdir().unwrap();
    let app = app_with_team("turns", dir.path());
    let turn = app.with_server(|db| db.start_turn("shop", "G-1/bender", Some("G-1"), Some("bender"), None)).unwrap();
    app.with_tracker("shop", |t| {
        t.bus().send(SendMail {
            team: "G-1",
            from: "anna",
            from_role: "human",
            to: "bender",
            text: "LEASED: write a1",
            kind: "owner",
            ..Default::default()
        })?;
        t.bus().lease(Some("G-1"), "bender", turn)?;
        Ok(())
    })
    .unwrap();
    genie::runtime::recover(&app).unwrap();
    let pending = app.with_tracker("shop", |t| t.bus().pending(Some("G-1"), "bender")).unwrap();
    assert_eq!(pending.len(), 1, "the leased letter is offered again: {pending:?}");
    assert_eq!(pending[0].text, "LEASED: write a1");
    assert!(pending.iter().all(|m| m.kind != "system"), "turns mode needs no note");
}

/// An app in `mode` with project `shop`, task `G-1` and a one-member team (`bender`); the
/// runtime is disabled so no scheduler runs while a test drives recovery by hand.
fn app_with_team(mode: &str, dir: &std::path::Path) -> Arc<App> {
    let mut cfg = Config::load(dir).unwrap();
    // Test files live in the data directory, which a sandboxed agent does not see.
    cfg.runtime.sandbox.mode = "off".into();
    cfg.runtime.mode = mode.into();
    cfg.runtime.enabled = false;
    let app = App::open(dir, cfg, PathBuf::from("/nonexistent")).unwrap();
    app.create_project("shop", "Shop", None, None, None).unwrap();
    app.with_tracker("shop", |t| {
        t.create(&Actor::new("anna", Role::Human), CreateInput { title: "CSV export".into(), ..Default::default() })?;
        t.bus().create(
            "anna",
            "human",
            NewTeam {
                id: "G-1".into(),
                task: "G-1".into(),
                cwd: ".".into(),
                members: vec![NewMember { name: "bender".into(), role: "executor".into(), ..Default::default() }],
                ..Default::default()
            },
        )?;
        Ok(())
    })
    .unwrap();
    app
}

#[test]
fn recovery_stops_only_verified_stray_agent_processes() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::load(dir.path()).unwrap();
    // Test files live in the data directory, which a sandboxed agent does not see.
    cfg.runtime.sandbox.mode = "off".into();
    cfg.runtime.enabled = false;
    let app = App::open(dir.path(), cfg, PathBuf::from("/nonexistent")).unwrap();
    app.create_project("shop", "", None, None, None).unwrap();
    let spawn = |env: &[(&str, &str)]| std::process::Command::new("sleep").arg("30").envs(env.iter().copied()).spawn().unwrap();
    let mut stray = spawn(&[("GENIE_PROJECT", "shop"), ("GENIE_AGENT_NAME", "bender")]);
    let mut other = spawn(&[]);
    app.with_server(|db| {
        let a = db.start_turn("shop", "G-1/bender", Some("G-1"), Some("bender"), None)?;
        db.set_turn_pid(a, stray.id())?;
        let b = db.start_turn("shop", "G-1/yoda", Some("G-1"), Some("yoda"), None)?;
        db.set_turn_pid(b, other.id())
    })
    .unwrap();
    genie::runtime::recover(&app).unwrap();
    // Stopping is a signal: give the process a moment to go, even on a busy machine.
    let start = Instant::now();
    while stray.try_wait().unwrap().is_none() && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(stray.try_wait().unwrap().is_some(), "the stray agent was stopped");
    assert!(other.try_wait().unwrap().is_none(), "a process that is not that agent is left alone");
    other.kill().unwrap();
}
