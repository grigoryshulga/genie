//! The whole agent loop over real HTTP and processes: inbox → orchestrator →
//! team → review → done, with a scripted agent standing in for the model.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use genie::config::Config;
use genie::state::App;
use genie_core::conversations::{Message, Response, ResponseStatus, Submission};
use genie_core::server_db::ProjectRole;
use genie_core::team::{CHAT_MAIL, NewMember, NewTeam, ORCHESTRATOR, SendMail};
use genie_core::work::NewJob;
use genie_core::{Activity, Actor, CreateInput, Role, Status, TeamState};
use serde_json::json;

struct Live {
    _dir: tempfile::TempDir,
    app: Arc<App>,
    _stop: tokio::sync::oneshot::Sender<()>,
}

async fn live(env: &[(&str, &str)]) -> Live {
    live_cfg(env, |_| {}).await
}

async fn live_cfg(env: &[(&str, &str)], f: impl FnOnce(&mut Config)) -> Live {
    live_with_clock(env, f, Arc::new(genie::clock::SystemClock)).await
}

async fn live_with_clock(env: &[(&str, &str)], f: impl FnOnce(&mut Config), clock: Arc<dyn genie::clock::Clock>) -> Live {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut cfg = Config::load(dir.path()).unwrap();
    cfg.vault.commit = Some(false);
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
    f(&mut cfg);
    crate::common::give_roles_models(dir.path());
    let app = App::open_with_clock(dir.path(), cfg, PathBuf::from("/nonexistent"), clock).unwrap();
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
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let turns = app.with_server(|db| db.turns("shop", None, 50)).unwrap();
    let log: Vec<String> =
        turns.iter().map(|t| format!("{} {} {:?}\n{}", t.agent, t.status, t.error, t.log.clone().unwrap_or_default())).collect();
    panic!("timed out waiting for {what}\n{}", log.join("\n---\n"));
}

fn status(app: &App) -> Status {
    app.with_tracker("shop", |t| Ok(t.get("G-1")?.status)).unwrap()
}

async fn incoming(mode: &str) -> Live {
    live_cfg(&[("REQUEST_MODE", mode)], |cfg| {
        // Explicit ticks make queue boundaries observable. Incoming requests must
        // still use one-shot workers when the rest of the project uses sessions.
        cfg.runtime.enabled = false;
        cfg.runtime.mode = "sessions".into();
        cfg.runtime.max_concurrent = 2;
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/request-agent.sh");
        cfg.runtime.command = vec![vec!["bash".into(), script.to_string_lossy().into_owned()], vec!["{message}".into()]];
    })
    .await
}

fn requester(app: &App, login: &str) -> i64 {
    app.with_server(|db| {
        let u = db.create_user(login, login, None, None, false)?;
        db.set_membership("shop", u.id, ProjectRole::Member)?;
        Ok(u.id)
    })
    .unwrap()
}

fn incoming_response(app: &App, user: i64, text: &str, previous: Option<&Response>) -> Response {
    app.with_tracker("shop", |t| {
        t.conversations().submit(Submission {
            principal: format!("user:{user}"),
            initiator: None,
            conversation_id: previous.map(|r| r.conversation_id.clone()),
            previous_response_id: previous.map(|r| r.id.clone()),
            input: vec![Message { role: "user".into(), content: text.into() }],
            idempotency_key: None,
            fingerprint: text.into(),
        })
    })
    .unwrap()
}

