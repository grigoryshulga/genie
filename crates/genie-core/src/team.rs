//! Teams, members and peer-to-peer mail, stored in the project's tracker
//! database. Ported from the TypeScript version's team bus, extended for
//! live agent sessions:
//!
//! - a live session (a long-running `pi --mode rpc`) takes its mail in
//!   *deliveries*: at every step boundary it leases what is pending
//!   (`lease_delivery`), most urgent first and within a size budget, puts it into
//!   the session and acknowledges it once the model sees it (`ack_delivery`); an
//!   unacknowledged delivery is released and offered again, and ids the session
//!   already holds (`seen`) are settled instead of being injected twice;
//! - a turn-based agent (any other harness) leases its whole mailbox to a turn
//!   (`lease`) and the mail is delivered only when the turn succeeds;
//! - a message with a `topic` supersedes the sender's undelivered message on the
//!   same topic; an *ask* awaits a reply (`reply`, `take_reply`);
//! - the orchestrator's mailbox is its global box plus every team except teams
//!   stopped on purpose (their late mail is dropped, as before).

use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::db::now;
use crate::error::{GenieError, Result};
use crate::events;
use crate::model::{Activity, MemberState, TeamState};
use crate::tracker::Tracker;

pub const ORCHESTRATOR: &str = "orchestrator";
pub const BROADCAST: &str = "all";

/// Stops made on purpose: such teams are not revived and their late mail is dropped.
pub const DELIBERATE_STOPS: &[&str] = &["orchestrator", "owner", "task_closed", "budget"];
const DELIBERATE_SQL: &str = "('orchestrator', 'owner', 'task_closed', 'budget')";

