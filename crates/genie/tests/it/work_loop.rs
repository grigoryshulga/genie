//! The built-in work loop: preparation prompts, dispatch of ready tasks by priority, reconciliation and
//! the retirement of the playbooks it replaced. The project manager, not the loop, moves a task to ready.

use crate::common::Harness;
use genie::work_loop;
use genie_core::management::manager_name;
use genie_core::team::{NewMember, NewTeam, ORCHESTRATOR};
use genie_core::{Actor, CreateInput, Role, Status, StatusOptions, TaskType, TeamState};
use serde_json::{Value, json};

fn harness() -> Harness {
    let h = Harness::new();
    h.project("shop");
    h
}

fn anna() -> Actor {
    Actor::new("anna", Role::Human)
}

fn tick(h: &Harness) {
    genie::engine::tick(&h.app).unwrap();
}

fn create(h: &Harness, input: CreateInput) -> String {
    h.app.with_tracker("shop", |t| t.create(&anna(), input)).unwrap().id
}

/// A task with a description and criteria: nothing but its status keeps it from being ready.
fn complete(title: &str, priority: i64) -> CreateInput {
    CreateInput {
        title: title.into(),
        description: Some("Export the orders".into()),
        acceptance: vec!["A CSV file downloads".into()],
        priority: Some(priority),
        ..Default::default()
    }
}

fn set(h: &Harness, id: &str, status: Status) {
    h.app.with_tracker("shop", |t| t.set_status(&anna(), id, status, StatusOptions { force: true, ..Default::default() })).unwrap();
}

fn status(h: &Harness, id: &str) -> Status {
    h.app.with_tracker("shop", |t| t.get(id)).unwrap().status
}

/// The loop's prompts about a task, read or not.
fn sent(h: &Harness, id: &str) -> i64 {
    h.app.with_tracker("shop", |t| t.bus().sent_about("work-loop", id)).unwrap().sent
}

/// The project manager's turn reads and finishes everything addressed to it.
fn read_mail(h: &Harness) {
    h.app
        .with_tracker("shop", |t| {
            t.bus().lease(None, ORCHESTRATOR, 1)?;
            t.bus().complete_lease(1)
        })
        .unwrap();
}

/// The task manager of a task, when one was dispatched and is not completed.
fn manager_active(h: &Harness, id: &str) -> bool {
    h.app.with_tracker("shop", |t| t.management().manager(id)).unwrap().is_some_and(|m| m.state == "active")
}

#[test]
fn a_task_entering_preparation_prompts_the_project_manager_once() {
    let h = harness();
    let id = create(&h, CreateInput { title: "Export orders".into(), ..Default::default() });
    set(&h, &id, Status::Refining);
    tick(&h);
    assert_eq!(sent(&h, &id), 1);
    let prompt =
        h.app.with_tracker("shop", |t| t.bus().pending(None, ORCHESTRATOR)).unwrap().into_iter().find(|m| m.from == "work-loop").unwrap();
    assert_eq!(prompt.from, "work-loop");
    assert_eq!(prompt.task.as_deref(), Some(id.as_str()));
    assert!(prompt.text.contains("is in preparation") && prompt.text.contains("`ready`"), "{}", prompt.text);

    // Moved again while the first prompt is still unread: no second prompt.
    set(&h, &id, Status::Draft);
    set(&h, &id, Status::Refining);
    tick(&h);
    assert_eq!(sent(&h, &id), 1, "an unread prompt is not repeated");

    // Once the manager has read it, entering preparation again prompts again.
    read_mail(&h);
    set(&h, &id, Status::Draft);
    set(&h, &id, Status::Refining);
    tick(&h);
    assert_eq!(sent(&h, &id), 2);
}

#[test]
fn no_prompt_for_an_inbox_task_or_an_epic_and_a_manual_project_is_left_to_people() {
    let h = harness();
    let inbox = create(&h, CreateInput { title: "From the owner".into(), status: Some(Status::Inbox), ..Default::default() });
    let epic = create(&h, CreateInput { title: "Reports".into(), task_type: Some(TaskType::Epic), ..Default::default() });
    set(&h, &epic, Status::Refining);
    tick(&h);
    assert_eq!((sent(&h, &inbox), sent(&h, &epic)), (0, 0), "the inbox never moves by itself, epics are not prepared");

    h.app.with_server(|db| db.set_autonomy("shop", "manual")).unwrap();
    let manual = create(&h, CreateInput { title: "Manual".into(), ..Default::default() });
    set(&h, &manual, Status::Refining);
    let ready = create(&h, complete("Ready in manual", 1));
    set(&h, &ready, Status::Ready);
    tick(&h);
    assert_eq!(sent(&h, &manual), 0, "a manual project gets no prompts");
    assert_eq!(status(&h, &ready), Status::Ready);
    assert!(!manager_active(&h, &ready), "and no dispatch");

    // The cursor moved on in the manual project: enabling autonomy does not replay what passed.
    h.app.with_server(|db| db.set_autonomy("shop", "autonomous")).unwrap();
    tick(&h);
    assert_eq!(sent(&h, &manual), 0, "old history is not replayed when autonomy comes back");
}

