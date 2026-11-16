//! Port of the tracker cases in `test/tracker.test.ts`. The TypeScript tests are
//! the behavioural spec: same scenarios, same error texts.

use std::collections::HashSet;

use genie_core::*;

fn human() -> Actor {
    Actor::new("me", Role::Human)
}
fn orch() -> Actor {
    Actor::new("orchestrator", Role::Orchestrator)
}
fn executor() -> Actor {
    Actor::new("executor", Role::Executor)
}
fn reviewer() -> Actor {
    Actor::new("reviewer", Role::Reviewer)
}
fn analyst() -> Actor {
    Actor::new("analyst", Role::Analyst)
}

fn fresh() -> (tempfile::TempDir, Tracker) {
    let dir = tempfile::tempdir().unwrap();
    let t = Tracker::init(dir.path().join(".genie"), None, None).unwrap();
    (dir, t)
}

fn task(title: &str, description: &str, acceptance: &[&str]) -> CreateInput {
    CreateInput {
        title: title.into(),
        description: Some(description.into()),
        acceptance: acceptance.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

fn titled(title: &str) -> CreateInput {
    CreateInput { title: title.into(), ..Default::default() }
}

fn to(t: &Tracker, actor: &Actor, id: &str, status: Status) -> Result<Task> {
    t.set_status(actor, id, status, StatusOptions::default())
}

fn to_note(t: &Tracker, actor: &Actor, id: &str, status: Status, note: &str) -> Result<Task> {
    t.set_status(actor, id, status, StatusOptions { note: Some(note.into()), force: false, ..Default::default() })
}

fn forced(t: &Tracker, actor: &Actor, id: &str, status: Status) -> Result<Task> {
    t.set_status(actor, id, status, StatusOptions { note: None, force: true, ..Default::default() })
}

#[track_caller]
fn assert_err<T: std::fmt::Debug>(r: Result<T>, needle: &str) {
    let err = r.expect_err(&format!("expected an error containing {needle:?}")).to_string();
    assert!(err.contains(needle), "error {err:?} does not contain {needle:?}");
}

/// Undelivered mail to the orchestrator, marked delivered (what `TeamBus.receive` does).
fn orchestrator_mail(t: &Tracker) -> Vec<String> {
    let conn = t.conn();
    let mut stmt = conn.prepare("SELECT id, text FROM mail WHERE recipient = 'orchestrator' AND delivered_at IS NULL ORDER BY id").unwrap();
    let rows: Vec<(i64, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
    for (id, _) in &rows {
        conn.execute("UPDATE mail SET delivered_at = 'now' WHERE id = ?1", [id]).unwrap();
    }
    rows.into_iter().map(|(_, text)| text).collect()
}

#[test]
fn full_lifecycle_with_role_permissions_and_dor_dod_gates() {
    let (_d, t) = fresh();
    let created = t.create(&orch(), task("Export", "CSV export", &["downloads", "tests green"])).unwrap();
    assert_eq!(created.id, "G-1");
    assert_eq!(created.status, Status::Draft);

    assert_err(to(&t, &executor(), "G-1", Status::Ready), "not allowed");
    to(&t, &orch(), "G-1", Status::Ready).unwrap();
    to(&t, &executor(), "1", Status::InProgress).unwrap();
    assert_err(to(&t, &executor(), "G-1", Status::Approved), "not allowed");
    to_note(&t, &executor(), "G-1", Status::Review, "done").unwrap();
    to_note(&t, &reviewer(), "G-1", Status::ChangesRequested, "missing header row").unwrap();
    to(&t, &executor(), "G-1", Status::InProgress).unwrap();
    to(&t, &executor(), "G-1", Status::Review).unwrap();
    t.check(&reviewer(), "G-1", 1, true).unwrap();
    to(&t, &reviewer(), "G-1", Status::Approved).unwrap();

    assert_err(to(&t, &reviewer(), "G-1", Status::Done), "not allowed");
    assert_err(to(&t, &orch(), "G-1", Status::Done), "unchecked acceptance criteria: #2");
    t.check(&reviewer(), "G-1", 2, true).unwrap();
    let done = to_note(&t, &orch(), "G-1", Status::Done, "accepted").unwrap();
    assert_eq!(done.status, Status::Done);
    assert!(done.history.iter().any(|h| h.to.as_deref() == Some("changes_requested")));
    assert!(done.comments.iter().any(|c| c.kind == CommentKind::Review));
}

#[test]
fn definition_of_ready_requires_description_and_criteria() {
    let (_d, t) = fresh();
    t.create(&human(), titled("vague")).unwrap();
    assert_err(to(&t, &orch(), "G-1", Status::Ready), "description is empty; no acceptance criteria");
    assert_eq!(forced(&t, &orch(), "G-1", Status::Ready).unwrap().status, Status::Ready);
}

#[test]
fn split_makes_an_epic_and_deps_gate_the_ready_queue() {
    let (_d, t) = fresh();
    t.create(&orch(), task("Feature", "x", &["y"])).unwrap();
    let parts = t.split(&orch(), "G-1", vec![task("backend", "d", &["a"]), task("ui", "d", &["b"])]).unwrap();
    let (a, b) = (&parts[0], &parts[1]);
    t.update(&orch(), &b.id, UpdateInput { add_deps: vec![a.id.clone()], ..Default::default() }).unwrap();
    assert_eq!(t.get("G-1").unwrap().task_type, TaskType::Epic);
    assert_eq!(t.get("G-1").unwrap().children, vec![a.id.clone(), b.id.clone()]);
    to(&t, &orch(), &a.id, Status::Ready).unwrap();
    to(&t, &orch(), &b.id, Status::Ready).unwrap();
    assert_eq!(t.ready_queue().unwrap().iter().map(|x| x.id.clone()).collect::<Vec<_>>(), vec![a.id.clone()]);
    assert_err(to(&t, &executor(), &b.id, Status::InProgress), "depends on unfinished");
    assert_err(to(&t, &orch(), "G-1", Status::Done), "unfinished child");
}

#[test]
fn field_permissions_executor_cannot_rewrite_scope_analyst_can_plan() {
    let (_d, t) = fresh();
    t.create(&orch(), titled("x")).unwrap();
    assert_err(t.update(&executor(), "G-1", UpdateInput { description: Some("sneaky".into()), ..Default::default() }), "not allowed");
    t.update(&analyst(), "G-1", UpdateInput { plan: Some("1. do it".into()), add_acceptance: vec!["works".into()], ..Default::default() })
        .unwrap();
    t.update(&executor(), "G-1", UpdateInput { append_notes: Some("switched to streaming writer".into()), ..Default::default() }).unwrap();
    let got = t.get("G-1").unwrap();
    assert_eq!(got.plan, "1. do it");
    let at = got.notes.find("executor (executor)").expect("notes header");
    assert!(got.notes[at..].contains("streaming writer"));
    assert_err(t.check(&executor(), "G-1", 1, true), "not allowed");
}

#[test]
fn artifacts_live_in_the_database_not_in_files() {
    let (_d, t) = fresh();
    t.create(&orch(), titled("x")).unwrap();
    let content = |kind, name: &str, body: &str| ArtifactInput {
        kind: Some(kind),
        source: ArtifactSource::Content(body.as_bytes().to_vec()),
        name: Some(name.into()),
        note: None,
    };
    let got = t.add_artifact(&reviewer(), "G-1", content(ArtifactKind::Review, "review.md", "# LGTM")).unwrap();
    let listed: Vec<_> = got.artifacts.iter().map(|a| (a.id, a.name.clone(), a.kind)).collect();
    assert_eq!(listed, vec![(1, "review.md".to_string(), ArtifactKind::Review)]);
    assert_eq!(t.read_artifact("G-1", 1).unwrap().text.as_deref(), Some("# LGTM"));
    t.add_artifact(&reviewer(), "G-1", content(ArtifactKind::Code, "zcl_x.clas.abap", "CLASS zcl_x DEFINITION.")).unwrap();
    assert_eq!(t.read_artifact("G-1", 2).unwrap().kind, ArtifactKind::Code);
    let stray: Vec<_> = std::fs::read_dir(t.dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|f| !f.starts_with("genie.db"))
        .collect();
    assert!(stray.is_empty(), "unexpected files: {stray:?}");
    assert_err(t.read_artifact("G-1", 3), "G-1 has no artifact #3");
}

#[test]
fn owner_inbox_and_comments_wake_the_orchestrator() {
    let (_d, t) = fresh();
    let created = t.create(&human(), CreateInput { status: Some(Status::Inbox), ..titled("From the web") }).unwrap();
    assert_eq!(created.status, Status::Inbox);
    t.comment(&human(), &created.id, "please hurry", CommentKind::Note).unwrap();
    let mails = orchestrator_mail(&t);
    assert_eq!(mails.len(), 2);
    assert!(mails[0].contains("New task G-1 in the inbox"));
    assert_eq!(t.get(&created.id).unwrap().comments[0].kind, CommentKind::Owner);
    assert!(orchestrator_mail(&t).is_empty());
    t.comment(&orch(), &created.id, "on it", CommentKind::Note).unwrap();
    assert!(orchestrator_mail(&t).is_empty(), "agent activity does not wake the orchestrator");
}

#[test]
fn needs_owner_keeps_the_question_and_the_status_to_return_to() {
    let (_d, t) = fresh();
    t.create(&orch(), task("x", "d", &["a"])).unwrap();
    to(&t, &orch(), "G-1", Status::Ready).unwrap();
    assert_err(to(&t, &orch(), "G-1", Status::NeedsOwner), "requires a note");
    assert_err(to_note(&t, &executor(), "G-1", Status::NeedsOwner, "?"), "not allowed");
    let before = t.last_event_id().unwrap();
    let waiting = to_note(&t, &orch(), "G-1", Status::NeedsOwner, "Keep ZPR1?").unwrap();
    let no = waiting.needs_owner.unwrap();
    assert_eq!((no.question.as_str(), no.previous), ("Keep ZPR1?", Status::Ready));
    let changes: Vec<_> = t
        .events_after(before, 100)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind == events::TASK_STATUS_CHANGED)
        .map(|e| format!("{}->{}", e.payload["from"].as_str().unwrap(), e.payload["to"].as_str().unwrap()))
        .collect();
    assert_eq!(changes, vec!["ready->needs_owner"]);
    let listed = t.list(&ListFilter { status: vec![Status::NeedsOwner], ..Default::default() }).unwrap();
    assert_eq!(listed[0].needs_owner.as_ref().unwrap().question, "Keep ZPR1?");
    let back = to(&t, &orch(), "G-1", Status::Ready).unwrap();
    assert!(back.needs_owner.is_none());
}

#[test]
fn optional_artifact_gates() {
    let (_d, mut t) = fresh();
    t.create(&orch(), task("x", "d", &["a"])).unwrap();
    to(&t, &orch(), "G-1", Status::Ready).unwrap();
    to(&t, &executor(), "G-1", Status::InProgress).unwrap();
    to(&t, &executor(), "G-1", Status::Review).unwrap();
    to(&t, &reviewer(), "G-1", Status::ChangesRequested).unwrap();
    to(&t, &executor(), "G-1", Status::InProgress).unwrap();
    t.gates = Gates { require_test_report: true, require_review_artifact: true };
    assert_err(to(&t, &executor(), "G-1", Status::Review), "test-report");
    let report =
        ArtifactInput { kind: Some(ArtifactKind::TestReport), source: ArtifactSource::Content(b"ok".to_vec()), name: None, note: None };
    t.add_artifact(&executor(), "G-1", report).unwrap();
    to(&t, &executor(), "G-1", Status::Review).unwrap();
    assert_err(to(&t, &reviewer(), "G-1", Status::Approved), "review artifact");
}

#[test]
fn concurrent_writers_do_not_lose_updates() {
    let (_d, t) = fresh();
    t.create(&orch(), titled("x")).unwrap();
    let dir = t.dir().to_path_buf();
    let writers: Vec<_> = (1..=4)
        .map(|n| {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let w = Tracker::open(&dir).unwrap();
                let actor = Actor::new(format!("w{n}"), Role::Executor);
                for i in 0..20 {
                    w.comment(&actor, "G-1", &format!("c{i}"), CommentKind::Note).unwrap();
                }
            })
        })
        .collect();
    for w in writers {
        w.join().unwrap();
    }
    assert_eq!(t.get("G-1").unwrap().comments.len(), 80);
    let commented = t.events_after(0, 1000).unwrap().into_iter().filter(|e| e.kind == events::TASK_COMMENTED).count();
    assert_eq!(commented, 80, "every committed comment has exactly one event");
}

