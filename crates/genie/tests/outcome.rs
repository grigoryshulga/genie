//! How a run ended decides what follows, the same for turns and live sessions: failures in a
//! row count across both, the last allowed one puts the member in `error` with the reason and
//! tells the orchestrator, and nothing but a restart brings it back. No agent process runs here.

use std::path::PathBuf;
use std::sync::Arc;

use genie::config::Config;
use genie::outcome::{self, Outcome};
use genie::runtime::AgentKey;
use genie::state::App;
use genie_core::team::{Member, NewMember, NewTeam, ORCHESTRATOR};
use genie_core::{Activity, Actor, CreateInput, MemberState, Role};

fn app() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::load(dir.path()).unwrap();
    cfg.runtime.enabled = false;
    cfg.runtime.max_attempts = 3;
    let app = App::open(dir.path(), cfg, PathBuf::from("/nonexistent")).unwrap();
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
    (dir, app)
}

fn bender() -> AgentKey {
    AgentKey::Member { project: "shop".into(), team: "G-1".into(), member: "bender".into() }
}

fn member(app: &App) -> Member {
    app.with_tracker("shop", |t| t.bus().get("G-1")).unwrap().members.into_iter().find(|m| m.name == "bender").unwrap()
}

fn orchestrator_mail(app: &App) -> Vec<String> {
    app.with_tracker("shop", |t| t.bus().pending(None, ORCHESTRATOR)).unwrap().into_iter().map(|m| m.text).collect()
}

fn failed(app: &App) -> outcome::Verdict {
    outcome::record(app, &bender(), Some("G-1"), Outcome::Failed { error: "model error: overloaded", log: "…the end of the log" })
}

#[test]
fn failures_in_turns_and_sessions_count_together_and_the_last_one_gives_up() {
    let (_d, app) = app();
    let before = orchestrator_mail(&app).len();
    let first = failed(&app);
    assert_eq!((first.failures, first.gave_up), (1, false));
    assert!(!app.attempts.may_start(&bender()), "it waits out a backoff");
    // A crash of its live session continues the streak.
    let second = outcome::record(&app, &bender(), Some("G-1"), Outcome::Crashed { code: Some(1), stderr: "panic", at_work: false });
    assert_eq!((second.failures, second.gave_up), (2, false));
    assert_eq!(member(&app).state, MemberState::Active);
    assert_eq!(orchestrator_mail(&app).len(), before, "nobody is told before the agent gives up");

    let third = failed(&app);
    assert_eq!((third.failures, third.gave_up), (3, true));
    let m = member(&app);
    assert_eq!((m.state, m.activity), (MemberState::Error, Activity::Error));
    assert_eq!(m.status, "stopped: model error: overloaded", "the board shows why");
    let mail = orchestrator_mail(&app);
    assert_eq!(mail.len(), before + 1, "{mail:?}");
    let notice = mail.last().unwrap();
    assert!(notice.contains("failed 3 runs in a row (model error: overloaded)"), "{notice}");
    assert!(notice.contains("genie team restart G-1 bender") && notice.contains("…the end of the log"), "{notice}");

    // Its session stops afterwards: it stays given up.
    outcome::record(&app, &bender(), Some("G-1"), Outcome::Stopped);
    assert_eq!(member(&app).state, MemberState::Error);

    genie::runtime::restart_member(&app, "shop", "G-1", "bender").unwrap();
    assert_eq!(member(&app).state, MemberState::Active);
    assert_eq!(app.attempts.failures(&bender()), 0);
}

#[test]
fn a_run_that_did_not_fail_ends_the_streak() {
    let (_d, app) = app();
    failed(&app);
    failed(&app);
    let ran = outcome::record(&app, &bender(), Some("G-1"), Outcome::Ran);
    assert_eq!((ran.failures, ran.gave_up), (0, false));
    assert!(app.attempts.may_start(&bender()));
    assert_eq!(failed(&app).failures, 1, "the next failure starts a new streak");
}

#[test]
fn a_crash_at_work_leaves_the_agent_a_note_to_continue() {
    let (_d, app) = app();
    outcome::record(&app, &bender(), Some("G-1"), Outcome::Crashed { code: Some(137), stderr: "", at_work: true });
    let mail = app.with_tracker("shop", |t| t.bus().pending(Some("G-1"), "bender")).unwrap();
    assert_eq!(mail.len(), 1);
    assert!(mail[0].text.contains("restarted after a crash (exit Some(137))"), "{}", mail[0].text);
}