#[test]
fn the_loop_never_moves_a_prepared_task_to_ready() {
    let h = harness();
    // Meets the Definition of Ready: only the project manager's move to `ready` makes it ready.
    let id = create(&h, complete("Export orders", 1));
    set(&h, &id, Status::Refining);
    tick(&h);
    work_loop::reconcile(&h.app, "shop", chrono::Utc::now() + chrono::Duration::hours(2)).unwrap();
    assert_eq!(status(&h, &id), Status::Refining, "the loop leaves the move to the manager");
    assert_eq!(sent(&h, &id), 1, "the manager is prompted to prepare it, and that is all");
}

#[test]
fn a_ready_task_waits_quietly_for_its_open_dependency() {
    let h = harness();
    let dep = create(&h, complete("Warehouse API", 1));
    let waiting = create(&h, CreateInput { deps: vec![dep.clone()], ..complete("Export orders", 0) });
    // The project manager moved it to ready while the dependency is still open.
    set(&h, &waiting, Status::Ready);
    tick(&h);
    assert_eq!(status(&h, &waiting), Status::Ready, "the loop does not change its status");
    assert!(!manager_active(&h, &waiting), "an open dependency holds the dispatch");
    assert!(
        work_loop::dispatch_ready(&h.app, "shop").unwrap().is_empty(),
        "a held task is skipped without a dispatch error: nothing is dispatched, and nothing is logged"
    );

    set(&h, &dep, Status::Done);
    tick(&h);
    assert!(manager_active(&h, &waiting), "the dependency closing starts the waiting task");
}

#[test]
fn a_blocked_ready_task_is_dispatched_once_unblocked() {
    let h = harness();
    let blocked = create(&h, complete("Export returns", 0));
    set(&h, &blocked, Status::Ready);
    h.app.with_tracker("shop", |t| t.block(&anna(), &blocked, "no access to the warehouse API")).unwrap();
    tick(&h);
    assert!(!manager_active(&h, &blocked), "a block holds it");
    assert_eq!(status(&h, &blocked), Status::Ready);

    h.app.with_tracker("shop", |t| t.unblock(&anna(), &blocked)).unwrap();
    tick(&h);
    assert!(manager_active(&h, &blocked), "the unblock event runs a dispatch pass");

    // Reconciliation alone also starts a task that was unblocked, whatever the journal says.
    let again = create(&h, complete("Export returns again", 0));
    set(&h, &again, Status::Ready);
    h.app.with_tracker("shop", |t| t.block(&anna(), &again, "waiting for the warehouse")).unwrap();
    tick(&h);
    h.app.with_tracker("shop", |t| t.unblock(&anna(), &again)).unwrap();
    work_loop::reconcile(&h.app, "shop", chrono::Utc::now()).unwrap();
    assert!(manager_active(&h, &again), "the reconciliation pass starts it");
}

#[test]
fn an_owner_decision_that_readies_a_task_starts_it() {
    let h = harness();
    let id = create(&h, complete("Export in CSV or XLSX", 0));
    h.app
        .with_tracker("shop", |t| {
            t.set_status(&anna(), &id, Status::NeedsOwner, StatusOptions { note: Some("CSV or XLSX?".into()), ..Default::default() })
        })
        .unwrap();
    tick(&h);
    assert!(!manager_active(&h, &id), "a task waiting for the owner is not dispatched");

    set(&h, &id, Status::Ready);
    tick(&h);
    assert!(manager_active(&h, &id), "the decision that moves it to ready starts it");
}