#[test]
fn orchestrator_needs_force_to_fake_the_teams_verdict() {
    let (_d, t) = fresh();
    t.create(&orch(), task("x", "d", &["a"])).unwrap();
    to(&t, &orch(), "G-1", Status::Ready).unwrap();
    to(&t, &executor(), "G-1", Status::InProgress).unwrap();
    assert_err(to(&t, &orch(), "G-1", Status::Review), "team's verdicts");
    assert_eq!(forced(&t, &orch(), "G-1", Status::Review).unwrap().status, Status::Review);
    assert_err(forced(&t, &executor(), "G-1", Status::Approved), "not allowed");
}

#[test]
fn epics_tasks_move_in_and_out_no_nesting_list_filters_and_progress() {
    let (_d, t) = fresh();
    let epic = t.create(&orch(), CreateInput { task_type: Some(TaskType::Epic), ..task("Returns", "goal", &["works"]) }).unwrap();
    let a = t.create(&orch(), CreateInput { parent: Some(epic.id.clone()), ..titled("a") }).unwrap();
    let b = t.create(&orch(), titled("b")).unwrap();
    assert_err(
        t.create(&orch(), CreateInput { task_type: Some(TaskType::Epic), parent: Some(epic.id.clone()), ..titled("nested") }),
        "cannot be nested",
    );
    let move_to = |id: &str| UpdateInput { parent: Some(Some(id.to_string())), ..Default::default() };
    assert_err(t.update(&orch(), &b.id, move_to(&a.id)), "not an epic");
    assert_err(t.update(&orch(), &epic.id, move_to(&epic.id)), "own epic");
    assert_err(t.update(&executor(), &b.id, move_to(&epic.id)), "not allowed to change epic");
    t.update(&analyst(), &b.id, move_to(&epic.id)).unwrap();
    assert_eq!(t.get(&epic.id).unwrap().children, vec![a.id.clone(), b.id.clone()]);
    assert!(t.get(&epic.id).unwrap().history.last().unwrap().event.contains(&format!("child {} moved in", b.id)));
    t.update(&orch(), &b.id, UpdateInput { parent: Some(None), ..Default::default() }).unwrap();
    assert!(t.get(&b.id).unwrap().parent.is_none());
    assert_err(t.update(&orch(), &a.id, UpdateInput { task_type: Some(TaskType::Epic), ..Default::default() }), "cannot be nested");

    let ids = |f: ListFilter| t.list(&f).unwrap().into_iter().map(|x| x.id).collect::<Vec<_>>();
    assert_eq!(ids(ListFilter { task_type: vec![TaskType::Epic], ..Default::default() }), vec![epic.id.clone()]);
    assert!(!t.list(&ListFilter { exclude_epics: true, ..Default::default() }).unwrap().iter().any(|x| x.task_type == TaskType::Epic));
    assert_eq!(ids(ListFilter { parent: Some(epic.id.clone()), ..Default::default() }), vec![a.id.clone()]);
    let summary = &t.list(&ListFilter { task_type: vec![TaskType::Epic], ..Default::default() }).unwrap()[0];
    assert_eq!((summary.children, summary.children_closed), (1, 0));
}

