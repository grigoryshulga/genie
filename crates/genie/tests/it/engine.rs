//! The automation engine's event intake: a rule applies to events from its creation on.

use std::path::PathBuf;

use genie::config::Config;
use genie::state::App;
use genie_core::{Actor, CreateInput, Role, Status, StatusOptions};
use serde_json::json;

fn create_task(app: &App, title: &str) -> String {
    app.with_tracker("shop", |t| t.create(&Actor::new("anna", Role::Human), CreateInput { title: title.into(), ..Default::default() }))
        .unwrap()
        .id
}

#[test]
fn a_rule_sees_events_after_its_creation_only() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path(), Config::load(dir.path()).unwrap(), PathBuf::from("/nonexistent")).unwrap();
    app.create_project("shop", "Shop", None, None, None).unwrap();
    // An event the engine has not read yet when the rule appears: history, not a trigger.
    create_task(&app, "before the rule");
    std::thread::sleep(std::time::Duration::from_millis(5));
    let spec = json!({ "name": "Comment new tasks", "on": { "event": "task.created" }, "steps": [{ "id": "c", "task.comment": { "text": "seen" } }] });
    let rule = app.with_server(|db| db.create_automation("shop", &spec, "anna")).unwrap();
    // Created right after the rule, before any engine pass: a trigger (this used to race when the
    // engine loaded its rules before reading the journal).
    let second = create_task(&app, "after the rule");
    genie::engine::tick(&app).unwrap();
    let runs = app.with_server(|db| db.runs("shop", Some(rule.id), 10)).unwrap();
    assert_eq!(runs.len(), 1, "{runs:?}");
    let event = app
        .with_tracker("shop", |t| t.events_after(0, 50))
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "task.created" && e.subject.as_deref() == Some(second.as_str()))
        .unwrap();
    assert_eq!(runs[0].trigger_key, format!("event:shop:{}", event.id), "the run is for the task created after the rule");
}

#[test]
fn the_auto_ready_playbook_moves_a_task_once_nothing_holds_it() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path(), Config::load(dir.path()).unwrap(), PathBuf::from("/nonexistent")).unwrap();
    app.create_project("shop", "Shop", None, None, None).unwrap();
    let (_, _, spec) = genie::engine::playbooks().into_iter().find(|(id, ..)| *id == "auto-ready").unwrap();
    assert!(genie_core::automation::validate(&spec).is_empty());
    app.with_server(|db| db.create_automation("shop", &spec, "anna")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));

    let anna = Actor::new("anna", Role::Human);
    let status = |id: &str| app.with_tracker("shop", |t| t.get(id)).unwrap().status;
    let dep = create_task(&app, "API of the warehouse");
    let ready = |title: &str, deps: Vec<String>| {
        app.with_tracker("shop", |t| {
            t.create(
                &anna,
                CreateInput {
                    title: title.into(),
                    description: Some("Export the orders".into()),
                    acceptance: vec!["A CSV file downloads".into()],
                    deps,
                    ..Default::default()
                },
            )
        })
        .unwrap()
        .id
    };
    let waiting = ready("Export orders", vec![dep.clone()]);
    let blocked = ready("Export returns", vec![]);
    app.with_tracker("shop", |t| t.block(&anna, &blocked, "no access to the warehouse API")).unwrap();
    genie::engine::tick(&app).unwrap();
    // The dependency has no description or criteria; the others wait for it or are blocked.
    assert_eq!(status(&dep), Status::Draft);
    assert_eq!(status(&waiting), Status::Draft, "an open dependency holds it");
    assert_eq!(status(&blocked), Status::Draft, "a block holds it");

    app.with_tracker("shop", |t| t.unblock(&anna, &blocked)).unwrap();
    app.with_tracker("shop", |t| t.set_status(&anna, &dep, Status::Done, StatusOptions { note: None, force: true, ..Default::default() }))
        .unwrap();
    genie::engine::tick(&app).unwrap();
    assert_eq!(status(&blocked), Status::Ready, "unblocked, nothing else holds it");
    assert_eq!(status(&waiting), Status::Ready, "its dependency is done");
}