#[test]
fn ready_tasks_are_dispatched_by_priority_within_the_team_limit() {
    let h = Harness::with_config(|c| c.limits.max_active_teams = 2);
    h.project("shop");
    let low = create(&h, complete("Low", 3));
    let urgent = create(&h, complete("Urgent", 0));
    let normal = create(&h, complete("Normal", 2));
    for id in [&low, &urgent, &normal] {
        set(&h, id, Status::Ready);
    }
    tick(&h);
    assert!(manager_active(&h, &urgent) && manager_active(&h, &normal), "the two most urgent go first");
    assert!(!manager_active(&h, &low), "the limit of two active managers holds the third back");
    let mail = h.app.with_tracker("shop", |t| t.bus().pending(None, &manager_name(&urgent))).unwrap();
    assert!(mail.iter().any(|m| m.text.contains("own this task")), "the manager gets its brief");
}

#[test]
fn the_next_ready_task_is_dispatched_when_a_manager_completes() {
    let h = Harness::with_config(|c| c.limits.max_active_teams = 1);
    h.project("shop");
    let first = create(&h, complete("First", 0));
    let second = create(&h, complete("Second", 1));
    set(&h, &first, Status::Ready);
    set(&h, &second, Status::Ready);
    tick(&h);
    assert!(manager_active(&h, &first) && !manager_active(&h, &second));

    // Closing the first task retires its manager: that frees the room, before the report is published.
    set(&h, &first, Status::Done);
    tick(&h);
    assert!(manager_active(&h, &second), "a retrospective manager does not hold the room");

    h.app.with_tracker("shop", |t| t.management().publish_retrospective(&anna(), &first, "Shipped the export.", true)).unwrap();
    tick(&h);
    let completed = h.app.with_tracker("shop", |t| t.management().manager(&first)).unwrap().unwrap();
    assert_eq!(completed.state, "completed");
    assert!(manager_active(&h, &second), "publication changes nothing for the next task");
}

#[test]
fn a_task_that_cannot_be_dispatched_is_skipped_and_the_rest_go_on() {
    let h = harness();
    let dep = create(&h, CreateInput { title: "Warehouse API".into(), ..Default::default() });
    // Forced ready with a dependency still open: held on purpose, left out, the next task starts.
    let blocked_by_dep = create(&h, CreateInput { deps: vec![dep.clone()], ..complete("Waits for the API", 0) });
    let ready = create(&h, complete("Starts now", 1));
    set(&h, &blocked_by_dep, Status::Ready);
    set(&h, &ready, Status::Ready);
    tick(&h);
    assert!(!manager_active(&h, &blocked_by_dep), "its dependency is not done");
    assert!(manager_active(&h, &ready));
}

#[test]
fn a_stopped_team_runs_a_dispatch_pass_but_frees_no_manager_slot() {
    let h = Harness::with_config(|c| c.limits.max_active_teams = 1);
    h.project("shop");
    let busy = create(&h, complete("Running", 0));
    let waiting = create(&h, complete("Waiting for room", 1));
    set(&h, &busy, Status::Ready);
    set(&h, &waiting, Status::Ready);
    tick(&h);
    assert!(manager_active(&h, &busy) && !manager_active(&h, &waiting), "the one slot goes to the first task");

    // A team of the running task stops. Capacity counts task managers, not teams: the slot stays taken.
    h.app
        .with_tracker("shop", |t| {
            t.bus().create(
                "anna",
                "human",
                NewTeam {
                    id: busy.clone(),
                    task: busy.clone(),
                    cwd: h.dir.path().to_string_lossy().into_owned(),
                    members: vec![NewMember { name: "bender".into(), role: "executor".into(), ..Default::default() }],
                    ..Default::default()
                },
            )?;
            t.bus().set_state(&busy, TeamState::Stopped, Some("done"), "anna")
        })
        .unwrap();
    tick(&h);
    assert!(!manager_active(&h, &waiting), "a stopped team does not free the slot of its task manager");
}

#[test]
fn reconciliation_prompts_a_task_that_was_never_prompted_after_two_minutes() {
    let h = harness();
    let id = create(&h, CreateInput { title: "Created in draft".into(), ..Default::default() });
    tick(&h);
    assert_eq!(sent(&h, &id), 0, "a fresh task is left alone");

    let now = chrono::Utc::now();
    work_loop::reconcile(&h.app, "shop", now + chrono::Duration::seconds(60)).unwrap();
    assert_eq!(sent(&h, &id), 0, "a minute of quiet is not yet enough");
    work_loop::reconcile(&h.app, "shop", now + chrono::Duration::minutes(3)).unwrap();
    assert_eq!(sent(&h, &id), 1, "quiet for more than two minutes: the manager is prompted");
}