#[test]
fn epics_start_with_their_first_task_and_ask_the_orchestrator_to_close_them() {
    let (_d, t) = fresh();
    let epic = t.create(&orch(), CreateInput { task_type: Some(TaskType::Epic), ..task("Returns", "goal", &["works"]) }).unwrap();
    let ready = |title: &str| {
        let x = t.create(&orch(), CreateInput { parent: Some(epic.id.clone()), ..task(title, "d", &["c"]) }).unwrap();
        to(&t, &orch(), &x.id, Status::Ready).unwrap();
        x
    };
    let a = ready("a");
    let b = ready("b");
    assert_eq!(t.get(&epic.id).unwrap().status, Status::Draft);
    to(&t, &executor(), &a.id, Status::InProgress).unwrap();
    assert_eq!(t.get(&epic.id).unwrap().status, Status::InProgress, "work on a task starts its epic");
    assert_eq!(t.get(&epic.id).unwrap().history.last().unwrap().actor, "genie");

    to(&t, &executor(), &a.id, Status::Review).unwrap();
    t.check(&reviewer(), &a.id, 1, true).unwrap();
    to(&t, &reviewer(), &a.id, Status::Approved).unwrap();
    to(&t, &orch(), &a.id, Status::Done).unwrap();
    assert!(!orchestrator_mail(&t).iter().any(|m| m.contains("All tasks of epic")), "one task is still open");
    to(&t, &orch(), &b.id, Status::Cancelled).unwrap();
    assert!(orchestrator_mail(&t).iter().any(|m| m.contains(&format!("All tasks of epic {} are closed", epic.id))));
    assert_eq!(t.list(&ListFilter { task_type: vec![TaskType::Epic], ..Default::default() }).unwrap()[0].children_closed, 2);

    assert_err(to(&t, &orch(), &epic.id, Status::Done), "unchecked acceptance");
    t.check(&orch(), &epic.id, 1, true).unwrap();
    to(&t, &orch(), &epic.id, Status::Done).unwrap();
    assert_eq!(t.get(&epic.id).unwrap().status, Status::Done);
}

