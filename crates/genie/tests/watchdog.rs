//! The silent-team watchdog (G-118): a team that stops doing anything on a task in
//! progress is reported to the orchestrator once per silence streak, and waiting for a
//! person or for CI is not silence. No pi is involved: the watchdog is called directly.

mod common;

use chrono::{SecondsFormat, Utc};
use common::*;
use genie_core::inbox::NewQuestion;
use genie_core::repos::{Delivery, NewRepo};
use genie_core::team::{NewMember, NewTeam, ORCHESTRATOR, SendMail};
use genie_core::{Actor, CreateInput, Role, Status, StatusOptions};

/// A project with a team on a task, everything idle.
struct Rig {
    h: Harness,
    task: String,
    team: String,
}

fn rig(stall: u64, status: Status) -> Rig {
    let h = Harness::with_config(|c| c.runtime.stall_secs = stall);
    h.project("shop");
    let (task, team) = h
        .app
        .with_tracker("shop", |t| {
            let orch = Actor::new("orchestrator", Role::Orchestrator);
            let task = t.create(
                &orch,
                CreateInput {
                    title: "silent work".into(),
                    description: Some("do it".into()),
                    acceptance: vec!["it works".into()],
                    ..Default::default()
                },
            )?;
            t.set_status(&orch, &task.id, Status::Ready, StatusOptions::default())?;
            if status != Status::Ready {
                t.set_status(&orch, &task.id, status, StatusOptions { note: Some("which way?".into()), ..Default::default() })?;
            }
            let team = t.bus().create(
                "orchestrator",
                "orchestrator",
                NewTeam {
                    id: task.id.clone(),
                    task: task.id.clone(),
                    cwd: "/tmp".into(),
                    members: vec![
                        NewMember { name: "bender".into(), role: "executor".into(), ..Default::default() },
                        NewMember { name: "yoda".into(), role: "reviewer".into(), ..Default::default() },
                    ],
                    ..Default::default()
                },
            )?;
            Ok((task.id, team.id))
        })
        .unwrap();
    Rig { h, task, team }
}

fn ago(secs: i64) -> String {
    (Utc::now() - chrono::Duration::seconds(secs)).to_rfc3339_opts(SecondsFormat::Millis, true)
}

impl Rig {
    /// Move every sign of life `secs` into the past, as if the team had been quiet since
    /// then. Reports of earlier streaks go further back, so they cannot suppress a new one.
    fn quieten(&self, secs: i64) {
        self.h
            .app
            .with_tracker("shop", |t| {
                t.conn().execute("UPDATE teams SET created = ?1, updated = ?1", [&ago(secs)])?;
                t.conn().execute("UPDATE members SET activity_at = NULL, heartbeat_at = NULL", [])?;
                t.conn().execute("UPDATE mail SET at = ?1", [&ago(secs)])?;
                t.conn().execute("UPDATE log SET at = ?1", [&ago(secs + 60)])?;
                Ok(())
            })
            .unwrap();
    }

    /// What the orchestrator has been told so far.
    fn letters(&self) -> Vec<String> {
        self.h
            .app
            .with_tracker("shop", |t| {
                t.bus().pending(None, ORCHESTRATOR).map(|mails| mails.into_iter().map(|m| m.text).collect::<Vec<_>>())
            })
            .unwrap()
    }

    fn team_log(&self) -> Vec<String> {
        self.h
            .app
            .with_tracker("shop", |t| {
                t.bus().read_log(&self.team, 20).map(|rows| rows.iter().filter_map(|v| v["event"].as_str().map(str::to_string)).collect())
            })
            .unwrap()
    }

    async fn watch(&self) {
        genie::sessions::watch_silent_teams(&self.h.app).await.unwrap();
    }

    fn send(&self, from: &str, to: &str, text: &str) -> i64 {
        self.h
            .app
            .with_tracker("shop", |t| {
                let mails = t.bus().send(SendMail {
                    team: &self.team,
                    from,
                    from_role: "orchestrator",
                    to,
                    text,
                    kind: "message",
                    ..Default::default()
                })?;
                Ok(mails.into_iter().map(|m| m.id).collect::<Vec<_>>())
            })
            .unwrap()[0]
    }
}