#[test]
fn the_ready_start_playbook_wakes_the_orchestrator_to_start_work() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path(), Config::load(dir.path()).unwrap(), PathBuf::from("/nonexistent")).unwrap();
    app.create_project("shop", "Shop", None, None, None).unwrap();
    for name in ["auto-ready", "ready-start"] {
        let (_, _, spec) = genie::engine::playbooks().into_iter().find(|(id, ..)| *id == name).unwrap();
        assert!(genie_core::automation::validate(&spec).is_empty(), "{name}");
        app.with_server(|db| db.create_automation("shop", &spec, "anna")).unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(5));

    let anna = Actor::new("anna", Role::Human);
    let input = |title: &str, kind: genie_core::TaskType| CreateInput {
        title: title.into(),
        task_type: Some(kind),
        description: Some("Export the orders".into()),
        acceptance: vec!["A CSV file downloads".into()],
        ..Default::default()
    };
    let epic = app.with_tracker("shop", |t| t.create(&anna, input("Reports", genie_core::TaskType::Epic))).unwrap().id;
    let task = app.with_tracker("shop", |t| t.create(&anna, input("Export orders", genie_core::TaskType::Task))).unwrap().id;
    app.with_tracker("shop", |t| {
        t.set_status(&anna, &epic, Status::Ready, StatusOptions { note: None, force: true, ..Default::default() })
    })
    .unwrap();
    // auto-ready moves the task to ready; that (not the epic going to ready) wakes the orchestrator.
    genie::engine::tick(&app).unwrap();
    genie::engine::tick(&app).unwrap();
    assert_eq!(app.with_tracker("shop", |t| t.get(&task)).unwrap().status, Status::Ready);
    let mail = app.with_tracker("shop", |t| t.bus().pending(None, "orchestrator")).unwrap();
    let starts: Vec<_> = mail.iter().filter(|m| m.text.contains("start work")).collect();
    assert_eq!(starts.len(), 1, "{mail:?}");
    assert_eq!(starts[0].task.as_deref(), Some(task.as_str()));
    assert!(starts[0].text.contains(&task) && !starts[0].text.contains(&epic), "{}", starts[0].text);
}

/// G-89: a stopped team frees room, and the orchestrator is asked to take the next ready task.
#[test]
fn the_ready_next_playbook_wakes_the_orchestrator_when_a_team_stops() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path(), Config::load(dir.path()).unwrap(), PathBuf::from("/nonexistent")).unwrap();
    app.create_project("shop", "Shop", None, None, None).unwrap();
    let (_, _, spec) = genie::engine::playbooks().into_iter().find(|(id, ..)| *id == "ready-next").unwrap();
    assert!(genie_core::automation::validate(&spec).is_empty());
    app.with_server(|db| db.create_automation("shop", &spec, "anna")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));

    let anna = Actor::new("anna", Role::Human);
    let task = app.with_tracker("shop", |t| t.create(&anna, CreateInput { title: "Export".into(), ..Default::default() })).unwrap().id;
    app.with_tracker("shop", |t| {
        t.bus().create(
            "anna",
            "human",
            genie_core::team::NewTeam {
                id: task.clone(),
                task: task.clone(),
                cwd: dir.path().to_string_lossy().into_owned(),
                members: vec![genie_core::team::NewMember { name: "bender".into(), role: "executor".into(), ..Default::default() }],
                ..Default::default()
            },
        )
    })
    .unwrap();
    genie::engine::tick(&app).unwrap();
    let asked = |app: &App| {
        app.with_tracker("shop", |t| t.bus().pending(None, "orchestrator"))
            .unwrap()
            .iter()
            .filter(|m| m.text.contains("A team stopped"))
            .count()
    };
    assert_eq!(asked(&app), 0, "a team that starts does not free room");

    app.with_tracker("shop", |t| t.bus().set_state(&task, genie_core::TeamState::Stopped, Some("done"), "anna")).unwrap();
    genie::engine::tick(&app).unwrap();
    assert_eq!(asked(&app), 1, "a stopped team asks the orchestrator for the next ready task");
}