/// `interrupt` stops the recipient's current step (orchestrator and people only).
pub const MAIL_LEVELS: &[&str] = &["low", "normal", "high", "interrupt"];
pub const MAIL_INTENTS: &[&str] = &["question", "blocker", "verdict", "done", "fyi"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
pub struct TeamWorktree {
    pub path: String,
    pub branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct Member {
    pub name: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    pub status: String,
    pub status_at: String,
    pub state: MemberState,
    pub activity: Activity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heartbeat_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "unknown")]
    pub runtime: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct Team {
    pub id: String,
    pub task: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<TeamWorktree>,
    pub state: TeamState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    pub created: String,
    pub updated: String,
    pub members: Vec<Member>,
    /// How the team works, fixed when it was assembled: the template, member
    /// keys, relations between members and the team charter.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "unknown")]
    pub spec: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct Mail {
    pub id: i64,
    pub at: String,
    /// `null` for the orchestrator's global mailbox.
    #[ts(optional = false)]
    pub team: Option<String>,
    pub from: String,
    pub from_role: String,
    pub to: String,
    pub text: String,
    pub urgent: bool,
    #[ts(type = r#""low" | "normal" | "high" | "interrupt""#)]
    pub level: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(type = r#""question" | "blocker" | "verdict" | "done" | "fyi""#)]
    pub intent: Option<String>,
    /// `message` | `kickoff` | `system` | `owner`
    #[ts(type = r#""message" | "kickoff" | "system" | "owner""#)]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// The message this one answers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<i64>,
    /// An ask: the sender waits for a reply.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>")]
    pub awaits: bool,
}

impl Mail {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<Mail> {
        let urgent: i64 = r.get("urgent")?;
        let level: String = r.get("level")?;
        let level = if urgent != 0 {
            "high".to_string()
        } else if MAIL_LEVELS.contains(&level.as_str()) {
            level
        } else {
            "normal".into()
        };
        Ok(Mail {
            id: r.get("id")?,
            at: r.get("at")?,
            team: r.get("team")?,
            from: r.get("sender")?,
            from_role: r.get("sender_role")?,
            to: r.get("recipient")?,
            text: r.get("text")?,
            urgent: level == "high",
            level,
            intent: r.get("intent")?,
            kind: r.get("kind")?,
            task: r.get("task")?,
            delivered_at: r.get("delivered_at")?,
            topic: r.get("topic")?,
            reply_to: r.get("reply_to")?,
            awaits: r.get::<_, i64>("awaits")? != 0,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct NewMember {
    pub name: String,
    pub role: String,
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub instructions: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct NewTeam {
    pub id: String,
    pub task: String,
    pub template: Option<String>,
    pub cwd: String,
    pub worktree: Option<TeamWorktree>,
    pub members: Vec<NewMember>,
    pub spec: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct SendMail<'a> {
    pub team: &'a str,
    pub from: &'a str,
    pub from_role: &'a str,
    pub to: &'a str,
    pub text: &'a str,
    pub level: Option<&'a str>,
    pub intent: Option<&'a str>,
    pub kind: &'a str,
    /// Supersedes the sender's undelivered message on the same topic to the same recipient.
    pub topic: Option<&'a str>,
    /// Answers this message.
    pub reply_to: Option<i64>,
    /// The sender waits for a reply (an ask).
    pub awaits: bool,
}

/// Mail handed to a live session in one step boundary.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub id: i64,
    pub mails: Vec<Mail>,
    /// Mail left pending because of the size budget (taken at the next boundary).
    pub more: usize,
}

/// A delivery not acknowledged in time (its session died or hung).
#[derive(Debug, Clone)]
pub struct OpenDelivery {
    pub id: i64,
    pub team: Option<String>,
    pub recipient: String,
    pub created: String,
}

/// A mailbox with unread, unleased mail: `(team, recipient)`; team `None` is the orchestrator.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Mailbox {
    pub team: Option<String>,
    pub recipient: String,
}

/// A team that has gone quiet: nobody is working, nothing is waiting to be delivered
/// and the last sign of life is older than the watch threshold. Deliberate states
/// (`paused`, `stopped`, `error`) are not silence, and neither is a task waiting for a
/// person or CI — the server filters those out, since it holds the delivery side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuietTeam {
    pub id: String,
    pub task: String,
    /// How many members the team has; every one of them is active and idle.
    pub members: i64,
    /// The last sign of life — a letter, a member's step or the team's creation (RFC 3339).
    pub since: String,
    /// How long the silence has lasted, in seconds.
    pub idle_secs: i64,
}

fn member_from_row(r: &Row<'_>) -> rusqlite::Result<Member> {
    let runtime: Option<String> = r.get("runtime")?;
    Ok(Member {
        name: r.get("name")?,
        role: r.get("role")?,
        model: r.get("model")?,
        thinking: r.get("thinking")?,
        instructions: r.get("instructions")?,
        status: r.get("status")?,
        status_at: r.get("status_at")?,
        state: r.get("state")?,
        activity: r.get("activity")?,
        activity_at: r.get("activity_at")?,
        heartbeat_at: r.get("heartbeat_at")?,
        runtime: runtime.and_then(|s| serde_json::from_str(&s).ok()),
        session_file: r.get("session_file")?,
    })
}

fn valid_member_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Team registry and mailboxes on top of a project tracker.
/// SQL (members `m`, teams `t`): the member may run — an active member of an active team.
const RUNNABLE: &str = "t.state = 'active' AND m.state = 'active'";
/// SQL (members `m`, teams `t`): the member's mail waits for it — paused and gave-up members
/// keep theirs; a stopped one (with its team) does not.
const KEEPS_MAIL: &str = "t.state = 'active' AND m.state <> 'stopped'";

pub struct Bus<'a> {
    t: &'a Tracker,
}

impl Tracker {
    pub fn bus(&self) -> Bus<'_> {
        Bus { t: self }
    }
}

impl Bus<'_> {
    fn conn(&self) -> &rusqlite::Connection {
        self.t.conn()
    }

    pub fn exists(&self, team: &str) -> Result<bool> {
        Ok(self.conn().query_row("SELECT 1 FROM teams WHERE id = ?1", [team], |_| Ok(())).optional()?.is_some())
    }

    pub fn get(&self, team: &str) -> Result<Team> {
        type Row = (String, String, Option<String>, String, Option<String>, TeamState, Option<String>, String, String, Option<String>);
        let row: Row = self
            .conn()
            .query_row(
                "SELECT id, task, template, cwd, worktree, state, stop_reason, created, updated, spec FROM teams WHERE id = ?1",
                [team],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?)),
            )
            .optional()?
            .ok_or_else(|| GenieError::not_found(format!("team {team} not found")))?;
        let mut stmt = self.conn().prepare_cached("SELECT * FROM members WHERE team = ?1 ORDER BY ord")?;
        let members = stmt.query_map([team], member_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Team {
            id: row.0,
            task: row.1,
            template: row.2,
            cwd: row.3,
            worktree: row.4.and_then(|w| serde_json::from_str(&w).ok()),
            state: row.5,
            stop_reason: row.6,
            created: row.7,
            updated: row.8,
            members,
            spec: row.9.and_then(|s| serde_json::from_str(&s).ok()),
        })
    }

    pub fn list(&self, include_stopped: bool) -> Result<Vec<Team>> {
        let sql = if include_stopped {
            "SELECT id FROM teams ORDER BY created"
        } else {
            "SELECT id FROM teams WHERE state = 'active' ORDER BY created"
        };
        let mut stmt = self.conn().prepare(sql)?;
        let ids = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        ids.iter().map(|id| self.get(id)).collect()
    }

    pub fn active_count(&self) -> Result<i64> {
        Ok(self.conn().query_row("SELECT COUNT(*) FROM teams WHERE state = 'active'", [], |r| r.get(0))?)
    }

    /// Active teams whose task is a child of `epic`.
    pub fn active_count_in_epic(&self, epic: &str) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM teams t JOIN tasks k ON k.id = t.task WHERE t.state = 'active' AND k.parent = ?1",
            [epic],
            |r| r.get(0),
        )?)
    }

    /// Active teams where nobody is working, nothing is waiting to be delivered and the
    /// last sign of life is older than `stall`. A team waiting for a person or CI is left
    /// to the caller: questionnaires, jobs and deliveries live on the server's side.
    pub fn quiet_teams(&self, stall: Duration) -> Result<Vec<QuietTeam>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT t.id, t.task, COUNT(m.name) AS members,
                    MAX(COALESCE(m.activity_at, m.heartbeat_at, t.created)) AS member_at,
                    (SELECT MAX(x.at) FROM mail x WHERE x.team = t.id) AS mail_at,
                    t.created
               FROM teams t JOIN members m ON m.team = t.id
              WHERE t.state = 'active'
              GROUP BY t.id
             HAVING SUM(CASE WHEN m.state = 'active' AND m.activity = 'idle' THEN 1 ELSE 0 END) = COUNT(*)
                AND NOT EXISTS (SELECT 1 FROM mail x WHERE x.team = t.id AND x.delivered_at IS NULL)",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let now = Utc::now();
        let mut out = Vec::new();
        for row in rows {
            let (id, task, members, member_at, mail_at, created) = row?;
            // The last sign of life: a letter, a member's step, or the team's own creation.
            let since = match (member_at, mail_at) {
                (Some(member), Some(mail)) => std::cmp::max(member, mail),
                (Some(member), None) => member,
                (None, Some(mail)) => mail,
                (None, None) => created,
            };
            let Ok(at) = DateTime::parse_from_rfc3339(&since) else { continue };
            let idle_secs = (now - at.with_timezone(&Utc)).num_seconds().max(0);
            if (idle_secs as u64) < stall.as_secs() {
                continue;
            }
            out.push(QuietTeam { id, task, members, since, idle_secs });
        }
        Ok(out)
    }

    /// When `event` last happened in a team's journal (`None` if it never did).
    pub fn last_event_at(&self, team: &str, event: &str) -> Result<Option<String>> {
        Ok(self.conn().query_row("SELECT MAX(at) FROM log WHERE team = ?1 AND event = ?2", params![team, event], |r| r.get(0))?)
    }

    /// A free team id derived from the task id: G-7, G-7b, G-7c…
    pub fn free_id(&self, task: &str) -> Result<String> {
        if !self.exists(task)? {
            return Ok(task.to_string());
        }
        for c in b'b'..=b'z' {
            let id = format!("{task}{}", c as char);
            if !self.exists(&id)? {
                return Ok(id);
            }
        }
        Ok(format!("{task}-{}", chrono::Utc::now().timestamp()))
    }

    /// Names used by members of active teams, so new members get distinct names.
    pub fn taken_names(&self) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn().prepare("SELECT m.name FROM members m JOIN teams t ON t.id = m.team WHERE t.state = 'active'")?;
        Ok(stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn log(&self, team: &str, event: &str, data: Value) -> Result<()> {
        self.conn()
            .execute("INSERT INTO log(team, at, event, data) VALUES (?1, ?2, ?3, ?4)", params![team, now(), event, data.to_string()])?;
        Ok(())
    }

    pub fn read_log(&self, team: &str, limit: i64) -> Result<Vec<Value>> {
        let mut stmt =
            self.conn().prepare("SELECT at, event, data FROM (SELECT * FROM log WHERE team = ?1 ORDER BY id DESC LIMIT ?2) ORDER BY id")?;
        let rows =
            stmt.query_map(params![team, limit], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        rows.map(|r| {
            let (at, event, data) = r?;
            let mut v: Value = serde_json::from_str(&data).unwrap_or_else(|_| json!({}));
            if let Some(o) = v.as_object_mut() {
                o.insert("at".into(), json!(at));
                o.insert("event".into(), json!(event));
            }
            Ok(v)
        })
        .collect()
    }

    fn insert_member(&self, team: &str, m: &NewMember, ord: i64) -> Result<()> {
        if !valid_member_name(&m.name) {
            return Err(GenieError::invalid(format!("member name \"{}\" must match [a-z][a-z0-9_-]*", m.name)));
        }
        if m.name == ORCHESTRATOR || m.name == BROADCAST {
            return Err(GenieError::invalid(format!("member name \"{}\" is reserved", m.name)));
        }
        let at = now();
        self.conn().execute(
            "INSERT INTO members(team, name, role, model, thinking, instructions, status, status_at, state, activity, runtime, session_file, ord)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'starting', ?7, 'active', 'idle', ?8, ?9, ?10)",
            params![
                team,
                m.name,
                m.role,
                m.model,
                m.thinking,
                m.instructions,
                at,
                json!({ "kind": "turn" }).to_string(),
                format!("{team}-{}", m.name).to_lowercase(),
                ord
            ],
        )?;
        Ok(())
    }

    pub fn create(&self, actor: &str, actor_role: &str, team: NewTeam) -> Result<Team> {
        self.t.tx(|| {
            if self.exists(&team.id)? {
                return Err(GenieError::invalid(format!("team {} already exists", team.id)));
            }
            let at = now();
            self.conn().execute(
                "INSERT INTO teams(id, task, template, cwd, worktree, state, created, updated, spec) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?6, ?7)",
                params![
                    team.id,
                    team.task,
                    team.template,
                    team.cwd,
                    team.worktree.as_ref().map(|w| json!(w).to_string()),
                    at,
                    team.spec.as_ref().map(Value::to_string)
                ],
            )?;
            for (i, m) in team.members.iter().enumerate() {
                self.insert_member(&team.id, m, i as i64)?;
            }
            let roster: Vec<String> = team.members.iter().map(|m| format!("{}:{}:{}", m.name, m.role, m.model.as_deref().unwrap_or("default"))).collect();
            self.log(&team.id, "team_created", json!({ "members": roster }))?;
            self.t.append_event(
                "team.spawned",
                Some(&team.task),
                actor,
                actor_role,
                json!({ "team": team.id, "template": team.template, "members": roster }),
            )?;
            Ok(())
        })?;
        self.get(&team.id)
    }

    /// Replace the snapshot of how the team works (a member joined or left).
    pub fn set_spec(&self, team: &str, spec: &Value) -> Result<()> {
        self.conn().execute("UPDATE teams SET spec = ?1, updated = ?2 WHERE id = ?3", params![spec.to_string(), now(), team])?;
        Ok(())
    }

    pub fn add_member(&self, team: &str, m: NewMember) -> Result<Team> {
        self.t.tx(|| {
            if self
                .conn()
                .query_row("SELECT 1 FROM members WHERE team = ?1 AND name = ?2", params![team, m.name], |_| Ok(()))
                .optional()?
                .is_some()
            {
                return Err(GenieError::invalid(format!("team {team} already has a member {}", m.name)));
            }
            let ord: i64 = self.conn().query_row("SELECT COUNT(*) FROM members WHERE team = ?1", [team], |r| r.get(0))?;
            self.insert_member(team, &m, ord)?;
            self.log(team, "member_added", json!({ "member": format!("{}:{}", m.name, m.role) }))
        })?;
        self.get(team)
    }

    /// Remove a member; its unread mail is closed (the history stays).
    pub fn remove_member(&self, team: &str, member: &str) -> Result<()> {
        self.t.tx(|| {
            let n = self.conn().execute("DELETE FROM members WHERE team = ?1 AND name = ?2", params![team, member])?;
            if n == 0 {
                return Err(GenieError::not_found(format!("team {team} has no member {member}")));
            }
            self.conn().execute(
                "UPDATE mail SET delivered_at = ?1, lease = NULL WHERE team = ?2 AND recipient = ?3 AND delivered_at IS NULL",
                params![now(), team, member],
            )?;
            self.conn().execute("UPDATE teams SET updated = ?1 WHERE id = ?2", params![now(), team])?;
            self.log(team, "member_removed", json!({ "member": member }))
        })
    }

    /// Stop (`stopped`, with a reason) or reactivate (`active`) a team.
    pub fn set_state(&self, team: &str, state: TeamState, reason: Option<&str>, actor: &str) -> Result<()> {
        let stopped = state == TeamState::Stopped;
        self.t.tx(|| {
            let task: String = self
                .conn()
                .query_row("SELECT task FROM teams WHERE id = ?1", [team], |r| r.get(0))
                .optional()?
                .ok_or_else(|| GenieError::not_found(format!("team {team} not found")))?;
            let reason = stopped.then(|| reason.unwrap_or("orchestrator"));
            self.conn().execute(
                "UPDATE teams SET state = ?1, stop_reason = ?2, updated = ?3 WHERE id = ?4",
                params![state, reason, now(), team],
            )?;
            if let Some(r) = reason
                && DELIBERATE_STOPS.contains(&r)
            {
                self.conn().execute("UPDATE members SET state = 'stopped', activity = 'idle' WHERE team = ?1", [team])?;
            }
            if !stopped {
                self.conn().execute("UPDATE members SET state = 'active' WHERE team = ?1 AND state = 'stopped'", [team])?;
            }
            self.log(team, if stopped { "team_stopped" } else { "team_started" }, json!({ "reason": reason, "by": actor }))?;
            self.t.append_event(
                if stopped { "team.stopped" } else { "team.started" },
                Some(&task),
                actor,
                "system",
                json!({ "team": team, "reason": reason }),
            )?;
            Ok(())
        })
    }

    /// Delete a team with its roster, mail and log.
    pub fn delete(&self, team: &str) -> Result<()> {
        self.t.tx(|| {
            for table in ["mail", "log", "members"] {
                self.conn().execute(&format!("DELETE FROM {table} WHERE team = ?1"), [team])?;
            }
            self.conn().execute("DELETE FROM teams WHERE id = ?1", [team])?;
            Ok(())
        })
    }

    /// The member's own status line (shown in the team card).
    pub fn set_member_status(&self, team: &str, member: &str, status: &str) -> Result<()> {
        if member == ORCHESTRATOR {
            return Ok(());
        }
        self.t.tx(|| {
            let n = self.conn().execute(
                "UPDATE members SET status = ?1, status_at = ?2 WHERE team = ?3 AND name = ?4",
                params![status, now(), team, member],
            )?;
            if n == 0 {
                return Err(GenieError::not_found(format!("team {team} has no member {member}")));
            }
            self.conn().execute("UPDATE teams SET updated = ?1 WHERE id = ?2", params![now(), team])?;
            self.log(team, "status", json!({ "member": member, "status": status }))
        })
    }

    /// The member's own model and thinking level; `None` goes back to its role's.
    pub fn set_member_model(&self, team: &str, member: &str, model: Option<&str>, thinking: Option<&str>, by: &str) -> Result<()> {
        self.t.tx(|| {
            let n = self.conn().execute(
                "UPDATE members SET model = ?1, thinking = ?2 WHERE team = ?3 AND name = ?4",
                params![model, thinking, team, member],
            )?;
            if n == 0 {
                return Err(GenieError::not_found(format!("team {team} has no member {member}")));
            }
            self.conn().execute("UPDATE teams SET updated = ?1 WHERE id = ?2", params![now(), team])?;
            self.log(team, "member_model", json!({ "member": member, "model": model, "thinking": thinking, "by": by }))
        })
    }

    /// Pause (`paused`) or resume (`active`) a member: a paused member keeps its mail
    /// but gets no deliveries and its session is stopped.
    pub fn set_paused(&self, team: &str, member: &str, paused: bool, by: &str) -> Result<()> {
        self.t.tx(|| {
            let (from, to) = if paused { ("active", "paused") } else { ("paused", "active") };
            let n = self
                .conn()
                .execute("UPDATE members SET state = ?1 WHERE team = ?2 AND name = ?3 AND state = ?4", params![to, team, member, from])?;
            if n == 0 {
                let state: Option<String> = self
                    .conn()
                    .query_row("SELECT state FROM members WHERE team = ?1 AND name = ?2", params![team, member], |r| r.get(0))
                    .optional()?;
                return match state {
                    None => Err(GenieError::not_found(format!("team {team} has no member {member}"))),
                    Some(s) if s == to => Ok(()),
                    Some(s) => Err(GenieError::invalid(format!("{member} is {s}, not {from}"))),
                };
            }
            self.log(team, if paused { "member_paused" } else { "member_resumed" }, json!({ "member": member, "by": by }))
        })
    }

    // --- a member's run: the only ways its state and activity change at runtime -------

    /// A run started (`runtime` says which: a turn or a session, its pid).
    pub fn member_working(&self, team: &str, member: &str, runtime: Value) -> Result<()> {
        self.conn().execute(
            "UPDATE members SET activity = 'working', activity_at = ?1, heartbeat_at = ?1, runtime = ?2 WHERE team = ?3 AND name = ?4",
            params![now(), runtime.to_string(), team, member],
        )?;
        Ok(())
    }

    /// A run ended or its session stopped: the member is idle — unless it gave up, which
    /// stays on the board until someone restarts it.
    pub fn member_idle(&self, team: &str, member: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE members SET activity = 'idle', activity_at = ?1, heartbeat_at = ?1 WHERE team = ?2 AND name = ?3 AND state != 'error'",
            params![now(), team, member],
        )?;
        Ok(())
    }

    /// The member failed `attempts` runs in a row and stops trying: `error`, with the reason
    /// as its status line (not the line it set when it started) and in the team log.
    pub fn member_gave_up(&self, team: &str, member: &str, error: &str, attempts: u32) -> Result<()> {
        self.t.tx(|| {
            let at = now();
            let why: String = error.chars().take(200).collect();
            self.conn().execute(
                "UPDATE members SET state = 'error', activity = 'error', activity_at = ?1, heartbeat_at = ?1, status = ?2, status_at = ?1
                 WHERE team = ?3 AND name = ?4",
                params![at, format!("stopped: {why}"), team, member],
            )?;
            self.conn().execute("UPDATE teams SET updated = ?1 WHERE id = ?2", params![at, team])?;
            self.log(team, "agent_error", json!({ "member": member, "error": error, "attempts": attempts }))
        })
    }

    /// Let a member that gave up work again (its kept mail is offered next).
    pub fn member_restarted(&self, team: &str, member: &str) -> Result<()> {
        self.t.tx(|| {
            self.conn().execute(
                "UPDATE members SET state = 'active', activity = 'idle', activity_at = ?1 WHERE team = ?2 AND name = ?3 AND state = 'error'",
                params![now(), team, member],
            )?;
            self.log(team, "member_restarted", json!({ "member": member }))
        })
    }

    /// Whether the member may run now: an active member of an active team.
    pub fn runnable(&self, team: &str, member: &str) -> Result<bool> {
        Ok(self
            .conn()
            .query_row(
                &format!("SELECT 1 FROM members m JOIN teams t ON t.id = m.team WHERE m.team = ?1 AND m.name = ?2 AND {RUNNABLE}"),
                params![team, member],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// The team working on a task: the task's team, if it is active.
    pub fn active_team_of(&self, task: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT t.id FROM tasks k JOIN teams t ON t.id = k.team WHERE k.id = ?1 AND t.state = 'active'", [task], |r| {
                r.get(0)
            })
            .optional()?)
    }

    /// Deliver a message. `to` is a member name, `orchestrator` or `all` (everyone but the sender).
    pub fn send(&self, m: SendMail<'_>) -> Result<Vec<Mail>> {
        let level = m.level.unwrap_or("normal");
        if !MAIL_LEVELS.contains(&level) {
            return Err(GenieError::invalid(format!("invalid mail level \"{level}\"; expected one of {}", MAIL_LEVELS.join(", "))));
        }
        if let Some(i) = m.intent
            && !MAIL_INTENTS.contains(&i)
        {
            return Err(GenieError::invalid(format!("invalid mail intent \"{i}\"; expected one of {}", MAIL_INTENTS.join(", "))));
        }
        if m.text.trim().is_empty() {
            return Err(GenieError::invalid("message text is empty"));
        }
        let team = self.get(m.team)?;
        let mut names: Vec<String> = team.members.iter().map(|x| x.name.clone()).collect();
        names.push(ORCHESTRATOR.into());
        let recipients: Vec<String> = if m.to == BROADCAST {
            names.iter().filter(|n| *n != m.from).cloned().collect()
        } else if names.iter().any(|n| n == m.to) {
            vec![m.to.to_string()]
        } else {
            return Err(GenieError::invalid(format!("team {} has no member \"{}\". Members: {}", team.id, m.to, names.join(", "))));
        };
        let topic = m.topic.map(str::trim).filter(|t| !t.is_empty());
        let ids = self.t.tx(|| {
            let at = now();
            let mut ids = Vec::new();
            for to in &recipients {
                // `urgent` stays the legacy flag for `high` only: both trackers' migrations
                // rewrite an urgent row to `high`, which would silently demote an interrupt.
                self.conn().execute(
                    "INSERT INTO mail(team, at, sender, sender_role, recipient, text, urgent, level, intent, kind, task, topic, reply_to, awaits)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                    params![
                        team.id,
                        at,
                        m.from,
                        m.from_role,
                        to,
                        m.text,
                        (level == "high") as i64,
                        level,
                        m.intent,
                        m.kind,
                        team.task,
                        topic,
                        m.reply_to,
                        m.awaits as i64
                    ],
                )?;
                let id = self.conn().last_insert_rowid();
                if let Some(topic) = topic {
                    self.conn().execute(
                        "UPDATE mail SET delivered_at = ?1, superseded_by = ?2
                         WHERE team = ?3 AND sender = ?4 AND recipient = ?5 AND topic = ?6 AND id <> ?2
                           AND delivered_at IS NULL AND lease IS NULL AND delivery IS NULL",
                        params![at, id, team.id, m.from, to, topic],
                    )?;
                }
                ids.push(id);
            }
            let short: String = m.text.chars().take(500).collect();
            self.log(&team.id, "mail", json!({ "from": m.from, "to": m.to, "level": level, "intent": m.intent, "text": short }))?;
            self.t.append_event(
                events::MAIL_SENT,
                Some(&team.task),
                m.from,
                m.from_role,
                json!({ "team": team.id, "to": m.to, "recipients": recipients, "level": level, "intent": m.intent, "kind": m.kind }),
            )?;
            Ok(ids)
        })?;
        ids.iter().map(|id| self.mail(*id)).collect()
    }

    /// A system message to the orchestrator's global mailbox.
    pub fn notify_orchestrator(&self, from: &str, from_role: &str, kind: &str, text: &str, task: Option<&str>) -> Result<()> {
        self.t.tx(|| {
            self.conn().execute(
                "INSERT INTO mail(team, at, sender, sender_role, recipient, text, urgent, kind, task) VALUES (NULL, ?1, ?2, ?3, 'orchestrator', ?4, 0, ?5, ?6)",
                params![now(), from, from_role, text, kind, task],
            )?;
            self.t.append_event(events::MAIL_SENT, task, from, from_role, json!({ "to": ORCHESTRATOR, "kind": kind }))?;
            Ok(())
        })
    }

    pub fn mail(&self, id: i64) -> Result<Mail> {
        Ok(self.conn().query_row("SELECT * FROM mail WHERE id = ?1", [id], Mail::from_row)?)
    }

    /// Messages of a team, newest last.
    pub fn history(&self, team: &str, limit: i64) -> Result<Vec<Mail>> {
        let mut stmt = self.conn().prepare("SELECT * FROM (SELECT * FROM mail WHERE team = ?1 ORDER BY id DESC LIMIT ?2) ORDER BY id")?;
        Ok(stmt.query_map(params![team, limit], Mail::from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    /// Unread, unleased mail for one member (or, with `team == None`, the orchestrator).
    fn unclaimed_sql(team: Option<&str>) -> String {
        match team {
            Some(_) => {
                "SELECT * FROM mail WHERE team = ?1 AND recipient = ?2 AND delivered_at IS NULL AND lease IS NULL AND delivery IS NULL ORDER BY id"
                    .into()
            }
            None => format!(
                "SELECT * FROM mail WHERE ?1 IS NULL AND recipient = ?2 AND delivered_at IS NULL AND lease IS NULL AND delivery IS NULL
                 AND (team IS NULL OR team NOT IN (SELECT id FROM teams WHERE state = 'stopped' AND COALESCE(stop_reason, 'orchestrator') IN {DELIBERATE_SQL}))
                 ORDER BY id"
            ),
        }
    }

    pub fn pending(&self, team: Option<&str>, recipient: &str) -> Result<Vec<Mail>> {
        let mut stmt = self.conn().prepare(&Self::unclaimed_sql(team))?;
        Ok(stmt.query_map(params![team, recipient], Mail::from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn pending_count(&self, team: &str, member: &str) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM mail WHERE team = ?1 AND recipient = ?2 AND delivered_at IS NULL",
            params![team, member],
            |r| r.get(0),
        )?)
    }

    /// Lease a mailbox's unread mail to a turn. Returns the leased messages.
    pub fn lease(&self, team: Option<&str>, recipient: &str, turn: i64) -> Result<Vec<Mail>> {
        self.t.tx(|| {
            let rows = self.pending(team, recipient)?;
            for r in &rows {
                self.conn().execute("UPDATE mail SET lease = ?1 WHERE id = ?2", params![turn, r.id])?;
            }
            Ok(rows)
        })
    }

    /// The turn succeeded: its leased mail is delivered.
    pub fn complete_lease(&self, turn: i64) -> Result<usize> {
        Ok(self.conn().execute("UPDATE mail SET delivered_at = ?1 WHERE lease = ?2 AND delivered_at IS NULL", params![now(), turn])?)
    }

    /// The turn failed or was interrupted: its mail is offered again.
    pub fn release_lease(&self, turn: i64) -> Result<usize> {
        Ok(self.conn().execute("UPDATE mail SET lease = NULL WHERE lease = ?1 AND delivered_at IS NULL", [turn])?)
    }

    /// After a restart no turn or session is running: every open lease and delivery is released.
    pub fn release_all_leases(&self) -> Result<usize> {
        self.t.tx(|| {
            let turns = self.conn().execute("UPDATE mail SET lease = NULL WHERE lease IS NOT NULL AND delivered_at IS NULL", [])?;
            let open: Vec<i64> = {
                let mut stmt = self.conn().prepare("SELECT id FROM deliveries WHERE acked_at IS NULL AND released_at IS NULL")?;
                stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
            };
            let mut n = turns;
            for id in open {
                n += self.release_delivery(id)?;
            }
            Ok(n)
        })
    }

    /// Mail addressed to a member that cannot run (removed, stopped team): drop it quietly.
    pub fn close_undeliverable(&self) -> Result<usize> {
        Ok(self.conn().execute(
            &format!(
                "UPDATE mail SET delivered_at = ?1 WHERE delivered_at IS NULL AND recipient <> 'orchestrator' AND team IS NOT NULL
                 AND NOT EXISTS (SELECT 1 FROM members m JOIN teams t ON t.id = m.team
                                 WHERE m.team = mail.team AND m.name = mail.recipient AND {KEEPS_MAIL})"
            ),
            [now()],
        )?)
    }

    /// Mailboxes that have unread, unleased mail and an agent able to read it.
    pub fn mailboxes_with_mail(&self) -> Result<Vec<Mailbox>> {
        self.close_undeliverable()?;
        let mut out = Vec::new();
        let mut stmt = self.conn().prepare(&format!(
            "SELECT DISTINCT x.team, x.recipient FROM mail x JOIN members m ON m.team = x.team AND m.name = x.recipient
             JOIN teams t ON t.id = x.team
             WHERE x.delivered_at IS NULL AND x.lease IS NULL AND x.delivery IS NULL AND {RUNNABLE}"
        ))?;
        for r in stmt.query_map([], |r| Ok(Mailbox { team: r.get(0)?, recipient: r.get(1)? }))? {
            out.push(r?);
        }
        if !self.pending(None, ORCHESTRATOR)?.is_empty() {
            out.push(Mailbox { team: None, recipient: ORCHESTRATOR.into() });
        }
        Ok(out)
    }

    // --- live sessions: deliveries -------------------------------------------------

    /// Lease a live session's pending mail for one step boundary: most urgent first,
    /// within `budget` characters of rendered text (at least one message). `seen` are
    /// mail ids the session already holds (injected before a crash or a lost ack):
    /// they are settled as delivered instead of being offered again.
    pub fn lease_delivery(&self, team: Option<&str>, recipient: &str, seen: &[i64], budget: usize) -> Result<Option<Delivery>> {
        self.t.tx(|| {
            let at = now();
            for id in seen {
                self.conn().execute(
                    "UPDATE mail SET delivered_at = ?1 WHERE id = ?2 AND recipient = ?3 AND delivered_at IS NULL AND lease IS NULL",
                    params![at, id, recipient],
                )?;
            }
            let mut pending = self.pending(team, recipient)?;
            if pending.is_empty() {
                return Ok(None);
            }
            pending.sort_by_key(|m| (rank_mail(m), m.id));
            let mut taken = Vec::new();
            let mut used = 0;
            for m in &pending {
                let cost = render_one(m).chars().count() + 2;
                if !taken.is_empty() && used + cost > budget {
                    break;
                }
                used += cost;
                taken.push(m.clone());
            }
            let more = pending.len() - taken.len();
            let ids: Vec<i64> = taken.iter().map(|m| m.id).collect();
            self.conn().execute(
                "INSERT INTO deliveries(team, recipient, created, mail) VALUES (?1, ?2, ?3, ?4)",
                params![team, recipient, at, serde_json::to_string(&ids)?],
            )?;
            let id = self.conn().last_insert_rowid();
            for m in &ids {
                self.conn().execute("UPDATE mail SET delivery = ?1 WHERE id = ?2", params![id, m])?;
            }
            Ok(Some(Delivery { id, mails: taken, more }))
        })
    }

    /// The session holds the delivery (the model has seen it): its mail is delivered.
    /// Idempotent; `recipient` must own the delivery.
    pub fn ack_delivery(&self, id: i64, recipient: &str) -> Result<usize> {
        self.t.tx(|| {
            let owner: Option<String> =
                self.conn().query_row("SELECT recipient FROM deliveries WHERE id = ?1", [id], |r| r.get(0)).optional()?;
            match owner {
                None => return Err(GenieError::not_found(format!("delivery {id} not found"))),
                Some(o) if o != recipient => return Err(GenieError::Denied(format!("delivery {id} is not yours"))),
                _ => {}
            }
            let at = now();
            self.conn().execute("UPDATE deliveries SET acked_at = COALESCE(acked_at, ?1) WHERE id = ?2", params![at, id])?;
            Ok(self.conn().execute("UPDATE mail SET delivered_at = ?1 WHERE delivery = ?2 AND delivered_at IS NULL", params![at, id])?)
        })
    }

    /// The delivery never reached the model: its mail is offered again.
    pub fn release_delivery(&self, id: i64) -> Result<usize> {
        self.t.tx(|| {
            self.conn().execute("UPDATE deliveries SET released_at = ?1 WHERE id = ?2 AND acked_at IS NULL", params![now(), id])?;
            Ok(self.conn().execute("UPDATE mail SET delivery = NULL WHERE delivery = ?1 AND delivered_at IS NULL", [id])?)
        })
    }

    /// Open (neither acknowledged nor released) deliveries created before `before`.
    pub fn open_deliveries(&self, before: &str) -> Result<Vec<OpenDelivery>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, team, recipient, created FROM deliveries WHERE acked_at IS NULL AND released_at IS NULL AND created < ?1 ORDER BY id",
        )?;
        Ok(stmt
            .query_map([before], |r| Ok(OpenDelivery { id: r.get(0)?, team: r.get(1)?, recipient: r.get(2)?, created: r.get(3)? }))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Release every open delivery of one agent (its session ended).
    pub fn release_deliveries_of(&self, team: Option<&str>, recipient: &str) -> Result<usize> {
        let ids: Vec<i64> = {
            let mut stmt = self
                .conn()
                .prepare("SELECT id FROM deliveries WHERE acked_at IS NULL AND released_at IS NULL AND recipient = ?1 AND team IS ?2")?;
            stmt.query_map(params![recipient, team], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
        };
        let mut n = 0;
        for id in ids {
            n += self.release_delivery(id)?;
        }
        Ok(n)
    }

    /// Mailboxes holding an undelivered `interrupt` not yet handed to the session.
    pub fn interrupted_mailboxes(&self) -> Result<Vec<Mailbox>> {
        let mut stmt = self.conn().prepare(
            "SELECT DISTINCT CASE WHEN recipient = 'orchestrator' THEN NULL ELSE team END, recipient FROM mail
             WHERE level = 'interrupt' AND delivered_at IS NULL AND lease IS NULL AND delivery IS NULL",
        )?;
        Ok(stmt.query_map([], |r| Ok(Mailbox { team: r.get(0)?, recipient: r.get(1)? }))?.collect::<rusqlite::Result<_>>()?)
    }

    /// Delivery latency (seconds from sending to delivery) of mail delivered since `since`, by level.
    pub fn delivery_latencies(&self, since: &str) -> Result<Vec<(String, f64)>> {
        let mut stmt = self.conn().prepare(
            "SELECT level, (julianday(delivered_at) - julianday(at)) * 86400.0 FROM mail
             WHERE delivered_at IS NOT NULL AND delivered_at >= ?1 AND superseded_by IS NULL AND kind = 'message'",
        )?;
        Ok(stmt.query_map([since], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?)
    }

    // --- asks and replies ------------------------------------------------------------

    /// Answer `to_mail` (a message addressed to `from`): the reply goes back to its sender.
    pub fn reply(&self, from: &str, from_role: &str, to_mail: i64, text: &str) -> Result<Vec<Mail>> {
        let original = self.mail(to_mail).map_err(|_| GenieError::not_found(format!("message {to_mail} not found")))?;
        if original.to != from {
            return Err(GenieError::Denied(format!("message {to_mail} was not addressed to {from}")));
        }
        let Some(team) = original.team.clone() else {
            return Err(GenieError::invalid(format!("message {to_mail} has no team to answer in")));
        };
        self.send(SendMail {
            team: &team,
            from,
            from_role,
            to: &original.from,
            text,
            level: Some(if original.awaits { "high" } else { "normal" }),
            intent: (original.intent.as_deref() == Some("question")).then_some("verdict"),
            kind: "message",
            reply_to: Some(to_mail),
            ..Default::default()
        })
    }

    /// The first reply to an ask, taken by the waiting sender (marked delivered so it
    /// is not injected into its session as well). `None` while nobody has answered.
    pub fn take_reply(&self, ask: i64, asker: &str) -> Result<Option<Mail>> {
        self.t.tx(|| {
            let reply = self
                .conn()
                .query_row(
                    "SELECT * FROM mail WHERE reply_to = ?1 AND recipient = ?2 AND delivered_at IS NULL AND lease IS NULL AND delivery IS NULL
                     ORDER BY id LIMIT 1",
                    params![ask, asker],
                    Mail::from_row,
                )
                .optional()?;
            if let Some(r) = &reply {
                self.conn().execute("UPDATE mail SET delivered_at = ?1 WHERE id = ?2", params![now(), r.id])?;
            }
            Ok(reply)
        })
    }
}

// --- names -------------------------------------------------------------------

/// Default name pools per role; the id is the lowercase name used for mail.
pub fn name_pool(role: &str) -> &'static [&'static str] {
    match role {
        "analyst" => &["sherlock", "poirot", "marple", "columbo", "scully", "mulder", "watson", "clouseau"],
        "executor" => &["bender", "baymax", "walle", "optimus", "johnny5", "r2d2", "tars", "robocop"],
        "reviewer" => &["gandalf", "yoda", "hermione", "spock", "galadriel", "dumbledore", "morpheus", "picard"],
        "tester" => &["murphy", "gremlin", "loki", "jinx", "chaos", "moriarty"],
        "documenter" => &["tolkien", "homer", "shakespeare", "pushkin", "dickens", "chekhov"],
        _ => &["agent"],
    }
}

pub fn display_name(name: &str) -> String {
    match name {
        "walle" => "WALL-E".into(),
        "johnny5" => "Johnny 5".into(),
        "r2d2" => "R2-D2".into(),
        "tars" => "TARS".into(),
        "robocop" => "RoboCop".into(),
        _ => name
            .split(['-', '_'])
            .filter(|p| !p.is_empty())
            .map(|p| {
                let mut c = p.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// Pick a free name for a role; `taken` is extended.
pub fn pick_name(role: &str, taken: &mut std::collections::HashSet<String>) -> String {
    let pool: Vec<String> = name_pool(role).iter().map(|s| s.to_string()).collect();
    pick_from(&pool, taken)
}

/// Pick a free name from a pool (a configured role's names): a random free one,
/// else the first with a number. `taken` is extended.
pub fn pick_from(pool: &[String], taken: &mut std::collections::HashSet<String>) -> String {
    let free: Vec<&String> = pool.iter().filter(|n| !taken.contains(*n)).collect();
    let name = if free.is_empty() {
        let base = pool.first().map(String::as_str).unwrap_or("agent");
        (2..).map(|i| format!("{base}{i}")).find(|c| !taken.contains(c)).unwrap_or_default()
    } else {
        let mut b = [0u8; 2];
        getrandom::fill(&mut b).expect("OS random source");
        free[u16::from_le_bytes(b) as usize % free.len()].clone()
    };
    taken.insert(name.clone());
    name
}

// --- digest ------------------------------------------------------------------

fn rank_intent(intent: Option<&str>) -> u8 {
    match intent {
        Some("blocker" | "question") => 0,
        Some("verdict" | "done") => 1,
        Some("fyi") => 3,
        _ => 2,
    }
}

fn one_line(text: &str, limit: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > limit { format!("{}…", flat.chars().take(limit).collect::<String>()) } else { flat }
}

pub fn digest_line(m: &Mail) -> String {
    let intent = m.intent.as_deref().map(|i| format!(" · {i}")).unwrap_or_default();
    format!("- {} ({}) · {}{intent} · {}", m.from, m.from_role, m.level, one_line(&m.text, 240))
}

/// The orchestrator's batch of team messages as a digest: grouped by team, one
/// line per sender (its latest message), teams waiting for an answer first, FYI
/// last. Non-`message` rows are rendered verbatim by the caller.
pub fn render_digest(mails: &[Mail]) -> String {
    use std::collections::BTreeMap;
    let messages: Vec<&Mail> = mails.iter().filter(|m| m.kind == "message").collect();
    if messages.is_empty() {
        return String::new();
    }
    let mut latest: BTreeMap<(String, String), &Mail> = BTreeMap::new();
    for m in &messages {
        let key = (m.team.clone().unwrap_or_else(|| "(global)".into()), m.from.clone());
        if latest.get(&key).is_none_or(|p| p.id < m.id) {
            latest.insert(key, m);
        }
    }
    let mut fyi: Vec<&Mail> = Vec::new();
    let mut by_team: BTreeMap<String, Vec<&Mail>> = BTreeMap::new();
    for ((team, _), m) in latest {
        if m.intent.as_deref() == Some("fyi") {
            fyi.push(m);
        } else {
            by_team.entry(team).or_default().push(m);
        }
    }
    let mut sections: Vec<(String, Vec<&Mail>, u8, i64)> = by_team
        .into_iter()
        .map(|(team, mut lines)| {
            lines.sort_by_key(|m| m.id);
            let last = lines.iter().max_by_key(|m| m.id).expect("non-empty");
            let (rank, at) = (rank_intent(last.intent.as_deref()), last.id);
            (team, lines, rank, at)
        })
        .collect();
    sections.sort_by_key(|s| (s.2, s.3));
    fyi.sort_by_key(|m| m.id);
    let rows = sections.iter().map(|s| s.1.len()).sum::<usize>() + fyi.len();
    let mut shape = Vec::new();
    if !sections.is_empty() {
        shape.push(format!("{} team section{}", sections.len(), if sections.len() == 1 { "" } else { "s" }));
    }
    if !fyi.is_empty() {
        shape.push("FYI".to_string());
    }
    let mut out = vec![format!("[genie digest · {rows} message{} in {}]", if rows == 1 { "" } else { "s" }, shape.join(" + "))];
    for (team, lines, ..) in &sections {
        out.push(String::new());
        out.push(format!("## {team} ({})", lines.len()));
        out.extend(lines.iter().map(|m| digest_line(m)));
    }
    if !fyi.is_empty() {
        out.push(String::new());
        out.push("## FYI".into());
        out.extend(fyi.iter().map(|m| digest_line(m)));
    }
    out.join("\n")
}

// --- deliveries to live sessions ------------------------------------------------

/// Longest message body put into a session; the rest is read with `genie mail read <id>`.
pub const MAX_MESSAGE_CHARS: usize = 2000;
/// Default size budget of one delivery (rendered characters).
pub const DELIVERY_BUDGET: usize = 8000;

/// Delivery order: kickoff, interrupts, urgent and people, blockers and questions,
/// verdicts, the rest, FYI and low last; ties keep the sending order.
fn rank_mail(m: &Mail) -> (u8, u8) {
    let level = match (m.kind.as_str(), m.level.as_str()) {
        ("kickoff", _) => 0,
        (_, "interrupt") => 1,
        ("owner" | "system", _) | (_, "high") => 2,
        (_, "low") => 4,
        _ => 3,
    };
    (level, rank_intent(m.intent.as_deref()))
}

fn clip(text: &str, id: i64) -> String {
    let text = text.trim();
    if text.chars().count() <= MAX_MESSAGE_CHARS {
        return text.to_string();
    }
    let head: String = text.chars().take(MAX_MESSAGE_CHARS).collect();
    format!("{head}…\n[cut: {} more characters — read all with `genie mail read {id}`]", text.chars().count() - MAX_MESSAGE_CHARS)
}

/// One message as it appears in a session.
pub fn render_one(m: &Mail) -> String {
    let time = m.at.get(11..16).unwrap_or("");
    let head = match m.kind.as_str() {
        "kickoff" => format!("## Kickoff · team {}", m.team.as_deref().unwrap_or("")),
        "system" => format!("## System note from {} · {time}", m.from),
        "owner" => format!("## From the owner ({}) · {time}", m.from),
        _ => {
            let mut h = format!("## #{} from {} ({})", m.id, m.from, m.from_role);
            if m.level == "interrupt" {
                h.push_str(" · INTERRUPT — your previous step was stopped for this");
            } else if m.level != "normal" {
                h.push_str(&format!(" · {}", m.level));
            }
            if let Some(i) = &m.intent {
                h.push_str(&format!(" · {i}"));
            }
            if let Some(r) = m.reply_to {
                h.push_str(&format!(" · reply to #{r}"));
            }
            h.push_str(&format!(" · {time}"));
            h
        }
    };
    let mut out = format!("{head}\n\n{}", clip(&m.text, m.id));
    if m.awaits {
        out.push_str(&format!("\n\n→ {} is waiting for your answer: `genie mail reply {} \"…\"`", m.from, m.id));
    }
    out
}

/// A delivery as one message for a live session. The orchestrator gets the
/// specials verbatim and team mail as a digest; a member gets every message.
pub fn render_delivery(d: &Delivery, orchestrator: bool) -> String {
    let mut out = vec![format!(
        "[genie mail · {} new{}]",
        d.mails.len(),
        if d.more > 0 { format!(" · {} more at your next step", d.more) } else { String::new() }
    )];
    if orchestrator {
        let (messages, specials): (Vec<Mail>, Vec<Mail>) =
            d.mails.iter().cloned().partition(|m| m.kind == "message" && m.level != "interrupt" && !m.awaits && m.reply_to.is_none());
        out.extend(specials.iter().map(render_one));
        let digest = render_digest(&messages);
        if !digest.is_empty() {
            out.push(digest);
        }
        out.push("(Act where a decision, answer, unblock or acceptance is needed; informational updates need no reply.)".into());
    } else {
        out.extend(d.mails.iter().map(render_one));
        out.push("(Handle what needs action, then carry on. Reply only when needed — `genie mail send` or `genie mail reply <id>`; no acknowledgements.)".into());
    }
    out.join("\n\n")
}

/// A member's batch as one message: kickoffs and system notes verbatim, then mail.
pub fn render_batch(mails: &[Mail]) -> String {
    let mut out = Vec::new();
    for m in mails {
        let head = match m.kind.as_str() {
            "kickoff" => "## Kickoff".to_string(),
            "system" => format!("## System note ({})", m.from),
            "owner" => format!("## From the owner ({})", m.from),
            _ => format!(
                "## From {} ({}) · {}{}",
                m.from,
                m.from_role,
                m.level,
                m.intent.as_deref().map(|i| format!(" · {i}")).unwrap_or_default()
            ),
        };
        out.push(format!("{head}\n\n{}", m.text.trim()));
    }
    out.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Actor, Role};
    use crate::tracker::CreateInput;

    fn fresh() -> (tempfile::TempDir, Tracker) {
        let dir = tempfile::tempdir().unwrap();
        let t = Tracker::init(dir.path().join(".genie"), None, None).unwrap();
        t.create(&Actor::new("o", Role::Orchestrator), CreateInput { title: "x".into(), ..Default::default() }).unwrap();
        (dir, t)
    }

    fn team(t: &Tracker) {
        let m = |n: &str, r: &str| NewMember { name: n.into(), role: r.into(), ..Default::default() };
        t.bus()
            .create(
                "orchestrator",
                "orchestrator",
                NewTeam {
                    id: "G-1".into(),
                    task: "G-1".into(),
                    cwd: "/tmp".into(),
                    members: vec![m("sherlock", "analyst"), m("bender", "executor"), m("yoda", "reviewer")],
                    ..Default::default()
                },
            )
            .unwrap();
    }

    fn send<'a>(from: &'a str, to: &'a str, text: &'a str) -> SendMail<'a> {
        SendMail { team: "G-1", from, from_role: "analyst", to, text, kind: "message", ..Default::default() }
    }

    #[test]
    fn direct_and_broadcast_mail_with_leases() {
        let (_d, t) = fresh();
        team(&t);
        let bus = t.bus();
        bus.send(send("sherlock", "bender", "plan is ready")).unwrap();
        bus.send(send("bender", "all", "starting")).unwrap();
        assert!(bus.send(send("x", "nobody", "?")).unwrap_err().to_string().contains("no member \"nobody\""));
        assert!(bus.send(SendMail { level: Some("loud"), ..send("a", "bender", "x") }).is_err());

        let leased = bus.lease(Some("G-1"), "bender", 10).unwrap();
        assert_eq!(leased.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), vec!["plan is ready"]);
        assert!(bus.lease(Some("G-1"), "bender", 11).unwrap().is_empty(), "leased mail is not offered twice");
        assert_eq!(bus.release_lease(10).unwrap(), 1);
        let again = bus.lease(Some("G-1"), "bender", 12).unwrap();
        assert_eq!(again.len(), 1, "a failed turn gives the mail back");
        bus.complete_lease(12).unwrap();
        assert!(bus.pending(Some("G-1"), "bender").unwrap().is_empty());
        assert_eq!(bus.pending(None, ORCHESTRATOR).unwrap().len(), 1, "the orchestrator hears the broadcast");
        let boxes = bus.mailboxes_with_mail().unwrap();
        assert!(boxes.contains(&Mailbox { team: Some("G-1".into()), recipient: "yoda".into() }));
        assert!(boxes.contains(&Mailbox { team: None, recipient: ORCHESTRATOR.into() }));
        assert!(!boxes.iter().any(|b| b.recipient == "bender"));
    }

    #[test]
    fn deliveries_go_most_urgent_first_within_a_budget() {
        let (_d, t) = fresh();
        team(&t);
        let bus = t.bus();
        bus.send(SendMail { intent: Some("fyi"), level: Some("low"), ..send("sherlock", "bender", "fyi: notes updated") }).unwrap();
        bus.send(send("sherlock", "bender", "plan is ready")).unwrap();
        bus.send(SendMail { intent: Some("question"), ..send("yoda", "bender", "which API?") }).unwrap();
        bus.send(SendMail {
            from: ORCHESTRATOR,
            from_role: "orchestrator",
            level: Some("interrupt"),
            ..send("", "bender", "stop: wrong branch")
        })
        .unwrap();
        let d = bus.lease_delivery(Some("G-1"), "bender", &[], DELIVERY_BUDGET).unwrap().unwrap();
        let order: Vec<&str> = d.mails.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(order, vec!["stop: wrong branch", "which API?", "plan is ready", "fyi: notes updated"]);
        assert_eq!(d.more, 0);
        assert_eq!(d.mails[0].level, "interrupt", "an interrupt is not demoted by the legacy urgent flag");
        assert!(bus.pending(Some("G-1"), "bender").unwrap().is_empty(), "leased mail is not offered twice");
        assert!(bus.lease_delivery(Some("G-1"), "bender", &[], DELIVERY_BUDGET).unwrap().is_none());
        let text = render_delivery(&d, false);
        assert!(text.starts_with("[genie mail · 4 new]") && text.contains("INTERRUPT"), "{text}");

        // A tight budget leaves the rest for the next boundary (always at least one message).
        let long = "x".repeat(3000);
        for _ in 0..3 {
            bus.send(send("sherlock", "yoda", &long)).unwrap();
        }
        let first = bus.lease_delivery(Some("G-1"), "yoda", &[], 2500).unwrap().unwrap();
        assert_eq!((first.mails.len(), first.more), (1, 2));
        assert!(render_one(&first.mails[0]).contains("read all with `genie mail read"), "long messages are clipped");
        let rest = bus.lease_delivery(Some("G-1"), "yoda", &[], DELIVERY_BUDGET).unwrap().unwrap();
        assert_eq!(rest.mails.len(), 2);
    }

    #[test]
    fn deliveries_are_acknowledged_released_and_never_injected_twice() {
        let (_d, t) = fresh();
        team(&t);
        let bus = t.bus();
        bus.send(send("sherlock", "bender", "one")).unwrap();
        let d = bus.lease_delivery(Some("G-1"), "bender", &[], DELIVERY_BUDGET).unwrap().unwrap();
        assert!(bus.ack_delivery(d.id, "yoda").is_err(), "only the recipient acknowledges");
        assert_eq!(bus.release_delivery(d.id).unwrap(), 1);
        assert_eq!(bus.pending(Some("G-1"), "bender").unwrap().len(), 1, "a released delivery is offered again");

        let d = bus.lease_delivery(Some("G-1"), "bender", &[], DELIVERY_BUDGET).unwrap().unwrap();
        assert_eq!(bus.ack_delivery(d.id, "bender").unwrap(), 1);
        assert_eq!(bus.ack_delivery(d.id, "bender").unwrap(), 0, "acknowledging twice is harmless");
        assert!(bus.mail(d.mails[0].id).unwrap().delivered_at.is_some());

        // The session injected a delivery but its ack was lost; the delivery was released
        // as stale. The next lease names the id as seen: settled, not injected again.
        bus.send(send("sherlock", "bender", "two")).unwrap();
        let d = bus.lease_delivery(Some("G-1"), "bender", &[], DELIVERY_BUDGET).unwrap().unwrap();
        let stale = bus.open_deliveries("9999").unwrap();
        assert_eq!(stale.iter().map(|o| o.id).collect::<Vec<_>>(), vec![d.id]);
        bus.release_delivery(d.id).unwrap();
        assert!(bus.lease_delivery(Some("G-1"), "bender", &[d.mails[0].id], DELIVERY_BUDGET).unwrap().is_none());
        assert!(bus.mail(d.mails[0].id).unwrap().delivered_at.is_some());

        // A restart releases every open delivery.
        bus.send(send("sherlock", "yoda", "three")).unwrap();
        bus.lease_delivery(Some("G-1"), "yoda", &[], DELIVERY_BUDGET).unwrap().unwrap();
        assert!(bus.pending(Some("G-1"), "yoda").unwrap().is_empty());
        assert_eq!(bus.release_all_leases().unwrap(), 1);
        assert_eq!(bus.pending(Some("G-1"), "yoda").unwrap().len(), 1);
        assert!(bus.mailboxes_with_mail().unwrap().contains(&Mailbox { team: Some("G-1".into()), recipient: "yoda".into() }));
    }

    #[test]
    fn topics_supersede_and_asks_get_replies() {
        let (_d, t) = fresh();
        team(&t);
        let bus = t.bus();
        bus.send(SendMail { topic: Some("build"), ..send("bender", "yoda", "build 30%") }).unwrap();
        bus.send(SendMail { topic: Some("build"), ..send("bender", "yoda", "build 60%") }).unwrap();
        bus.send(SendMail { topic: Some("build"), ..send("sherlock", "yoda", "my build 10%") }).unwrap();
        let pending: Vec<String> = bus.pending(Some("G-1"), "yoda").unwrap().into_iter().map(|m| m.text).collect();
        assert_eq!(pending, vec!["build 60%", "my build 10%"], "a newer message on a topic replaces the sender's older one");

        let ask =
            bus.send(SendMail { intent: Some("question"), awaits: true, ..send("bender", "sherlock", "CSV or XLSX?") }).unwrap().remove(0);
        assert!(render_one(&ask).contains(&format!("genie mail reply {}", ask.id)));
        assert!(bus.take_reply(ask.id, "bender").unwrap().is_none());
        assert!(bus.reply("yoda", "reviewer", ask.id, "not mine").is_err(), "only the addressee answers");
        let reply = bus.reply("sherlock", "analyst", ask.id, "CSV").unwrap().remove(0);
        assert_eq!((reply.to.as_str(), reply.reply_to, reply.level.as_str()), ("bender", Some(ask.id), "high"));
        let taken = bus.take_reply(ask.id, "bender").unwrap().unwrap();
        assert_eq!(taken.text, "CSV");
        assert!(bus.pending(Some("G-1"), "bender").unwrap().is_empty(), "the waiting sender took it: not injected again");
        assert!(bus.take_reply(ask.id, "bender").unwrap().is_none());
    }

    #[test]
    fn interrupts_are_found_and_orchestrator_deliveries_digest_the_rest() {
        let (_d, t) = fresh();
        team(&t);
        let bus = t.bus();
        bus.send(SendMail { level: Some("interrupt"), from: ORCHESTRATOR, from_role: "orchestrator", ..send("", "yoda", "stop") }).unwrap();
        assert_eq!(bus.interrupted_mailboxes().unwrap(), vec![Mailbox { team: Some("G-1".into()), recipient: "yoda".into() }]);
        bus.send(SendMail { intent: Some("done"), ..send("bender", "orchestrator", "implemented") }).unwrap();
        bus.send(SendMail { intent: Some("question"), awaits: true, ..send("sherlock", "orchestrator", "scope?") }).unwrap();
        let d = bus.lease_delivery(None, ORCHESTRATOR, &[], DELIVERY_BUDGET).unwrap().unwrap();
        let text = render_delivery(&d, true);
        assert!(text.contains("[genie digest") && text.contains("implemented"), "{text}");
        assert!(text.contains("is waiting for your answer"), "an ask stays verbatim with its reply hint:\n{text}");
    }

    #[test]
    fn stopped_teams_are_silenced_and_removed_members_lose_their_mail() {
        let (_d, t) = fresh();
        team(&t);
        let bus = t.bus();
        bus.send(send("bender", "yoda", "review please")).unwrap();
        bus.remove_member("G-1", "yoda").unwrap();
        assert!(bus.pending(Some("G-1"), "yoda").unwrap().is_empty());
        bus.set_state("G-1", TeamState::Stopped, Some("task_closed"), "genie").unwrap();
        bus.send(send("bender", "orchestrator", "late")).unwrap();
        assert!(bus.pending(None, ORCHESTRATOR).unwrap().is_empty(), "a team stopped on purpose is silenced");
        assert!(bus.mailboxes_with_mail().unwrap().is_empty());
        let kinds: Vec<String> = t.events_after(0, 100).unwrap().into_iter().map(|e| e.kind).collect();
        assert!(kinds.contains(&"team.spawned".to_string()) && kinds.contains(&"team.stopped".to_string()));
    }

    #[test]
    fn digest_groups_by_team_and_puts_questions_first() {
        let mail = |id: i64, team: &str, from: &str, intent: Option<&str>, text: &str| Mail {
            id,
            at: String::new(),
            team: Some(team.into()),
            from: from.into(),
            from_role: "executor".into(),
            to: ORCHESTRATOR.into(),
            text: text.into(),
            urgent: false,
            level: "normal".into(),
            intent: intent.map(str::to_string),
            kind: "message".into(),
            ..Default::default()
        };
        let d = render_digest(&[
            mail(1, "G-1", "bender", Some("done"), "old"),
            mail(2, "G-2", "baymax", Some("question"), "which API?"),
            mail(3, "G-1", "bender", Some("verdict"), "approved"),
            mail(4, "G-3", "walle", Some("fyi"), "note"),
        ]);
        assert!(d.starts_with("[genie digest · 3 messages in 2 team sections + FYI]"));
        let g2 = d.find("## G-2").unwrap();
        let g1 = d.find("## G-1").unwrap();
        assert!(g2 < g1, "the team waiting for an answer comes first:\n{d}");
        assert!(d.contains("approved") && !d.contains("old"));
    }

    #[test]
    fn names_are_unique_and_displayable() {
        let mut taken: std::collections::HashSet<String> = ["sherlock".to_string()].into();
        let a = pick_name("analyst", &mut taken);
        assert_ne!(a, "sherlock");
        assert_eq!(display_name("walle"), "WALL-E");
        assert_eq!(display_name("big-bird"), "Big Bird");
        let mut all: std::collections::HashSet<String> = name_pool("tester").iter().map(|s| s.to_string()).collect();
        assert_eq!(pick_name("tester", &mut all), "murphy2");
    }

    /// Move every sign of life two hours into the past: the team has been quiet since then.
    fn aged(t: &Tracker) {
        let old = (Utc::now() - chrono::Duration::hours(2)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        t.conn().execute("UPDATE teams SET created = ?1, updated = ?1", [&old]).unwrap();
        t.conn().execute("UPDATE members SET activity_at = NULL, heartbeat_at = NULL", []).unwrap();
    }

    #[test]
    fn a_quiet_team_is_found_and_a_working_one_is_not() {
        let (_d, t) = fresh();
        team(&t);
        let bus = t.bus();
        assert!(bus.quiet_teams(Duration::from_secs(3600)).unwrap().is_empty(), "a team just created is not silent yet");
        aged(&t);
        let quiet = bus.quiet_teams(Duration::from_secs(3600)).unwrap();
        assert_eq!(quiet.len(), 1);
        assert_eq!((quiet[0].id.as_str(), quiet[0].task.as_str(), quiet[0].members), ("G-1", "G-1", 3));
        assert!(quiet[0].idle_secs >= 7200, "{}", quiet[0].idle_secs);

        bus.member_working("G-1", "bender", json!({ "kind": "turn" })).unwrap();
        assert!(bus.quiet_teams(Duration::from_secs(3600)).unwrap().is_empty(), "a member at work is not silence");

        bus.member_gave_up("G-1", "bender", "model error", 3).unwrap();
        assert!(bus.quiet_teams(Duration::from_secs(3600)).unwrap().is_empty(), "a member that gave up is G-80's news, not silence");

        bus.member_restarted("G-1", "bender").unwrap();
        bus.set_paused("G-1", "bender", true, "owner").unwrap();
        assert!(bus.quiet_teams(Duration::from_secs(3600)).unwrap().is_empty(), "a member held by a person is not silence");
    }

    #[test]
    fn a_member_that_gave_up_stays_in_error_until_restarted() {
        let (_d, t) = fresh();
        team(&t);
        let bus = t.bus();
        let bender = |bus: &Bus| bus.get("G-1").unwrap().members.into_iter().find(|m| m.name == "bender").unwrap();
        assert!(bus.runnable("G-1", "bender").unwrap());

        bus.member_working("G-1", "bender", json!({ "kind": "session" })).unwrap();
        bus.member_gave_up("G-1", "bender", "model error: overloaded", 3).unwrap();
        let m = bender(&bus);
        assert_eq!((m.state, m.activity, m.status.as_str()), (MemberState::Error, Activity::Error, "stopped: model error: overloaded"));
        assert!(!bus.runnable("G-1", "bender").unwrap());

        // Its session ends afterwards — planned or not: it does not come back by itself.
        bus.member_idle("G-1", "bender").unwrap();
        assert_eq!(bender(&bus).state, MemberState::Error);

        bus.member_restarted("G-1", "bender").unwrap();
        let m = bender(&bus);
        assert_eq!((m.state, m.activity), (MemberState::Active, Activity::Idle));
        assert!(bus.runnable("G-1", "bender").unwrap());
    }

    #[test]
    fn a_task_has_an_active_team_until_the_team_stops() {
        let (_d, t) = fresh();
        team(&t);
        let bus = t.bus();
        assert_eq!(bus.active_team_of("G-1").unwrap(), None, "a team on the task is not the task's team until assigned");
        t.assign_team(&Actor::new("genie", Role::Orchestrator), "G-1", Some("G-1"), None, None).unwrap();
        assert_eq!(bus.active_team_of("G-1").unwrap().as_deref(), Some("G-1"));
        bus.set_state("G-1", TeamState::Stopped, Some("owner"), "anna").unwrap();
        assert_eq!(bus.active_team_of("G-1").unwrap(), None);
        assert!(!bus.runnable("G-1", "bender").unwrap(), "a member of a stopped team does not run");
    }

    #[test]
    fn mail_revives_a_team_and_undelivered_mail_is_not_silence() {
        let (_d, t) = fresh();
        team(&t);
        aged(&t);
        let bus = t.bus();
        assert_eq!(bus.quiet_teams(Duration::from_secs(3600)).unwrap().len(), 1);

        bus.send(send("sherlock", "bender", "hello")).unwrap();
        assert!(bus.quiet_teams(Duration::from_secs(0)).unwrap().is_empty(), "mail the session has not picked up is not silence");
        bus.lease(Some("G-1"), "bender", 1).unwrap();
        bus.complete_lease(1).unwrap();
        assert!(bus.quiet_teams(Duration::from_secs(3600)).unwrap().is_empty(), "a delivered letter is a fresh sign of life");
    }

    #[test]
    fn the_journal_marker_is_found_by_event() {
        let (_d, t) = fresh();
        team(&t);
        let bus = t.bus();
        assert!(bus.last_event_at("G-1", "team_silent").unwrap().is_none(), "nothing written yet");
        bus.log("G-1", "team_silent", json!({ "idleSecs": 900 })).unwrap();
        let at = bus.last_event_at("G-1", "team_silent").unwrap().unwrap();
        assert!(DateTime::parse_from_rfc3339(&at).is_ok(), "{at}");
        assert!(bus.last_event_at("G-1", "team_created").unwrap().is_some());
        assert!(bus.last_event_at("G-9", "team_silent").unwrap().is_none(), "another team's journal is not the marker");
    }
}
