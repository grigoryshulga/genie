use genie_core::model_calls::{Admission, CallState, Money, NewCall, Scope, Worker};
use genie_core::usage::{ModelPrice, Tokens};
use genie_core::{Actor, CreateInput, Role, Tracker};
use serde_json::json;

fn price() -> ModelPrice {
    ModelPrice { input: 1.0, output: 2.0, cache_read: Some(0.5), cache_write: Some(0.0) }
}
fn task(t: &Tracker) -> String {
    t.create(&Actor::new("owner", Role::Human), CreateInput { title: "Bounded work".into(), ..Default::default() }).unwrap().id
}
fn worker(t: &Tracker, task: &str, id: &str) -> Worker {
    Worker {
        id: id.into(),
        token_hash: format!("hash-{id}"),
        agent: "worker".into(),
        task: Some(task.into()),
        model: "fake/model".into(),
        input_limit: 100,
        output_limit: 10,
        profile_fingerprint: "original-model-bounds".into(),
        profile: json!({"api":"openai-completions"}),
        initiator: None,
        execution_stamp: Some(t.execution().stamp(task).unwrap()),
        closed: false,
    }
}
fn new_call(id: &str, worker: &Worker) -> NewCall {
    NewCall { id: id.into(), worker: worker.id.clone(), task: worker.task.clone(), output_limit: 10, tariff: price() }
}
fn grant(t: &Tracker, id: &str, w: &Worker) -> genie_core::model_calls::Call {
    match t.model_calls().admit(new_call(id, w), |_, _, _| Ok((Money::nanos(1_000_000)?, Money::default()))).unwrap() {
        Admission::Granted(call) => call,
        other => panic!("expected a fresh admission, got {other:?}"),
    }
}

#[test]
fn monetary_rounding_and_unknown_tariffs_cannot_grant_free_headroom() {
    assert_eq!(genie_core::model_calls::reserve(&price(), 100, 10).unwrap().value(), 120_000);
    assert_eq!(
        genie_core::model_calls::charge_tokens(&price(), &Tokens { input: 40, output: 5, cache_read: 5, ..Default::default() })
            .unwrap()
            .value(),
        52_500
    );
    assert!(Money::limit(-1.0).is_err());
    assert!(Money::limit(f64::NAN).is_err());
    assert!(Money::limit(0.0000000001).is_err());
    assert!(Money::charge(f64::INFINITY).is_err());
    assert!(Money::nanos(i64::MAX).unwrap().checked_add(Money::nanos(1).unwrap()).is_err());
    assert!(genie_core::model_calls::reserve(&ModelPrice { cache_read: None, ..price() }, 100, 10).is_err());
    let free = ModelPrice { input: 0.0, output: 0.0, cache_read: Some(0.0), cache_write: Some(0.0) };
    assert_eq!(genie_core::model_calls::reserve(&free, 100, 10).unwrap(), Money::default());
}

#[test]
fn two_sqlite_connections_cannot_spend_the_same_last_allowance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".genie");
    let t = Tracker::init(&path, None, None).unwrap();
    let task = task(&t);
    let w = worker(&t, &task, "shared");
    t.model_calls().enroll(&w).unwrap();
    drop(t);
    let start = std::sync::Arc::new(std::sync::Barrier::new(2));
    let results = std::thread::scope(|scope| {
        let workers = (0..2)
            .map(|n| {
                let (path, w, start) = (path.clone(), w.clone(), start.clone());
                scope.spawn(move || {
                    let t = Tracker::open(&path).unwrap();
                    start.wait();
                    t.model_calls()
                        .admit(new_call(&format!("call-{n}"), &w), |_, _, _| Ok((Money::nanos(180_000)?, Money::default())))
                        .unwrap()
                })
            })
            .collect::<Vec<_>>();
        workers.into_iter().map(|worker| worker.join().unwrap()).collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|r| matches!(r, Admission::Granted(_))).count(), 1);
    assert_eq!(results.iter().filter(|r| matches!(r, Admission::Refused(_))).count(), 1);
    let t = Tracker::open(&path).unwrap();
    let scope = Scope::Task(task);
    assert_eq!(t.model_calls().exposure(&scope).unwrap().value(), 120_000);
    assert_eq!(t.model_calls().pending(&scope).unwrap().value(), 120_000);
    assert_eq!(t.model_calls().committed(&scope).unwrap(), Money::default());
    let refusal = results.iter().find_map(|r| if let Admission::Refused(r) = r { Some(r) } else { None }).unwrap();
    assert!(refusal.temporarily_reserved(), "an active call may release unused allowance");
    let id = results.iter().find_map(|r| if let Admission::Granted(call) = r { Some(call.id.as_str()) } else { None }).unwrap();
    t.model_calls().settle(id, Tokens { input: 40, output: 5, cache_read: 5, ..Default::default() }).unwrap();
    assert_eq!(t.model_calls().pending(&scope).unwrap(), Money::default());
    assert_eq!(t.model_calls().committed(&scope).unwrap().value(), 52_500);
    let another =
        t.model_calls().admit(new_call("after-settlement", &w), |_, _, _| Ok((Money::nanos(180_000)?, Money::default()))).unwrap();
    assert!(matches!(another, Admission::Granted(_)), "settled unused exposure is available again");
    assert_eq!(t.model_calls().exposure(&Scope::Day).unwrap().value(), 172_500);
}

