//! The owner's two scenarios end to end, over real HTTP, processes, the engine
//! and a fake Telegram Bot API:
//! 1. a PM files a task → an analyst job drafts questions → the PM answers in
//!    Telegram → the answers land in the task and wake the orchestrator;
//! 2. a task is done → a documenter job updates the knowledge base (as a
//!    proposal, per section policy) → a changelog entry → the owners are told.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Path, State};
use axum::routing::post;
use genie::config::{Config, TelegramConfig};
use genie::state::App;
use genie_core::server_db::ProjectRole;
use genie_core::{Actor, CreateInput, Role, Status, StatusOptions};
use serde_json::{Value, json};

type Sent = Arc<Mutex<Vec<(String, Value)>>>;

async fn fake_telegram() -> (String, Sent) {
    let sent: Sent = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    async fn handle(State(sent): State<Sent>, Path((_bot, method)): Path<(String, String)>, Json(body): Json<Value>) -> Json<Value> {
        let result = match method.as_str() {
            "getUpdates" => {
                tokio::time::sleep(Duration::from_millis(300)).await;
                json!([])
            }
            "getMe" => json!({ "username": "genie_test_bot" }),
            "sendMessage" => {
                let mut s = sent.lock().unwrap();
                s.push((method.clone(), body.clone()));
                json!({ "message_id": 1000 + s.len() })
            }
            _ => json!(true),
        };
        Json(json!({ "ok": true, "result": result }))
    }
    let router = axum::Router::new().route("/{bot}/{method}", post(handle)).with_state(sent.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (base, sent)
}

struct Live {
    _dir: tempfile::TempDir,
    app: Arc<App>,
    _stop: tokio::sync::oneshot::Sender<()>,
}

async fn live(telegram: Option<String>) -> Live {
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = Config::load(dir.path()).unwrap();
    // Test files live in the data directory, which a sandboxed agent does not see.
    cfg.runtime.sandbox.mode = "off".into();
    cfg.port = listener.local_addr().unwrap().port();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-agent.sh");
    cfg.runtime.command = vec![vec!["bash".into(), script.to_string_lossy().into_owned()], vec!["{message}".into()]];
    cfg.runtime.turn_timeout_secs = 60;
    cfg.runtime.env.insert("GENIE_BIN".into(), env!("CARGO_BIN_EXE_genie").into());
    cfg.telegram = telegram.map(|api| TelegramConfig { token: "TEST".into(), api_base: Some(api) });
    let app = App::open(dir.path(), cfg, PathBuf::from("/nonexistent")).unwrap();
    app.create_project("shop", "Магазин", None, None, Some("SHOP")).unwrap();
    // Owner's answer: no automatic orchestrator in these tests, the scenario is the automation.
    app.with_server(|db| db.set_autonomy("shop", "manual")).unwrap();
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

async fn wait_for(app: &Arc<App>, what: &str, secs: u64, mut done: impl FnMut(&App) -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(secs) {
        if done(app) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let runs = app.with_server(|db| db.runs("shop", None, 20)).unwrap();
    let mut dump = Vec::new();
    for r in &runs {
        let steps = app.with_server(|db| db.steps(r.id)).unwrap();
        dump.push(format!(
            "run {} {} {:?}\n{:#?}",
            r.id,
            r.status,
            r.error,
            steps.iter().map(|s| (&s.step_id, &s.status, &s.error)).collect::<Vec<_>>()
        ));
    }
    let turns = app.with_server(|db| db.turns("shop", None, 20)).unwrap();
    dump.extend(turns.iter().map(|t| format!("turn {} {} {:?}\n{}", t.agent, t.status, t.error, t.log.clone().unwrap_or_default())));
    panic!("timed out waiting for {what}\n{}", dump.join("\n---\n"));
}

fn install(app: &App, name: &str) {
    let spec = genie::engine::playbooks().into_iter().find(|(id, ..)| *id == name).unwrap().2;
    app.with_server(|db| db.create_automation("shop", &spec, "anna")).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn new_task_triage_asks_the_author_in_telegram() {
    let (tg, sent) = fake_telegram().await;
    let l = live(Some(tg)).await;
    let app = &l.app;
    let pm = app
        .with_server(|db| {
            let pm = db.create_user("pm", "Продакт", None, Some("password-1"), false)?;
            db.set_membership("shop", pm.id, ProjectRole::Member)?;
            db.link_channel(pm.id, "telegram", "4242")?;
            // The triage agent works for the PM and runs with their LiteLLM key.
            db.set_user_secret(pm.id, genie_core::secrets::LITELLM, "sk-pm-0123456789")?;
            Ok(pm)
        })
        .unwrap();
    install(app, "new-task-triage");

    // The PM files a task (through Telegram: /new …).
    let update = json!({ "update_id": 1, "message": { "message_id": 1, "chat": { "id": 4242 }, "text": "/new Экспорт заказов\nНужна выгрузка для бухгалтерии" } });
    genie::channels::handle_update(app, &update).await.unwrap();
    let task = app.with_tracker("shop", |t| t.get("SHOP-1")).unwrap();
    assert!(matches!(task.status, Status::Inbox | Status::Refining), "{:?}", task.status);
    assert_eq!(task.history[0].actor, "pm");

    wait_for(app, "questions sent to Telegram", 60, |_| {
        sent.lock().unwrap().iter().any(|(_, b)| b["text"].as_str().unwrap_or_default().contains("Кто получает"))
    })
    .await;
    let qn = app.with_server(|db| db.questionnaires_for(pm.id, true)).unwrap().remove(0);
    assert_eq!(qn.questions.len(), 2);
    assert_eq!(app.with_tracker("shop", |t| t.get("SHOP-1")).unwrap().status, Status::Refining);
    let (msg_q1, msg_q2) = {
        let s = sent.lock().unwrap();
        let find = |needle: &str| {
            s.iter().position(|(_, b)| b["text"].as_str().unwrap_or_default().contains(needle)).map(|i| (1001 + i) as i64).unwrap()
        };
        let q1 = s.iter().find(|(_, b)| b["text"].as_str().unwrap_or_default().contains("Какой формат")).unwrap().1.clone();
        assert_eq!(q1["reply_markup"]["inline_keyboard"][0][0]["text"], "CSV", "options become buttons");
        (find("Какой формат"), find("Кто получает"))
    };
    let _ = msg_q1;

    // The PM taps "XLSX" and replies to the second question.
    let tap = json!({ "update_id": 2, "callback_query": { "id": "cb1", "data": format!("q:{}:1:1", qn.id), "message": { "message_id": msg_q1, "chat": { "id": 4242 } } } });
    genie::channels::handle_update(app, &tap).await.unwrap();
    let reply = json!({ "update_id": 3, "message": { "message_id": 9, "chat": { "id": 4242 }, "text": "Бухгалтерия", "reply_to_message": { "message_id": msg_q2 } } });
    genie::channels::handle_update(app, &reply).await.unwrap();

    wait_for(app, "the triage run to finish", 60, |app| {
        app.with_server(|db| Ok(db.runs("shop", None, 5)?.iter().any(|r| r.status == "succeeded"))).unwrap()
    })
    .await;
    let task = app.with_tracker("shop", |t| t.get("SHOP-1")).unwrap();
    let answers = task.comments.iter().find(|c| c.text.starts_with("Ответы на вопросы")).expect("answers recorded in the task");
    assert!(answers.text.contains("XLSX") && answers.text.contains("Бухгалтерия"));
    assert_eq!(answers.author, "pm");
    assert!(task.comments.iter().any(|c| c.text.contains("файл скачивается")), "draft criteria posted");
    assert!(task.artifacts.iter().any(|a| a.kind == genie_core::ArtifactKind::Analysis));
    let orch_mail = app.with_tracker("shop", |t| t.bus().pending(None, "orchestrator")).unwrap();
    assert!(orch_mail.iter().any(|m| m.text.contains("Triage of SHOP-1 is complete")), "the orchestrator is woken to finish refinement");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn done_task_updates_knowledge_changelog_and_tells_the_owners() {
    let l = live(None).await;
    let app = &l.app;
    let anna = app.with_server(|db| db.create_user("anna", "Анна", Some("anna@example.com"), Some("password-1"), true)).unwrap();
    install(app, "task-done-knowledge");
    app.with_tracker("shop", |t| {
        let task = t.create(
            &Actor::new("anna", Role::Human),
            CreateInput { title: "Экспорт заказов".into(), status: Some(Status::Inbox), ..Default::default() },
        )?;
        t.set_status(
            &Actor::new("anna", Role::Human),
            &task.id,
            Status::Done,
            StatusOptions { note: Some("accepted".into()), force: true, ..Default::default() },
        )
    })
    .unwrap();
    app.wake_engine.notify_one();
    wait_for(app, "the knowledge run to finish", 90, |app| {
        app.with_server(|db| Ok(db.runs("shop", None, 5)?.iter().any(|r| r.status == "succeeded"))).unwrap()
    })
    .await;

    let proposals = app.with_server(|db| db.proposals(Some("open"), 10)).unwrap();
    assert_eq!(proposals.len(), 1, "agents' pages go to review by default");
    assert_eq!(proposals[0].path, "shop/features/export.md");
    assert_eq!(proposals[0].task.as_deref(), Some("SHOP-1"), "the proposal names the task it documents");
    let changelog = std::fs::read_to_string(app.cfg.vault_path(&app.data).join("shop/changelog.md")).unwrap();
    assert!(changelog.contains("### Добавлено") && changelog.contains("- Экспорт заказов в CSV (SHOP-1)"), "{changelog}");
    let notes = app.with_server(|db| db.notifications(anna.id, false, 10)).unwrap();
    assert!(notes.iter().any(|n| n.title.contains("SHOP-1 готова") && n.body.contains("Описан экспорт")), "{notes:#?}");
    assert!(notes.iter().any(|n| n.kind == "proposal"), "section owners/admins hear about the proposal");
    // Approving the proposal publishes the page.
    genie::knowledge::decide(app, proposals[0].id, true, "anna", None, false).unwrap();
    let page = app.with_vault(|v| v.read("shop/features/export.md", None, None)).unwrap();
    assert!(page.content.contains("CSV"));
}