/// AC1: a quiet team is reported, and only once — a second tick (or a restarted server)
/// does not repeat the letter, and the marker that says so lives in the journal.
#[tokio::test]
async fn a_silent_team_wakes_the_orchestrator() {
    let r = rig(900, Status::InProgress);
    r.watch().await;
    assert!(r.letters().is_empty(), "a team that was just alive is not silence");

    // Two hours with nothing said and nobody working: the orchestrator hears about it.
    r.quieten(7200);
    r.watch().await;
    let said = r.letters();
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].contains(&r.team) && said[0].contains(&r.task), "the letter names the team and the task:\n{}", said[0]);
    assert!(said[0].contains("has been silent for"), "{}", said[0]);
    assert!(said[0].contains("genie team board") && said[0].contains("genie team peek"), "it says what to do:\n{}", said[0]);
    assert!(r.team_log().contains(&"team_silent".to_string()), "the streak is marked in the journal: {:?}", r.team_log());

    r.watch().await;
    assert_eq!(r.letters().len(), 1, "one letter per silence streak, however often the tick runs");

    // Mail waiting to be delivered is not silence either.
    r.send(ORCHESTRATOR, "bender", "how is it going?");
    r.watch().await;
    assert_eq!(r.letters().len(), 1, "mail nobody has picked up yet is not silence");

    // Read and answered: a new silence streak, a new letter.
    r.h.app
        .with_tracker("shop", |t| {
            let n = t.bus().lease(Some(&r.team), "bender", 1)?.len();
            t.bus().complete_lease(1)?;
            Ok(n)
        })
        .unwrap();
    r.watch().await;
    assert_eq!(r.letters().len(), 1, "a fresh letter is a sign of life");
    r.quieten(7200);
    r.watch().await;
    assert_eq!(r.letters().len(), 2, "a sign of life older than the report opens a new streak");
}

/// AC2: waiting for a person is not silence — the task is blocked, waits for the owner's
/// decision, or someone was asked a question and has not answered.
#[tokio::test]
async fn waiting_for_a_person_is_not_silence() {
    let blocked = rig(900, Status::InProgress);
    blocked
        .h
        .app
        .with_tracker("shop", |t| t.block(&Actor::new("orchestrator", Role::Orchestrator), &blocked.task, "no credentials"))
        .unwrap();
    blocked.quieten(7200);
    blocked.watch().await;
    assert!(blocked.letters().is_empty(), "a blocked task is a wait, not silence");

    let owner = rig(900, Status::NeedsOwner);
    owner.quieten(7200);
    owner.watch().await;
    assert!(owner.letters().is_empty(), "waiting for the owner's decision is not silence");

    let asked = rig(900, Status::InProgress);
    let anna = asked.h.app.with_server(|db| db.create_user("anna", "Anna", None, None, false).map(|u| u.id)).unwrap();
    asked
        .h
        .app
        .with_server(|db| {
            db.create_questionnaire(
                "shop",
                Some(&asked.task),
                "bender",
                anna,
                "web",
                &[NewQuestion { text: "CSV or XLSX?".into(), why: "the library".into(), options: vec![] }],
                None,
                None,
                None,
            )
            .map(|_| ())
        })
        .unwrap();
    asked.quieten(7200);
    asked.watch().await;
    assert!(asked.letters().is_empty(), "a task waiting for answers is not silence");

    // Positive control: without the wait the very same team is reported.
    let plain = rig(900, Status::InProgress);
    plain.quieten(7200);
    plain.watch().await;
    assert_eq!(plain.letters().len(), 1, "without a wait the watchdog speaks");
}