fn response(app: &App, id: &str) -> Response {
    app.with_tracker("shop", |t| t.conversations().get(id)).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn incoming_requests_run_concurrently_with_private_history_and_no_manager_session() {
    let l = incoming("wait").await;
    let anna = requester(&l.app, "anna");
    let boris = requester(&l.app, "boris");
    let first = incoming_response(&l.app, anna, "private-anna-first", None);
    let other = incoming_response(&l.app, boris, "private-boris-first", None);
    genie::runtime::tick(&l.app).await.unwrap();
    let capture = l._dir.path().join("requests");
    wait_for(&l.app, "both independent request processes", Duration::from_secs(20), |app| {
        capture.join(format!("{}.cwd", first.id)).exists()
            && capture.join(format!("{}.cwd", other.id)).exists()
            && app.sched.snapshot(2, app.slots.available_permits()).running == 2
    })
    .await;
    let first_cwd = std::fs::read_to_string(capture.join(format!("{}.cwd", first.id))).unwrap();
    let other_cwd = std::fs::read_to_string(capture.join(format!("{}.cwd", other.id))).unwrap();
    assert_ne!(first_cwd, other_cwd);
    assert_eq!(l.app.slots.available_permits(), 0, "requests may use the manager's reserved slot");
    std::fs::write(capture.join("release"), "go").unwrap();
    wait_for(&l.app, "both persisted replies", Duration::from_secs(20), |app| {
        response(app, &first.id).status.is_terminal()
            && response(app, &other.id).status.is_terminal()
            && app.sched.snapshot(2, app.slots.available_permits()).running == 0
    })
    .await;
    let first = response(&l.app, &first.id);
    let other = response(&l.app, &other.id);
    assert_eq!(first.status, ResponseStatus::Completed, "{:?}", first.error);
    assert_eq!(other.status, ResponseStatus::Completed, "{:?}", other.error);
    assert!(first.output.contains("private-anna-first"));
    assert!(!first.output.contains("private-boris-first"));
    assert!(!other.output.contains("private-anna-first"));

    let next = incoming_response(&l.app, anna, "private-anna-next", Some(&first));
    genie::runtime::tick(&l.app).await.unwrap();
    wait_for(&l.app, "reply with only its own earlier context", Duration::from_secs(20), |app| {
        response(app, &next.id).status.is_terminal() && app.sched.snapshot(2, app.slots.available_permits()).running == 0
    })
    .await;
    let next = response(&l.app, &next.id);
    assert_eq!(next.status, ResponseStatus::Completed, "{:?}", next.error);
    let answer = next.output;
    assert!(answer.contains("private-anna-first"));
    assert!(answer.contains("private-anna-next"));
    assert!(!answer.contains("private-boris-first"));
    assert!(manager_turns(&l.app).is_empty());
    let turns = l.app.with_server(|db| db.turns("shop", None, 10)).unwrap();
    assert_eq!(turns.len(), 3);
    assert!(turns.iter().all(|t| t.agent.starts_with("request:") && t.status == "succeeded"));
    assert!(turns.iter().all(|t| t.log.is_none() && t.error.is_none()), "shared turn records must not contain private messages");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn incoming_large_unicode_continuations_reach_the_process_without_argv_limits() {
    for delivery in ["stdin", "file"] {
        let l = live_cfg(&[("REQUEST_INPUT", delivery)], |cfg| {
            cfg.runtime.enabled = false;
            let data = PathBuf::from(&cfg.runtime.env["GENIE_MARK"]).parent().unwrap().to_path_buf();
            std::fs::create_dir_all(data.join("agents")).unwrap();
            std::fs::write(data.join("agents/intake.md"), "---\nfiles: none\n---\n").unwrap();
            let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/request-agent.sh");
            cfg.runtime.command = if delivery == "stdin" {
                // A stand-in for Pi's documented stdin transport, without model/network calls.
                let pi = data.join("pi");
                std::fs::copy(script, &pi).unwrap();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o700)).unwrap();
                }
                vec![vec![pi.to_string_lossy().into_owned()], vec!["{message}".into()]]
            } else {
                vec![
                    vec!["bash".into(), script.to_string_lossy().into_owned()],
                    vec!["--message-file".into(), "{messageFile}".into()],
                    vec!["{message}".into()],
                ]
            };
        })
        .await;
        assert_eq!(l.app.agents().roles["intake"].files, genie::agent_config::FileAccess::None);
        let user = requester(&l.app, "anna");
        let earlier = "Ранее🙂\\\"\n".repeat(900);
        let first = incoming_response(&l.app, user, &earlier, None);
        genie::runtime::tick(&l.app).await.unwrap();
        wait_for(&l.app, "the earlier Unicode reply", Duration::from_secs(20), |app| {
            response(app, &first.id).status.is_terminal() && app.sched.snapshot(4, app.slots.available_permits()).running == 0
        })
        .await;
        assert_eq!(response(&l.app, &first.id).status, ResponseStatus::Completed);
        let memory = "confirmed-project-context ".repeat(1100);
        l.app.with_tracker("shop", |t| t.management().remember(&Actor::new("anna", Role::Human), &memory, "owner", None)).unwrap();
        let input = "Я🙂\\\"\n".repeat(13_500);
        assert!(input.len() < 128 * 1024, "valid OpenAI input before JSON escaping");
        let next = incoming_response(&l.app, user, &input, Some(&first));
        genie::runtime::tick(&l.app).await.unwrap();
        wait_for(&l.app, "the complete oversized continuation", Duration::from_secs(20), |app| {
            response(app, &next.id).status.is_terminal() && app.sched.snapshot(4, app.slots.available_permits()).running == 0
        })
        .await;
        let next = response(&l.app, &next.id);
        assert_eq!(next.status, ResponseStatus::Completed, "{delivery}: {:?}", next.error);
        let capture = l._dir.path().join("requests");
        let message = std::fs::read_to_string(capture.join(format!("{}.message", next.id))).unwrap();
        assert!(message.len() > 128 * 1024, "the old single-argument delivery could not spawn this request");
        assert_eq!(message, next.output);
        let context: serde_json::Value = serde_json::from_str(message.lines().last().unwrap()).unwrap();
        assert_eq!(context["input"][0]["content"], input);
        assert_eq!(context["history"][0]["input"][0]["content"], earlier);
        assert_eq!(context["project_memory"][0]["text"], memory);
        let argument = std::fs::read_to_string(capture.join(format!("{}.argument", next.id))).unwrap();
        assert!(argument.len() < 4096);
        assert!(!argument.contains(&input));
        if delivery == "file" {
            assert!(argument.contains(&format!("genie conversation show {}", next.id)));
            assert!(argument.contains("genie manager memory"));
        }
        let private_input = l.app.data.join("runtime/shop").join(format!("request:{}", next.id)).join("input.txt");
        assert_eq!(std::fs::read_to_string(&private_input).unwrap(), message);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(private_input).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(l.app.with_server(|db| db.turns("shop", None, 10)).unwrap().iter().all(|turn| turn.log.is_none()));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn incoming_workers_must_persist_the_reply_and_process_failures_do_not_loop() {
    for mode in ["no_reply", "crash"] {
        let l = incoming(mode).await;
        let user = requester(&l.app, "anna");
        let request = incoming_response(&l.app, user, "answer this", None);
        genie::runtime::tick(&l.app).await.unwrap();
        wait_for(&l.app, "a visibly failed response", Duration::from_secs(20), |app| {
            response(app, &request.id).status.is_terminal() && app.sched.snapshot(2, app.slots.available_permits()).running == 0
        })
        .await;
        let request = response(&l.app, &request.id);
        assert_eq!(request.status, ResponseStatus::Failed);
        assert!(request.output.is_empty());
        assert!(request.error.as_deref().unwrap().contains(if mode == "no_reply" { "conversation reply" } else { "exit code" }));
        genie::runtime::tick(&l.app).await.unwrap();
        let turns = l.app.with_server(|db| db.turns("shop", None, 10)).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].status, "failed");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_an_incoming_response_stops_its_process_and_preserves_cancellation() {
    let l = incoming("wait").await;
    let user = requester(&l.app, "anna");
    let request = incoming_response(&l.app, user, "will be cancelled", None);
    genie::runtime::tick(&l.app).await.unwrap();
    wait_for(&l.app, "the request process", Duration::from_secs(20), |_| {
        l._dir.path().join("requests").join(format!("{}.cwd", request.id)).exists()
    })
    .await;
    l.app.with_tracker("shop", |t| t.conversations().cancel(&request.id)).unwrap();
    genie::runtime::cancel_request(&l.app, "shop", &request.id);
    wait_for(&l.app, "cancelled worker to release its slot", Duration::from_secs(5), |app| {
        app.sched.snapshot(2, app.slots.available_permits()).running == 0
    })
    .await;
    assert_eq!(response(&l.app, &request.id).status, ResponseStatus::Cancelled);
    assert!(response(&l.app, &request.id).output.is_empty());
    let turns = l.app.with_server(|db| db.turns("shop", None, 10)).unwrap();
    assert_eq!(turns[0].error.as_deref(), Some("incoming response failed; see its private receipt"));
    assert!(turns[0].log.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_incoming_request_rechecks_the_initiators_membership_before_spawning() {
    let l = incoming("reply").await;
    let user = requester(&l.app, "anna");
    let request = incoming_response(&l.app, user, "queued before access changed", None);
    l.app.with_server(|db| db.remove_membership("shop", user)).unwrap();
    genie::runtime::tick(&l.app).await.unwrap();
    wait_for(&l.app, "revoked request to fail", Duration::from_secs(10), |app| response(app, &request.id).status.is_terminal()).await;
    let request = response(&l.app, &request.id);
    assert_eq!(request.status, ResponseStatus::Failed);
    assert!(l.app.with_server(|db| db.turns("shop", None, 10)).unwrap().is_empty());
    assert!(!l._dir.path().join("requests").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_interrupted_incoming_response_reuses_its_id_and_revokes_the_previous_worker() {
    let l = incoming("reply").await;
    let user = requester(&l.app, "anna");
    let request = incoming_response(&l.app, user, "recover me", None);
    l.app.with_tracker("shop", |t| t.conversations().claim(&request.id)).unwrap();
    let name = format!("request:{}", request.id);
    let old_token = l
        .app
        .with_server(|db| {
            db.start_turn("shop", &name, None, None, None)?;
            db.create_role_token("shop", Role::Analyst, Some("intake"), &name, None, None, chrono::Duration::hours(1))
        })
        .unwrap();
    genie::runtime::recover(&l.app).unwrap();
    assert_eq!(response(&l.app, &request.id).status, ResponseStatus::Queued);
    assert!(l.app.with_server(|db| db.resolve_token(&old_token)).unwrap().is_none());
    genie::runtime::tick(&l.app).await.unwrap();
    wait_for(&l.app, "recovered request's reply", Duration::from_secs(20), |app| response(app, &request.id).status.is_terminal()).await;
    assert_eq!(response(&l.app, &request.id).status, ResponseStatus::Completed);
    assert_eq!(l.app.with_tracker("shop", |t| t.conversations().history(&request.conversation_id)).unwrap().len(), 1);
    assert!(manager_turns(&l.app).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_last_slot_stays_free_for_the_orchestrator() {
    // Three slow members, three slots: two run, the third waits, and the last
    // slot stays free — so the orchestrator that wakes up into a full scheduler
    // starts at once instead of starving until a member finishes (G-83).
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/slow-agent.sh");
    let l = live_cfg(&[], |cfg| {
        cfg.runtime.max_concurrent = 3;
        cfg.runtime.command = vec![vec!["bash".into(), script.to_string_lossy().into_owned()], vec!["{message}".into()]];
        cfg.runtime.env.insert("GENIE_SLEEP".into(), "8".into());
    })
    .await;
    l.app.with_server(|db| db.set_autonomy("shop", "manual")).unwrap();
    l.app
        .with_tracker("shop", |t| {
            for n in 1..=3 {
                let task = t.create(
                    &Actor::new("anna", Role::Human),
                    CreateInput { title: format!("load {n}"), status: Some(Status::Ready), ..Default::default() },
                )?;
                t.bus().create(
                    "anna",
                    "human",
                    NewTeam {
                        id: task.id.clone(),
                        task: task.id.clone(),
                        cwd: "/tmp".into(),
                        members: vec![NewMember { name: format!("m{n}"), role: "executor".into(), ..Default::default() }],
                        ..Default::default()
                    },
                )?;
                t.bus().send(SendMail {
                    team: &task.id,
                    from: "anna",
                    from_role: "human",
                    to: &format!("m{n}"),
                    text: "work",
                    kind: "owner",
                    ..Default::default()
                })?;
            }
            Ok(())
        })
        .unwrap();
    l.app.wake_runtime.notify_one();
    wait_for(&l.app, "two members running, one waiting, one slot free", Duration::from_secs(30), |app| {
        let v = app.sched.snapshot(3, app.slots.available_permits());
        v.running == 2 && v.members_waiting == 1 && v.orchestrators_waiting == 0
    })
    .await;
    // The orchestrator gets mail and starts while both members still hold their slots.
    l.app.with_server(|db| db.set_autonomy("shop", "autonomous")).unwrap();
    l.app
        .with_tracker("shop", |t| {
            t.bus().notify_orchestrator("anna", "human", "owner", "decide something", None)?;
            Ok(())
        })
        .unwrap();
    l.app.wake_runtime.notify_one();
    wait_for(&l.app, "the orchestrator running on the reserved slot", Duration::from_secs(30), |app| {
        app.sched.snapshot(3, app.slots.available_permits()).running == 3
    })
    .await;
    // The third member still waits, now with no slot left; the orchestrator is not in the queue.
    wait_for(&l.app, "the queue after the orchestrator started", Duration::from_secs(5), |app| {
        let v = app.sched.snapshot(3, app.slots.available_permits());
        v.running == 3 && v.permits == 0 && v.members_waiting == 1 && v.orchestrators_waiting == 0
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_orchestrator_waiting_for_a_slot_is_counted() {
    // One slot, held by a slow member: the orchestrator that gets mail waits, and
    // the queue counters say so (the key whose slot was refused is in the queue).
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/slow-agent.sh");
    let l = live_cfg(&[], |cfg| {
        cfg.runtime.max_concurrent = 1;
        cfg.runtime.command = vec![vec!["bash".into(), script.to_string_lossy().into_owned()], vec!["{message}".into()]];
        cfg.runtime.env.insert("GENIE_SLEEP".into(), "8".into());
    })
    .await;
    l.app.with_server(|db| db.set_autonomy("shop", "manual")).unwrap();
    l.app
        .with_tracker("shop", |t| {
            let task = t.create(
                &Actor::new("anna", Role::Human),
                CreateInput { title: "load".into(), status: Some(Status::Ready), ..Default::default() },
            )?;
            t.bus().create(
                "anna",
                "human",
                NewTeam {
                    id: task.id.clone(),
                    task: task.id.clone(),
                    cwd: "/tmp".into(),
                    members: vec![NewMember { name: "m1".into(), role: "executor".into(), ..Default::default() }],
                    ..Default::default()
                },
            )?;
            t.bus().send(SendMail {
                team: &task.id,
                from: "anna",
                from_role: "human",
                to: "m1",
                text: "work",
                kind: "owner",
                ..Default::default()
            })?;
            Ok(())
        })
        .unwrap();
    l.app.wake_runtime.notify_one();
    wait_for(&l.app, "the member holding the only slot", Duration::from_secs(30), |app| {
        app.sched.snapshot(1, app.slots.available_permits()).running == 1
    })
    .await;
    l.app.with_server(|db| db.set_autonomy("shop", "autonomous")).unwrap();
    l.app
        .with_tracker("shop", |t| {
            t.bus().notify_orchestrator("anna", "human", "owner", "decide something", None)?;
            Ok(())
        })
        .unwrap();
    l.app.wake_runtime.notify_one();
    wait_for(&l.app, "the orchestrator in the queue", Duration::from_secs(5), |app| {
        let v = app.sched.snapshot(1, app.slots.available_permits());
        v.running == 1 && v.orchestrators_waiting == 1 && v.members_waiting == 0
    })
    .await;
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
    wait_for(&l.app, "mandatory retrospective completed", Duration::from_secs(20), |app| {
        app.with_tracker("shop", |t| Ok(t.management().manager("G-1")?.is_some_and(|m| m.state == "completed"))).unwrap()
    })
    .await;
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
    let clock = Arc::new(crate::common::ManualClock::default());
    let l = live_with_clock(&[("FAIL_ONCE", "1")], |_| {}, clock.clone()).await;
    l.app
        .with_tracker("shop", |t| {
            t.create(
                &Actor::new("anna", Role::Human),
                CreateInput { title: "CSV export".into(), status: Some(Status::Inbox), ..Default::default() },
            )
        })
        .unwrap();
    l.app.wake_runtime.notify_one();
    let mut crashed = None;
    wait_for(&l.app, "the simulated crash", Duration::from_secs(30), |app| {
        crashed = app.with_server(|db| db.turns("shop", None, 100)).unwrap().into_iter().find(|turn| turn.status == "failed");
        crashed.is_some()
    })
    .await;
    let crashed = crashed.unwrap();
    let key = genie::runtime::AgentKey::Member {
        project: "shop".into(),
        team: crashed.team.expect("the crashed executor belongs to a team"),
        member: crashed.member.expect("the crashed turn belongs to the executor"),
    };
    wait_for(&l.app, "the failed turn's backoff", Duration::from_secs(30), |app| app.attempts.failures(&key) == 1).await;
    genie::runtime::tick(&l.app).await.unwrap();
    assert!(!l.app.attempts.may_start(&key), "failed turns wait for the retry deadline");
    clock.advance(Duration::from_secs(1));
    l.app.wake_runtime.notify_one();
    wait_for(&l.app, "task done after a retry", Duration::from_secs(30), |app| status(app) == Status::Done).await;
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
    genie::engine::tick(&l.app).unwrap();
    genie::runtime::tick(&l.app).await.unwrap();
    assert_eq!(
        l.app.sched.snapshot(l.app.cfg.runtime.max_concurrent, l.app.slots.available_permits()).running,
        0,
        "no orchestrator scheduled while the console is held"
    );
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

#[test]
fn a_manual_managers_restart_note_is_chat_that_wakes_it_and_nothing_else_does() {
    // A manual project's manager takes chat alone, so the note that continues its interrupted turn is
    // chat from genie: it is delivered to the manager and starts it, where a system note would wait.
    let dir = tempfile::tempdir().unwrap();
    let app = app_with_team("sessions", dir.path());
    app.with_server(|db| db.set_autonomy("shop", "manual")).unwrap();
    app.with_tracker("shop", |t| {
        t.bus().notify_manager(ORCHESTRATOR, "anna", "human", CHAT_MAIL, "@pm status?", None)?;
        let d = t.bus().lease_delivery_kind(None, ORCHESTRATOR, &[], 10_000, Some(CHAT_MAIL))?.expect("the question is leased");
        t.bus().ack_delivery(d.id, ORCHESTRATOR)?;
        Ok(())
    })
    .unwrap();
    let turn = app.with_server(|db| db.start_turn("shop", ORCHESTRATOR, None, None, None)).unwrap();
    let project = app.with_server(|db| db.project("shop")).unwrap();
    assert!(!genie::runtime::manager_starts(&app, &project), "the answered question does not wake it again");

    genie::runtime::recover(&app).unwrap();

    assert_eq!(app.with_server(|db| db.turn(turn)).unwrap().status, "interrupted");
    let pending = || app.with_tracker("shop", |t| t.bus().pending(None, ORCHESTRATOR)).unwrap();
    let notes: Vec<_> = pending().into_iter().filter(|m| m.from == "genie").collect();
    assert_eq!(notes.len(), 1, "one note for the manager: {:?}", pending());
    assert_eq!(notes[0].kind, CHAT_MAIL, "the manager takes chat only, so its note is chat");
    assert!(notes[0].text.contains("server restarted"), "{}", notes[0].text);
    assert!(genie::runtime::manager_starts(&app, &project), "the note wakes the manager");

    // A restart in a loop adds no second note while the first one is unread.
    app.with_server(|db| db.start_turn("shop", ORCHESTRATOR, None, None, None)).unwrap();
    genie::runtime::recover(&app).unwrap();
    assert_eq!(pending().iter().filter(|m| m.from == "genie").count(), 1, "an unread note is enough: {:?}", pending());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_manual_managers_turn_takes_chat_and_leaves_the_rest_queued() {
    // The one-shot path: the manager's turn leases chat only; the system note waits untouched.
    let l = live_cfg(&[], |cfg| cfg.runtime.mode = "turns".into()).await;
    l.app.with_server(|db| db.set_autonomy("shop", "manual")).unwrap();
    l.app
        .with_tracker("shop", |t| {
            t.bus().notify_manager(ORCHESTRATOR, "genie", "system", "system", "AUTOMATION: a note nobody asked for", None)?;
            t.bus().notify_manager(ORCHESTRATOR, "anna", "human", CHAT_MAIL, "@pm status of the export?", None)?;
            Ok(())
        })
        .unwrap();
    l.app.wake_runtime.notify_one();
    wait_for(&l.app, "the manager's turn to answer the chat", Duration::from_secs(60), |app| {
        app.with_server(|db| db.turns("shop", Some("orchestrator"), 10)).unwrap().iter().any(|t| t.status == "succeeded")
    })
    .await;

    let pending = l.app.with_tracker("shop", |t| t.bus().pending(None, ORCHESTRATOR)).unwrap();
    assert_eq!(pending.len(), 1, "only the chat was taken: {pending:?}");
    assert!(pending[0].text.starts_with("AUTOMATION"), "{pending:?}");
    assert!(pending[0].delivered_at.is_none(), "the note is still queued");
    let project = l.app.with_server(|db| db.project("shop")).unwrap();
    assert!(!genie::runtime::manager_starts(&l.app, &project), "a note alone does not start a manual manager");
    let turns = l.app.with_server(|db| db.turns("shop", Some("orchestrator"), 10)).unwrap();
    assert_eq!(turns.len(), 1, "one turn, for the chat: {turns:?}");
}

/// An app in `mode` with project `shop`, task `G-1` and a one-member team (`bender`); the
/// runtime is disabled so no scheduler runs while a test drives recovery by hand.
fn app_with_team(mode: &str, dir: &std::path::Path) -> Arc<App> {
    let mut cfg = Config::load(dir).unwrap();
    // Test files live in the data directory, which a sandboxed agent does not see.
    cfg.runtime.sandbox.mode = "off".into();
    cfg.runtime.mode = mode.into();
    cfg.runtime.enabled = false;
    crate::common::give_roles_models(dir);
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
    crate::common::give_roles_models(dir.path());
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

/// The turns the project's manager has run (or is running), as their statuses.
fn manager_turns(app: &App) -> Vec<String> {
    app.with_server(|db| db.turns("shop", None, 50)).unwrap().into_iter().filter(|t| t.agent == ORCHESTRATOR).map(|t| t.status).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_manual_projects_manager_runs_for_chat_mail_and_not_for_the_owners() {
    // Owner mail waits for the person in a manual project; chat mail, a tag of the manager, starts it.
    let l = live(&[]).await;
    l.app.with_server(|db| db.set_autonomy("shop", "manual")).unwrap();
    l.app.with_tracker("shop", |t| t.bus().notify_orchestrator("anna", "human", "owner", "decide something", None)).unwrap();
    l.app.wake_runtime.notify_one();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(manager_turns(&l.app).is_empty(), "owner mail does not start a manual project's manager");

    l.app
        .with_tracker("shop", |t| {
            t.bus().notify_manager(ORCHESTRATOR, "anna", "human", CHAT_MAIL, "Project chat message #1 from @anna:\n\n@pm status?", None)
        })
        .unwrap();
    l.app.wake_runtime.notify_one();
    wait_for(&l.app, "the manager's turn for chat mail", Duration::from_secs(30), |app| !manager_turns(app).is_empty()).await;
}