#[test]
fn settlement_is_idempotent_and_frozen_tariffs_do_not_double_charge_statistics() {
    let dir = tempfile::tempdir().unwrap();
    let t = Tracker::init(dir.path().join(".genie"), None, None).unwrap();
    let task = task(&t);
    let w = worker(&t, &task, "worker");
    t.model_calls().enroll(&w).unwrap();
    t.record_usage("worker", Some(&task), "fake/model", &Tokens { input: 7, ..Default::default() }).unwrap();
    grant(&t, "paid-once", &w);
    let mut retry = new_call("paid-once", &w);
    retry.tariff = ModelPrice { input: 100.0, ..price() };
    match t.model_calls().admit(retry, |_, _, _| panic!("a retry must retain its original allowance")).unwrap() {
        Admission::Existing(call) => assert_eq!(call.tariff, price()),
        other => panic!("retry can never dispatch another provider call: {other:?}"),
    }
    let tokens = Tokens { input: 40, output: 5, cache_read: 5, ..Default::default() };
    let first = t.model_calls().settle("paid-once", tokens).unwrap();
    assert_eq!(first.cost_nanos.unwrap().value(), 52_500);
    let last = t.last_event_id().unwrap();
    assert_eq!(t.model_calls().settle("paid-once", tokens).unwrap(), first);
    assert_eq!(t.last_event_id().unwrap(), last);
    assert!(t.model_calls().settle("paid-once", Tokens { input: 1, ..tokens }).is_err());
    let rows = t.usage_of_task(&task).unwrap();
    assert_eq!(rows[0].calls, 2);
    assert_eq!(rows[0].tokens.input, 47);
    let legacy = t.model_calls().legacy_rows(rows).unwrap();
    assert_eq!(legacy[0].calls, 1);
    assert_eq!(legacy[0].tokens, Tokens { input: 7, ..Default::default() });
}

#[test]
fn restart_retains_unknown_exposure_and_only_a_person_reconciles_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".genie");
    let t = Tracker::init(&path, None, None).unwrap();
    let task = task(&t);
    let w = worker(&t, &task, "worker");
    t.model_calls().enroll(&w).unwrap();
    grant(&t, "uncertain", &w);
    let person = Actor::new("owner", Role::Human);
    assert!(t.model_calls().reconcile(&person, "uncertain", Money::default(), "Too soon").is_err());
    drop(t);
    let t = Tracker::open(&path).unwrap();
    assert_eq!(t.model_calls().recover().unwrap().len(), 1);
    assert_eq!(t.model_calls().call("uncertain").unwrap().state, CallState::Uncertain);
    assert_eq!(t.model_calls().exposure(&Scope::Day).unwrap().value(), 120_000);
    assert_eq!(t.model_calls().pending(&Scope::Day).unwrap(), Money::default());
    assert_eq!(t.model_calls().committed(&Scope::Day).unwrap().value(), 120_000);
    let last = t.last_event_id().unwrap();
    assert!(t.model_calls().recover().unwrap().is_empty());
    assert_eq!(t.last_event_id().unwrap(), last);
    assert!(t.model_calls().admit(new_call("closed-worker", &w), |_, _, _| Ok((Money::default(), Money::default()))).is_err());
    let cost = Money::nanos(80_000).unwrap();
    assert!(t.model_calls().reconcile(&Actor::new("manager", Role::Orchestrator), "uncertain", cost, "Guess").is_err());
    let reconciled = t.model_calls().reconcile(&person, "uncertain", cost, "Checked provider receipt").unwrap();
    assert_eq!(reconciled.cost_nanos, Some(cost));
    assert_eq!(t.model_calls().exposure(&Scope::Day).unwrap(), cost);
    let last = t.last_event_id().unwrap();
    assert_eq!(t.model_calls().reconcile(&person, "uncertain", cost, "Checked provider receipt").unwrap(), reconciled);
    assert_eq!(t.last_event_id().unwrap(), last);
    assert!(t.model_calls().reconcile(&person, "uncertain", Money::default(), "Reverse it").is_err());
    let receipt = t.model_calls().settle("uncertain", Tokens { input: 90, ..Default::default() }).unwrap();
    assert_eq!(receipt.cost_nanos.unwrap().value(), 90_000, "late authoritative usage replaces a human estimate");
}