#[test]
fn reconciliation_prompts_lost_preparation_again_and_stops_after_three_prompts() {
    let h = harness();
    let boss = h.app.with_server(|db| db.create_user("boss", "Boss", None, None, true)).unwrap();
    let id = create(&h, CreateInput { title: "Lost in refining".into(), ..Default::default() });
    set(&h, &id, Status::Refining);
    tick(&h);
    assert_eq!(sent(&h, &id), 1);

    // The manager reads its prompts, then nothing happens for longer than the retry interval.
    let t0 = chrono::Utc::now();
    read_mail(&h);
    work_loop::reconcile(&h.app, "shop", t0 + chrono::Duration::hours(2)).unwrap();
    assert_eq!(sent(&h, &id), 2, "lost: prompted again");
    read_mail(&h);
    work_loop::reconcile(&h.app, "shop", t0 + chrono::Duration::hours(4)).unwrap();
    assert_eq!(sent(&h, &id), 3);
    read_mail(&h);

    // The fourth look gives up: no prompt, and the admins hear about it once.
    work_loop::reconcile(&h.app, "shop", t0 + chrono::Duration::hours(6)).unwrap();
    work_loop::reconcile(&h.app, "shop", t0 + chrono::Duration::hours(8)).unwrap();
    assert_eq!(sent(&h, &id), 3, "no fourth prompt");
    let told: Vec<_> = h
        .app
        .with_server(|db| db.notifications(boss.id, false, 50))
        .unwrap()
        .into_iter()
        .filter(|n| n.title == format!("Подготовка {id} застряла"))
        .collect();
    assert_eq!(told.len(), 1, "one notification, however often the loop looks");
}

#[test]
fn a_prompt_is_not_repeated_while_the_task_is_held_on_purpose() {
    let h = harness();
    let id = create(&h, CreateInput { title: "Waits for the owner".into(), ..Default::default() });
    set(&h, &id, Status::Refining);
    tick(&h);
    read_mail(&h);
    h.app.with_tracker("shop", |t| t.block(&anna(), &id, "the warehouse access is not decided")).unwrap();
    work_loop::reconcile(&h.app, "shop", chrono::Utc::now() + chrono::Duration::hours(2)).unwrap();
    assert_eq!(sent(&h, &id), 1, "a blocked task waits for its block, not for the manager");
}

#[test]
fn retired_playbooks_are_disabled_once_and_own_rules_are_kept() {
    let h = harness();
    // The installed copies: the four playbooks as 4c612e0 created them, with limits the owner may have changed.
    for p in genie::engine::RETIRED_PLAYBOOKS {
        let mut spec = json!({ "name": p.name, "on": serde_json::from_str::<Value>(p.on).unwrap(), "steps": serde_json::from_str::<Value>(p.steps).unwrap() });
        spec["limits"] = json!({ "concurrency": 9 });
        h.app.with_server(|db| db.create_automation("shop", &spec, "anna")).unwrap();
    }
    // Owner rules: one that shares a retired name but has other steps, one with the retired steps under another name.
    let same_name = json!({ "name": genie::engine::RETIRED_PLAYBOOKS[1].name, "on": { "event": "task.*" }, "steps": [{ "id": "w", "wait": { "for": "1h" } }] });
    let same_steps = json!({ "name": "Ping on tasks", "on": { "event": "task.*" }, "steps": [{ "id": "ready", "task.ready": {} }] });
    let kept: Vec<i64> = [&same_name, &same_steps]
        .into_iter()
        .map(|spec| h.app.with_server(|db| db.create_automation("shop", spec, "anna")).unwrap().id)
        .collect();

    assert_eq!(genie::engine::disable_retired_playbooks(&h.app).unwrap(), 4, "every installed copy is switched off");
    assert_eq!(genie::engine::disable_retired_playbooks(&h.app).unwrap(), 0, "a second start changes nothing");
    let rules = h.app.with_server(|db| db.automations(Some("shop"))).unwrap();
    assert_eq!(rules.len(), 6);
    for rule in rules {
        let owned = kept.contains(&rule.id);
        assert_eq!(rule.enabled, owned, "{} ({})", rule.name, rule.id);
    }
}

#[test]
fn an_idle_project_wakes_the_engine_for_its_reconciliation() {
    let h = harness();
    let plan = genie::engine::pass(&h.app).unwrap();
    let next = plan.next_at().expect("a non-manual project reconciles within a minute");
    assert!(next <= chrono::Utc::now() + chrono::Duration::seconds(61), "{next}");
    let again = genie::engine::pass(&h.app).unwrap();
    assert_eq!(again.next_at(), Some(next), "throttled: the same reconciliation deadline until it is due");
}
