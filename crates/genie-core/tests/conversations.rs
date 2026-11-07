use std::sync::{Arc, Barrier};

use genie_core::conversations::{Message, ProposalAction, ProposalStatus, ResponseStatus, Submission};
use genie_core::{Actor, CreateInput, GenieError, Role, Tracker, db::SCHEMA_VERSION};

fn fresh() -> (tempfile::TempDir, Tracker) {
    let dir = tempfile::tempdir().unwrap();
    let tracker = Tracker::init(dir.path().join(".genie"), None, None).unwrap();
    (dir, tracker)
}

fn input(principal: &str, text: &str) -> Submission {
    Submission {
        principal: principal.into(),
        initiator: Some(principal.into()),
        conversation_id: None,
        previous_response_id: None,
        input: vec![Message { role: "user".into(), content: text.into() }],
        idempotency_key: None,
        fingerprint: text.into(),
    }
}

fn task(t: &Tracker, title: &str) -> String {
    t.create(&Actor::new("owner", Role::Human), CreateInput { title: title.into(), ..Default::default() }).unwrap().id
}

#[test]
fn independent_principals_have_private_history_and_separate_idempotency_scopes() {
    let (_dir, t) = fresh();
    let mut a = input("alice", "private for alice");
    a.idempotency_key = Some("same-key".into());
    let alice = t.conversations().submit(a.clone()).unwrap();
    assert_eq!(t.conversations().submit(a.clone()).unwrap(), alice);

    let mut b = input("bob", "private for bob");
    b.idempotency_key = Some("same-key".into());
    let bob = t.conversations().submit(b).unwrap();
    assert_ne!(alice.conversation_id, bob.conversation_id);
    assert_eq!(t.conversations().history(&alice.conversation_id).unwrap(), vec![alice.clone()]);
    assert_eq!(t.conversations().history(&bob.conversation_id).unwrap(), vec![bob]);

    a.fingerprint = "changed request".into();
    assert!(t.conversations().submit(a).unwrap_err().to_string().contains("idempotency"));
    assert_eq!(t.conversations().queued().unwrap().len(), 2);
    let journal = serde_json::to_string(&t.events_after(0, 100).unwrap()).unwrap();
    assert!(!journal.contains("private for"), "private messages must not enter project-wide events");
    t.conversations().complete(&alice.id, "private answer").unwrap();
    let journal = serde_json::to_string(&t.events_after(0, 100).unwrap()).unwrap();
    assert!(!journal.contains("private answer"));
}

#[test]
fn continuation_requires_owner_latest_response_and_an_idle_conversation() {
    let (_dir, t) = fresh();
    let first = t.conversations().submit(input("alice", "first")).unwrap();
    let mut next = input("alice", "next");
    next.previous_response_id = Some(first.id.clone());
    assert!(t.conversations().submit(next.clone()).unwrap_err().to_string().contains("still active"));
    next.previous_response_id = None;
    next.conversation_id = Some(first.conversation_id.clone());
    assert!(t.conversations().submit(next.clone()).unwrap_err().to_string().contains("active response"));
    t.conversations().complete(&first.id, "answer").unwrap();

    let mut foreign = input("bob", "sneak in");
    foreign.previous_response_id = Some(first.id.clone());
    assert!(matches!(t.conversations().submit(foreign), Err(GenieError::Denied(_))));
    next.initiator = Some("alice-renamed".into());
    let second = t.conversations().submit(next).unwrap();
    assert_eq!(second.initiator.as_deref(), Some("alice"), "keep historical display metadata, not an identity lock");
    assert_eq!(second.conversation_id, first.conversation_id);
    assert_eq!(second.previous_response_id.as_deref(), Some(first.id.as_str()));
    t.conversations().complete(&second.id, "second answer").unwrap();
    let mut stale = input("alice", "stale follow-up");
    stale.previous_response_id = Some(first.id);
    assert!(t.conversations().submit(stale).unwrap_err().to_string().contains("latest response"));
    assert_eq!(t.conversations().history(&first.conversation_id).unwrap().len(), 2);
}

#[test]
fn restart_recovers_the_same_receipt_and_input_without_reopening_terminal_responses() {
    let (dir, t) = fresh();
    let mut submission = input("alice", "recover me");
    submission.idempotency_key = Some("recover-key".into());
    let running = t.conversations().submit(submission.clone()).unwrap();
    let completed = t.conversations().submit(input("bob", "finished")).unwrap();
    assert!(t.conversations().claim(&running.id).unwrap());
    assert!(!t.conversations().claim(&running.id).unwrap());
    t.conversations().complete(&completed.id, "done").unwrap();
    drop(t);

    let t = Tracker::open(dir.path().join(".genie")).unwrap();
    assert_eq!(t.conversations().get(&running.id).unwrap().status, ResponseStatus::InProgress);
    assert_eq!(t.conversations().recover().unwrap(), 1);
    assert_eq!(t.conversations().recover().unwrap(), 0);
    assert_eq!(t.conversations().submit(submission).unwrap(), running);
    assert_eq!(t.conversations().get(&completed.id).unwrap().output, "done");
    assert_eq!(t.conversations().queued().unwrap(), vec![running]);
}