/// AC2: waiting for CI or for a request to be reviewed or merged is not silence.
#[tokio::test]
async fn pending_ci_is_not_silence() {
    for state in ["pending", "stalled"] {
        let checks = rig(900, Status::InProgress);
        checks
            .h
            .app
            .with_server(|db| {
                db.add_repo("shop", repo("api"))?;
                db.set_task_repos("shop", &checks.task, &[("api".into(), "write".into())])?;
                db.update_delivery("shop", &checks.task, "api", Delivery { ci_state: Some(state.into()), ..Default::default() })?;
                Ok(())
            })
            .unwrap();
        checks.quieten(7200);
        checks.watch().await;
        assert!(checks.letters().is_empty(), "checks {state} are still a wait, not silence");
    }

    let request = rig(900, Status::InProgress);
    request
        .h
        .app
        .with_server(|db| {
            db.add_repo("shop", repo("api"))?;
            db.set_task_repos("shop", &request.task, &[("api".into(), "write".into())])?;
            db.update_delivery(
                "shop",
                &request.task,
                "api",
                Delivery { state: Some("published".into()), cr_state: Some("open".into()), cr_number: Some(7), ..Default::default() },
            )?;
            Ok(())
        })
        .unwrap();
    request.quieten(7200);
    request.watch().await;
    assert!(request.letters().is_empty(), "a request waiting to be reviewed or merged is not silence");

    // Positive control: once the checks are over and the request is merged, silence is silence.
    request
        .h
        .app
        .with_server(|db| {
            db.update_delivery(
                "shop",
                &request.task,
                "api",
                Delivery { ci_state: Some("passed".into()), cr_state: Some("merged".into()), ..Default::default() },
            )?;
            Ok(())
        })
        .unwrap();
    request.quieten(7200);
    request.watch().await;
    assert_eq!(request.letters().len(), 1, "nothing is waited for any more");
}

fn repo(name: &str) -> NewRepo {
    NewRepo {
        name: name.into(),
        host: "h".into(),
        remote: format!("acme/{name}"),
        mount: Some(name.into()),
        access: Some("write".into()),
        ..Default::default()
    }
}

/// A team whose members are at work is not silence even after a long quiet spell — that is
/// G-80's step watchdog, and a member that gave up is already its news too.
#[tokio::test]
async fn a_working_member_is_not_silence() {
    let r = rig(900, Status::InProgress);
    r.quieten(7200);
    r.h.app.with_tracker("shop", |t| t.bus().member_working(&r.team, "bender", serde_json::json!({ "kind": "turn" }))).unwrap();
    r.watch().await;
    assert!(r.letters().is_empty(), "a member at work is not silence");

    r.h.app.with_tracker("shop", |t| t.bus().member_gave_up(&r.team, "bender", "model error", 3)).unwrap();
    r.h.app.with_tracker("shop", |t| Ok(t.conn().execute("UPDATE members SET activity_at = NULL, heartbeat_at = NULL", [])?)).unwrap();
    r.watch().await;
    assert!(r.letters().is_empty(), "an error is the agent watchdog's news, not this one's");
}

/// The threshold is a real threshold: less than `stallSecs` of silence is left alone.
#[tokio::test]
async fn the_threshold_decides() {
    let r = rig(3600, Status::InProgress);
    r.quieten(1800);
    r.watch().await;
    assert!(r.letters().is_empty(), "half an hour of silence is not an hour");
    r.quieten(7200);
    r.watch().await;
    assert_eq!(r.letters().len(), 1, "two hours of silence is more than an hour");
}

/// `stallSecs = 0` switches the watchdog off, and a project with no teams is not a problem.
#[tokio::test]
async fn the_watchdog_can_be_switched_off() {
    let off = rig(0, Status::InProgress);
    off.quieten(7200);
    off.watch().await;
    assert!(off.letters().is_empty(), "stallSecs = 0 disables the watchdog");

    let empty = Harness::with_config(|c| c.runtime.stall_secs = 900);
    empty.project("shop");
    genie::sessions::watch_silent_teams(&empty.app).await.unwrap();
    assert!(empty.app.with_tracker("shop", |t| t.bus().pending(None, ORCHESTRATOR)).unwrap().is_empty());
}