#[test]
fn provider_bound_violations_cannot_be_repeated_with_the_same_profile() {
    let dir = tempfile::tempdir().unwrap();
    let t = Tracker::init(dir.path().join(".genie"), None, None).unwrap();
    let task = task(&t);
    let w = worker(&t, &task, "worker");
    t.model_calls().enroll(&w).unwrap();
    grant(&t, "overrun", &w);
    let receipt = t.model_calls().settle("overrun", Tokens { input: 101, output: 11, ..Default::default() }).unwrap();
    assert_eq!(receipt.cost_nanos.unwrap().value(), 123_000, "actual cost is never clamped to its old reservation");
    assert!(t.model_calls().profile_fault(&w.profile_fingerprint).unwrap().is_some());
    assert!(t.model_calls().admit(new_call("again", &w), |_, _, _| Ok((Money::default(), Money::default()))).is_err());
    let mut corrected = worker(&t, &task, "corrected");
    corrected.input_limit = 200;
    corrected.output_limit = 20;
    corrected.profile_fingerprint = "corrected-model-bounds".into();
    t.model_calls().enroll(&corrected).unwrap();
    grant(&t, "corrected", &corrected);
}

#[test]
fn schema_twelve_upgrade_preserves_legacy_usage_and_adds_the_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".genie");
    let t = Tracker::init(&path, None, None).unwrap();
    let task = task(&t);
    t.record_usage("worker", Some(&task), "fake/model", &Tokens { input: 7, ..Default::default() }).unwrap();
    t.conn().execute_batch("DROP TABLE model_call_scopes; DROP TABLE model_calls; DROP TABLE model_workers; DROP TABLE model_profile_faults; UPDATE meta SET value='12' WHERE key='schema';").unwrap();
    drop(t);
    let t = Tracker::open(&path).unwrap();
    assert_eq!(t.usage_of_task(&task).unwrap()[0].tokens.input, 7);
    let w = worker(&t, &task, "migrated");
    t.model_calls().enroll(&w).unwrap();
    grant(&t, "new", &w);
}

#[test]
fn moving_a_task_does_not_move_already_admitted_financial_exposure() {
    let dir = tempfile::tempdir().unwrap();
    let t = Tracker::init(dir.path().join(".genie"), None, None).unwrap();
    let actor = Actor::new("owner", Role::Human);
    let epic = |title: &str| {
        t.create(&actor, CreateInput { title: title.into(), task_type: Some(genie_core::TaskType::Epic), ..Default::default() }).unwrap().id
    };
    let old_parent = epic("Original epic");
    let new_parent = epic("New epic");
    let child =
        t.create(&actor, CreateInput { title: "Movable child".into(), parent: Some(old_parent.clone()), ..Default::default() }).unwrap().id;
    let w = worker(&t, &child, "original-parent");
    t.model_calls().enroll(&w).unwrap();
    grant(&t, "before-move", &w);
    t.update(&actor, &child, genie_core::UpdateInput { parent: Some(Some(new_parent.clone())), ..Default::default() }).unwrap();
    t.model_calls().settle("before-move", Tokens { input: 40, ..Default::default() }).unwrap();
    assert_eq!(t.model_calls().exposure(&Scope::Epic(old_parent)).unwrap().value(), 40_000);
    assert_eq!(t.model_calls().exposure(&Scope::Epic(new_parent.clone())).unwrap(), Money::default());
    assert!(
        t.model_calls().legacy_rows(t.usage_of_task(&new_parent).unwrap()).unwrap().is_empty(),
        "settled tokens are not charged a second time through the current ancestry"
    );
    assert!(t.model_calls().admit(new_call("stale-parent", &w), |_, _, _| Ok((Money::default(), Money::default()))).is_err());
    let fresh = worker(&t, &child, "new-parent");
    t.model_calls().enroll(&fresh).unwrap();
    grant(&t, "after-move", &fresh);
    assert_eq!(t.model_calls().exposure(&Scope::Epic(new_parent)).unwrap().value(), 120_000);
}