#[test]
fn cancellation_and_completion_never_replace_a_terminal_result() {
    let (_dir, t) = fresh();
    let request = t.conversations().submit(input("alice", "work")).unwrap();
    t.conversations().claim(&request.id).unwrap();
    let cancelled = t.conversations().cancel(&request.id).unwrap();
    assert_eq!(cancelled.status, ResponseStatus::Cancelled);
    assert_eq!(t.conversations().cancel(&request.id).unwrap(), cancelled);
    assert!(!t.conversations().claim(&request.id).unwrap());
    assert!(t.conversations().complete(&request.id, "late answer").is_err());
    assert!(t.conversations().fail(&request.id, "late failure").is_err());

    let next = t.conversations().submit(input("alice", "different work")).unwrap();
    let completed = t.conversations().complete(&next.id, "answer").unwrap();
    assert_eq!(t.conversations().complete(&next.id, "answer").unwrap(), completed);
    assert_eq!(t.conversations().cancel(&next.id).unwrap(), completed);
    assert!(t.conversations().complete(&next.id, "different answer").is_err());
}

#[test]
fn task_links_and_proposal_receipts_preserve_the_originating_conversation() {
    let (_dir, t) = fresh();
    let a = t.conversations().submit(input("alice", "work")).unwrap();
    let b = t.conversations().submit(input("bob", "work")).unwrap();
    let task = task(&t, "Deliver");
    t.conversations().link_task(&a.conversation_id, &task).unwrap();
    t.conversations().link_task(&a.conversation_id, &task).unwrap();
    assert!(t.conversations().link_task(&b.conversation_id, &task).is_err());
    assert_eq!(t.conversations().conversation_for_task(&task).unwrap().as_deref(), Some(a.conversation_id.as_str()));
    assert_eq!(t.conversations().tasks(&a.conversation_id).unwrap(), vec![task.clone()]);
    assert!(t.conversations().propose(&b.conversation_id, &task, ProposalAction::Dispatch, "v1").is_err());

    let proposal = t.conversations().propose(&a.conversation_id, &task, ProposalAction::Dispatch, "v1").unwrap();
    assert_eq!(t.conversations().propose(&a.conversation_id, &task, ProposalAction::Dispatch, "v1").unwrap(), proposal);
    let accepted = t.conversations().decide(&proposal.id, true).unwrap();
    assert_eq!(accepted.status, ProposalStatus::Approved);
    assert_eq!(t.conversations().decide(&proposal.id, true).unwrap(), accepted);
    assert!(t.conversations().decide(&proposal.id, false).is_err());
    let changed = t.conversations().propose(&a.conversation_id, &task, ProposalAction::Dispatch, "v2").unwrap();
    assert_ne!(proposal.id, changed.id);
    assert_eq!(t.conversations().proposals(&a.conversation_id).unwrap().len(), 2);
}

#[test]
fn enclosing_transaction_rolls_back_receipts_links_and_events() {
    let (_dir, t) = fresh();
    let before = t.last_event_id().unwrap();
    let result: genie_core::Result<()> = t.tx(|| {
        t.conversations().submit(input("alice", "not committed"))?;
        Err(GenieError::invalid("rollback"))
    });
    assert!(result.is_err());
    assert!(t.conversations().queued().unwrap().is_empty());
    assert_eq!(t.last_event_id().unwrap(), before);

    let a = t.conversations().submit(input("alice", "work")).unwrap();
    let task = task(&t, "Deliver");
    t.conversations().link_task(&a.conversation_id, &task).unwrap();
    let proposal = t.conversations().propose(&a.conversation_id, &task, ProposalAction::Accept, "v1").unwrap();
    let before = t.last_event_id().unwrap();
    let result: genie_core::Result<()> = t.tx(|| {
        t.conversations().decide(&proposal.id, true)?;
        Err(GenieError::invalid("task gate refused"))
    });
    assert!(result.is_err());
    assert_eq!(t.conversations().proposal(&proposal.id).unwrap().status, ProposalStatus::Pending);
    assert_eq!(t.last_event_id().unwrap(), before);
}

#[test]
fn simultaneous_schedulers_claim_only_one_worker_and_conflicting_decisions_do_not_race() {
    let (dir, t) = fresh();
    let response = t.conversations().submit(input("alice", "work")).unwrap();
    let task = task(&t, "Deliver");
    t.conversations().link_task(&response.conversation_id, &task).unwrap();
    let proposal = t.conversations().propose(&response.conversation_id, &task, ProposalAction::Accept, "v1").unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|n| {
            let (barrier, path, response, proposal) =
                (barrier.clone(), dir.path().join(".genie"), response.id.clone(), proposal.id.clone());
            std::thread::spawn(move || {
                let t = Tracker::open(path).unwrap();
                barrier.wait();
                (t.conversations().claim(&response).unwrap(), t.conversations().decide(&proposal, n == 0).is_ok())
            })
        })
        .collect();
    let outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(outcomes.iter().filter(|(claimed, _)| *claimed).count(), 1);
    assert_eq!(outcomes.iter().filter(|(_, decided)| *decided).count(), 1);
}

