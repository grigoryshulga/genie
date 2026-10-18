use genie_core::{intelligence::*, *};
use serde_json::json;
fn fresh() -> (tempfile::TempDir, Tracker, String) {
    let d = tempfile::tempdir().unwrap();
    let t = Tracker::init(d.path(), Some("G"), None).unwrap();
    let task = t.create(&Actor::new("owner", Role::Human), CreateInput { title: "Task".into(), ..Default::default() }).unwrap();
    (d, t, task.id)
}
fn profile() -> SignalProfile {
    SignalProfile {
        schema_version: 1,
        brief_version: 0,
        brief_fingerprint: "brief-v1".into(),
        project_profile_version: "policy-v1".into(),
        extractor_version: "test-v1".into(),
        signals: SignalName::ALL.into_iter().map(|n| Signal::unknown(n, "test-v1")).collect(),
        affected_paths: vec![],
    }
}
fn plan() -> RoutingPlan {
    RoutingPlan {
        decision_kind: DecisionKind::Initial,
        router_provider: "configured".into(),
        router_version: "1".into(),
        policy_version: "1".into(),
        eligible_templates: vec!["solo".into()],
        selected_template: Some("solo".into()),
        selected_models: vec![],
        required_checks: vec![],
        decision_status: RoutingStatus::Selected,
        override_reason: None,
        reason_codes: vec![],
        provider_decision_ids: vec![],
    }
}
#[test]
fn immutable_revisions_and_attempts_survive_restart() {
    let (d, t, id) = fresh();
    let a = Actor::new("owner", Role::Human);
    let s1 = t.append_signal_snapshot(&a, &id, &profile(), None).unwrap();
    let mut p = profile();
    p.brief_version = 2;
    let s2 = t.append_signal_snapshot(&a, &id, &p, None).unwrap();
    assert_eq!(s2.revision, 2);
    assert_eq!(t.signal_snapshot(&id, s1.id).unwrap().profile.brief_version, 0);
    let attempt = t.begin_attempt(&a, &id).unwrap();
    assert_eq!(t.begin_attempt(&a, &id).unwrap().id, attempt.id);
    let r = t.append_routing(&a, &id, Some(attempt.id), s1.id, "dispatch-1", &plan()).unwrap();
    assert_eq!(t.append_routing(&a, &id, Some(attempt.id), s1.id, "dispatch-1", &plan()).unwrap().id, r.id);
    let event = t.tx(|| t.append_event("test.delivered", Some(&id), "owner", "human", json!({}))).unwrap();
    t.finalize_attempt(&a, &id, "accepted", event).unwrap();
    let next = t.begin_attempt(&a, &id).unwrap();
    assert_eq!(next.ordinal, 2);
    drop(t);
    let t = Tracker::open(d.path()).unwrap();
    let h = t.intelligence_history(&id).unwrap();
    assert_eq!(h.attempts.len(), 2);
    assert_eq!(h.snapshots.len(), 2);
    assert_eq!(h.routing.len(), 1);
    assert_eq!(h.assessments.len(), 1);
}
#[test]
fn malformed_or_cross_task_evidence_is_rejected_without_journal_changes() {
    let (_d, t, id) = fresh();
    let actor = Actor::new("owner", Role::Human);
    let other = t.create(&actor, CreateInput { title: "Other".into(), ..Default::default() }).unwrap();
    let event = t.tx(|| t.append_event("test", Some(&other.id), "owner", "human", json!({}))).unwrap();
    assert!(t.append_signal_snapshot(&actor, &id, &profile(), Some(event)).is_err());
    let mut p = profile();
    p.signals[0].value = "low".into();
    assert!(t.append_signal_snapshot(&actor, &id, &p, None).is_err());
    p = profile();
    p.signals[0].confidence = Some(f64::NAN);
    assert!(t.append_signal_snapshot(&actor, &id, &p, None).is_err());
    p = profile();
    p.signals[0].name = p.signals[1].name;
    assert!(t.append_signal_snapshot(&actor, &id, &p, None).is_err());
    let s = t.append_signal_snapshot(&actor, &other.id, &profile(), None).unwrap();
    assert!(t.append_routing(&actor, &id, None, s.id, "foreign", &plan()).is_err());
    let own = t.append_signal_snapshot(&actor, &id, &profile(), None).unwrap();
    let foreign = t.begin_attempt(&actor, &other.id).unwrap();
    assert!(t.append_routing(&actor, &id, Some(foreign.id), own.id, "foreign-attempt", &plan()).is_err());
    assert!(t.intelligence_history(&id).unwrap().routing.is_empty());
}
#[test]
fn policy_keeps_unknown_scope_protected_and_path_matching_is_bounded() {
    let p = IntelligencePolicy {
        enabled: true,
        components: vec![ProtectedComponent {
            id: "auth".into(),
            paths: vec!["src/auth".into()],
            sensitivity: "security".into(),
            required_checks: vec!["security_review".into()],
        }],
        checks: std::collections::BTreeMap::from([("security_review".into(), "reviewer".into())]),
        ..Default::default()
    };
    p.validate().unwrap();
    assert_eq!(p.required(&[]), vec!["security_review"]);
    assert_eq!(p.required(&["src/auth/login.rs".into()]), vec!["security_review"]);
    assert!(p.required(&["src/author.rs".into()]).is_empty());
    assert!(!valid_path("../secret"));
    assert!(!valid_path("/etc/passwd"));
    assert!(!valid_path("src/../auth"));
    assert!(!valid_path("src/*"));
}
#[test]
fn status_lifecycle_reuses_attempt_for_review_returns_and_creates_one_after_terminal() {
    let (_d, t, id) = fresh();
    let actor = Actor::new("owner", Role::Human);
    let to = |status| t.set_status(&actor, &id, status, StatusOptions { force: true, ..Default::default() }).unwrap();
    to(Status::InProgress);
    to(Status::Review);
    to(Status::ChangesRequested);
    to(Status::InProgress);
    assert_eq!(t.execution_attempts(&id).unwrap().len(), 1);
    to(Status::Cancelled);
    to(Status::InProgress);
    assert_eq!(t.execution_attempts(&id).unwrap().len(), 2);
    assert_eq!(t.intelligence_history(&id).unwrap().assessments[0].result, "cancelled");
}

#[test]
fn schema_six_upgrade_keeps_existing_data_and_does_not_guess_history() {
    let (d, t, id) = fresh();
    t.conn().execute("UPDATE meta SET value='6' WHERE key='schema'", []).unwrap();
    t.conn().execute_batch("DROP TABLE usage_attributions; DROP TABLE policy_evaluations; DROP TABLE routing_decisions; DROP TABLE task_signal_snapshots; DROP TABLE execution_assessments; DROP TABLE execution_attempts;").unwrap();
    let before = t.events_after(0, 1000).unwrap().len();
    drop(t);
    let t = Tracker::open(d.path()).unwrap();
    assert_eq!(t.get(&id).unwrap().title, "Task");
    assert_eq!(t.events_after(0, 1000).unwrap().len(), before);
    assert!(t.intelligence_history(&id).unwrap().snapshots.is_empty());
    assert!(t.intelligence_history(&id).unwrap().attempts.is_empty());
    let integrity: String = t.conn().query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
    assert_eq!(integrity, "ok");
}