#[test]
fn epic_context_and_split_inside_an_epic_keeps_pieces_in_the_epic() {
    let (_d, t) = fresh();
    let epic =
        t.create(&orch(), CreateInput { task_type: Some(TaskType::Epic), ..task("Returns", "Correct return prices.", &[]) }).unwrap();
    let child = t.create(&orch(), CreateInput { parent: Some(epic.id.clone()), ..titled("Reason codes") }).unwrap();
    let sub = t.create(&orch(), CreateInput { parent: Some(child.id.clone()), ..titled("sub-step") }).unwrap();
    assert_eq!(t.epic_context(&sub.id).unwrap().epic.unwrap().id, epic.id, "found through the parent task");
    assert_eq!(t.epic_context(&epic.id).unwrap().children.unwrap().len(), 1);

    let pieces = t.split(&orch(), &child.id, vec![titled("one"), titled("two")]).unwrap();
    assert!(pieces.iter().all(|p| t.get(&p.id).unwrap().parent.as_deref() == Some(epic.id.as_str())), "pieces join the epic");
    let split = t.get(&child.id).unwrap();
    assert_eq!(split.status, Status::Cancelled);
    assert_eq!(split.task_type, TaskType::Task, "a task inside an epic does not become a nested epic");
}

#[test]
fn readiness_problems_report_missing_dependencies() {
    let (_d, t) = fresh();
    t.create(&orch(), task("x", "d", &["a"])).unwrap();
    let mut got = t.get("G-1").unwrap();
    got.deps = vec!["G-99".into()];
    assert_eq!(readiness_problems(&got, &HashSet::new()), vec!["dependency G-99 does not exist"]);
}