fn open_app() -> (tempfile::TempDir, std::sync::Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path(), Config::load(dir.path()).unwrap(), PathBuf::from("/nonexistent")).unwrap();
    app.create_project("shop", "Shop", None, None, None).unwrap();
    (dir, app)
}

/// Whether the loop behind `notify` has a wake-up waiting (the permit `notify_one` keeps).
async fn woken(notify: &tokio::sync::Notify) -> bool {
    tokio::time::timeout(std::time::Duration::from_millis(20), notify.notified()).await.is_ok()
}

#[tokio::test]
async fn journal_events_wake_the_engine_and_mail_wakes_the_scheduler() {
    let (_dir, app) = open_app();
    assert!(!woken(&app.wake_engine).await && !woken(&app.wake_runtime).await, "nothing happened yet");
    // The tracker opens on first use, and every event it appends wakes the engine, whoever wrote it.
    let bot = Actor::new("bot", Role::Orchestrator);
    let id = app.with_tracker("shop", |t| t.create(&bot, CreateInput { title: "first".into(), ..Default::default() })).unwrap().id;
    assert!(woken(&app.wake_engine).await, "a task event wakes the engine");
    assert!(!woken(&app.wake_runtime).await, "but not the scheduler");
    app.with_tracker("shop", |t| t.bus().notify_orchestrator("genie", "system", "system", "look", Some(&id))).unwrap();
    assert!(woken(&app.wake_engine).await && woken(&app.wake_runtime).await, "mail is work for both");
}

#[test]
fn the_engine_sleeps_until_the_next_cron_fire_reminder_and_wait() {
    let (_dir, app) = open_app();
    let plan = genie::engine::pass(&app).unwrap();
    assert_eq!(plan.next_at(), None, "an idle server has no deadline: only the safety net");

    // A schedule: the next fire is its deadline.
    let spec = json!({ "name": "Daily", "on": { "schedule": "0 9 * * *", "tz": "Europe/Moscow" }, "steps": [{ "id": "w", "wait": { "for": "1h" } }] });
    app.with_server(|db| db.create_automation("shop", &spec, "anna")).unwrap();
    let fire = genie_core::automation::next_fire("0 9 * * *", Some("Europe/Moscow"), chrono::Utc::now()).unwrap();
    assert_eq!(genie::engine::pass(&app).unwrap().next_at(), Some(fire));

    // A questionnaire with a reminder (sooner than the fire) and a due time (later).
    let anna = app.with_server(|db| db.create_user("anna", "Anna", None, None, false)).unwrap();
    let questions = [genie_core::inbox::NewQuestion { text: "CSV or XLSX?".into(), ..Default::default() }];
    let (qn, _) = app
        .with_server(|db| {
            db.create_questionnaire(
                "shop",
                None,
                "analyst",
                anna.id,
                "web",
                &questions,
                None,
                Some(chrono::Duration::seconds(30)),
                Some(chrono::Duration::days(3)),
            )
        })
        .unwrap();
    let reminder = chrono::DateTime::parse_from_rfc3339(qn.remind_at.as_deref().unwrap()).unwrap().to_utc();
    assert_eq!(genie::engine::pass(&app).unwrap().next_at(), Some(reminder.min(fire)));

    // A run waiting for a duration: the end of the wait is a deadline, and a repeated look keeps it.
    let waiting = json!({ "name": "Pause", "on": { "manual": true }, "steps": [{ "id": "w", "wait": { "for": "10s" } }] });
    let rule = app.with_server(|db| db.create_automation("shop", &waiting, "anna")).unwrap();
    genie::engine::start_run(&app, &rule, "manual:1", json!({ "project": "shop" }), 0, None).unwrap();
    let at = genie::engine::pass(&app).unwrap().next_at().unwrap();
    assert!(at <= chrono::Utc::now() + chrono::Duration::seconds(10), "{at}");
    assert_eq!(genie::engine::pass(&app).unwrap().next_at(), Some(at));
}