#[test]
fn a_version_nine_tracker_gains_the_new_tables_and_keeps_existing_tasks() {
    let (dir, t) = fresh();
    let original = task(&t, "Keep me");
    t.conn()
        .execute_batch(
            "DROP TABLE conversation_proposals; DROP TABLE conversation_tasks;
        DROP TABLE conversation_responses; DROP TABLE conversations;
        UPDATE meta SET value = '9' WHERE key = 'schema';",
        )
        .unwrap();
    drop(t);
    let t = Tracker::open(dir.path().join(".genie")).unwrap();
    assert_eq!(genie_core::migrate::version(&dir.path().join(".genie/genie.db")).unwrap(), Some(SCHEMA_VERSION));
    assert_eq!(t.get(&original).unwrap().title, "Keep me");
    let response = t.conversations().submit(input("alice", "new functionality")).unwrap();
    t.conversations().link_task(&response.conversation_id, &original).unwrap();
    assert!(t.conversations().propose(&response.conversation_id, &original, ProposalAction::Dispatch, "v1").is_ok());
    let integrity: String = t.conn().query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
    assert_eq!(integrity, "ok");
    let foreign_keys: usize = t.conn().prepare("PRAGMA foreign_key_check").unwrap().query_map([], |_| Ok(())).unwrap().count();
    assert_eq!(foreign_keys, 0);
}

#[test]
fn a_tracker_from_a_newer_binary_is_rejected_without_running_this_migration() {
    let (dir, t) = fresh();
    t.conn().execute("UPDATE meta SET value = ?1 WHERE key = 'schema'", [(SCHEMA_VERSION + 1).to_string()]).unwrap();
    t.conn()
        .execute_batch(
            "DROP TABLE conversation_proposals; DROP TABLE conversation_tasks;
        DROP TABLE conversation_responses; DROP TABLE conversations;",
        )
        .unwrap();
    drop(t);
    let error = Tracker::open(dir.path().join(".genie")).err().expect("future schema must fail").to_string();
    assert!(error.contains("records schema"), "{error}");
    let conn = rusqlite::Connection::open(dir.path().join(".genie/genie.db")).unwrap();
    let tables: i64 =
        conn.query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'conversations'", [], |r| r.get(0)).unwrap();
    assert_eq!(tables, 0, "the unsupported database must not be migrated");
}

#[test]
fn shared_references_preserve_origin_and_survive_a_version_ten_upgrade() {
    let (dir, t) = fresh();
    let a = t.conversations().submit(input("alice", "private A")).unwrap();
    let b = t.conversations().submit(input("bob", "private B")).unwrap();
    let id = task(&t, "Shared work");
    t.conversations().link_task(&a.conversation_id, &id).unwrap();
    t.conn()
        .execute_batch(
            "DROP TABLE conversation_task_refs; DROP TABLE conversation_created_tasks; UPDATE meta SET value='10' WHERE key='schema';",
        )
        .unwrap();
    drop(t);
    let t = Tracker::open(dir.path().join(".genie")).unwrap();
    assert_eq!(t.conversations().created_task(&a.id, "primary").unwrap(), Some(id.clone()));
    t.conversations().reference_task(&b.conversation_id, &id).unwrap();
    t.conversations().reference_task(&b.conversation_id, &id).unwrap();
    assert_eq!(t.conversations().conversation_for_task(&id).unwrap(), Some(a.conversation_id));
    assert_eq!(t.conversations().tasks(&b.conversation_id).unwrap(), vec![id.clone()]);
    assert_eq!(t.conversations().history(&b.conversation_id).unwrap().len(), 1);
    t.conversations().propose(&b.conversation_id, &id, ProposalAction::Dispatch, "v1").unwrap();
    drop(t);
    let t = Tracker::open(dir.path().join(".genie")).unwrap();
    assert!(t.conversations().has_task(&b.conversation_id, &id).unwrap());
}

#[test]
fn client_sessions_survive_restart_and_cannot_switch_principals_or_conversations() {
    let (dir, t) = fresh();
    let first = t.conversations().submit_for_client(input("alice", "one"), Some("opencode:s1")).unwrap();
    t.conversations().complete(&first.id, "answer").unwrap();
    drop(t);
    let t = Tracker::open(dir.path().join(".genie")).unwrap();
    let next = t.conversations().submit_for_client(input("alice", "two"), Some("opencode:s1")).unwrap();
    assert_eq!(first.conversation_id, next.conversation_id);
    t.conversations().complete(&next.id, "answer").unwrap();
    let other = t.conversations().submit_for_client(input("bob", "one"), Some("opencode:s1")).unwrap();
    assert_ne!(first.conversation_id, other.conversation_id);
    let mut forged = input("alice", "switch");
    forged.conversation_id = Some(other.conversation_id);
    assert!(t.conversations().submit_for_client(forged, Some("opencode:s1")).is_err());
    assert_eq!(t.conversations().history(&first.conversation_id).unwrap().len(), 2);
}