#[test]
fn configured_roles_adjust_their_class_permissions() {
    let (_d, t) = fresh();
    t.create(&orch(), task("Export", "CSV export", &["downloads"])).unwrap();
    to(&t, &orch(), "G-1", Status::Ready).unwrap();
    // A researcher: an analyst that may submit its findings for review.
    let researcher = Actor::with_caps(
        "poirot",
        Role::Analyst,
        adjust_capabilities(&class_capabilities(Role::Analyst), &[Capability::StatusSubmit], &[]),
    );
    to(&t, &researcher, "G-1", Status::InProgress).unwrap();
    to(&t, &researcher, "G-1", Status::Review).unwrap();
    // A security reviewer that must not tick functional criteria.
    let security =
        Actor::with_caps("argus", Role::Reviewer, adjust_capabilities(&class_capabilities(Role::Reviewer), &[], &[Capability::TaskCheck]));
    assert_err(t.check(&security, "G-1", 1, true), "not allowed to check acceptance criteria");
    let quiet =
        Actor::with_caps("mute", Role::Executor, adjust_capabilities(&class_capabilities(Role::Executor), &[], &[Capability::TaskBlock]));
    assert_err(t.block(&quiet, "G-1", "stuck"), "not allowed to block tasks");
    to(&t, &security, "G-1", Status::Approved).unwrap();
}

#[test]
fn whoever_submitted_the_work_cannot_approve_it() {
    let (_d, t) = fresh();
    t.create(&orch(), task("Export", "CSV export", &["downloads"])).unwrap();
    to(&t, &orch(), "G-1", Status::Ready).unwrap();
    let both = Actor::with_caps(
        "solo",
        Role::Executor,
        adjust_capabilities(&class_capabilities(Role::Executor), &[Capability::StatusApprove], &[]),
    );
    to(&t, &both, "G-1", Status::InProgress).unwrap();
    to(&t, &both, "G-1", Status::Review).unwrap();
    assert_err(to(&t, &both, "G-1", Status::Approved), "submitted this work for review themselves");
    to(&t, &reviewer(), "G-1", Status::Approved).unwrap();
}

#[test]
fn a_question_for_the_owner_carries_an_action_with_checked_options() {
    let (_d, t) = fresh();
    let id = t.create(&orch(), task("Returns", "price of returns", &["agreed"])).unwrap().id;
    let ask = |action: OwnerAction| {
        t.set_status(
            &orch(),
            &id,
            Status::NeedsOwner,
            StatusOptions { note: Some("Цена?".into()), force: false, action: Some(action), context: None },
        )
    };
    assert_err(ask(OwnerAction::AskOwnerQuestion { options: vec!["Сохранять".into(), " Сохранять ".into()] }), "2 to 6");
    assert_err(ask(OwnerAction::AskForMergePr { repo: " ".into(), number: None, url: None }), "repository");

    let got =
        ask(OwnerAction::AskOwnerQuestion { options: vec![" Сохранять".into(), "Пересчитывать".into(), "".into()] }).unwrap();
    let owner = got.needs_owner.unwrap();
    assert_eq!(owner.action, Some(OwnerAction::AskOwnerQuestion { options: vec!["Сохранять".into(), "Пересчитывать".into()] }));
    assert!(got.comments.iter().any(|c| c.text.contains("options: Сохранять / Пересчитывать")), "{:?}", got.comments);

    // An action goes with needs_owner only.
    let back = t.set_status(&human(), &id, owner.previous, StatusOptions { action: Some(OwnerAction::AskFreeForm), ..Default::default() });
    assert_err(back, "needs_owner only");
    // What agents send: a kind the server knows, else the list of kinds.
    assert_err(OwnerAction::parse(&serde_json::json!({ "kind": "ask-doc-change" })), "ask-owner-question, ask-for-merge-pr, ask-free-form");
    assert_eq!(OwnerAction::parse(&serde_json::json!({ "kind": "ask-free-form" })).unwrap(), OwnerAction::AskFreeForm);
}

#[test]
fn owner_context_is_optional_bounded_and_round_trips() {
    let (_d, t) = fresh();
    let id = t.create(&orch(), task("Decision", "Choose storage", &["agreed"])).unwrap().id;
    let ask = |question: String, context: Option<String>| {
        t.set_status(&orch(), &id, Status::NeedsOwner, StatusOptions { note: Some(question), context, ..Default::default() })
    };
    assert_err(ask("я".repeat(401), None), "--context");
    assert_err(ask("Storage?".into(), Some("я".repeat(8001))), "8000");
    let got = ask("**Storage?**".into(), Some("## Context\n\n- Keep history".into())).unwrap();
    let owner = got.needs_owner.unwrap();
    assert_eq!(owner.context.as_deref(), Some("## Context\n\n- Keep history"));
    assert_eq!(t.get(&id).unwrap().needs_owner.unwrap(), owner);
    assert!(t.get(&id).unwrap().comments.iter().any(|c| c.text.contains("## Context\n\n- Keep history")));
    assert_err(
        t.set_status(&human(), &id, owner.previous, StatusOptions { context: Some("Not a decision".into()), ..Default::default() }),
        "needs_owner only",
    );
    let legacy: NeedsOwner = serde_json::from_value(serde_json::json!({
        "question": "Old question", "by": "orchestrator", "at": "2026-01-01", "previous": "draft"
    }))
    .unwrap();
    assert_eq!(legacy.context, None);
}
