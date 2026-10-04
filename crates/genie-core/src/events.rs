//! Event journal (outbox): every change appends a row in the same transaction as
//! the change itself, so a committed change always has its event and a rolled-back
//! one never does.
//!
//! Subscribers (web SSE, automations, the orchestrator, channels) read events after
//! their own cursor and acknowledge them. Delivery is at-least-once: a subscriber
//! that crashes between handling and `ack` sees the event again, so handlers are
//! idempotent by event id. A subscriber that writes to this database can `ack`
//! inside its own transaction and get exactly-once processing.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;

use crate::db::now;
use crate::error::Result;

pub const TASK_CREATED: &str = "task.created";
pub const TASK_UPDATED: &str = "task.updated";
pub const TASK_STATUS_CHANGED: &str = "task.status_changed";
pub const TASK_COMMENTED: &str = "task.commented";
pub const TASK_CRITERION_CHECKED: &str = "task.criterion_checked";
pub const TASK_ARTIFACT_ADDED: &str = "task.artifact_added";
pub const TASK_BLOCKED: &str = "task.blocked";
pub const TASK_UNBLOCKED: &str = "task.unblocked";
pub const TASK_TEAM_ASSIGNED: &str = "task.team_assigned";
/// A task was deleted for good (its earlier events stay in the journal).
pub const TASK_DELETED: &str = "task.deleted";
pub const MAIL_SENT: &str = "mail.sent";
/// A tool call through the MCP gateway.
pub const MCP_CALLED: &str = "mcp.called";
/// An agent pushed to a repository through the repository proxy.
pub const GIT_PUSHED: &str = "git.pushed";
/// The proxy refused an agent's push or clone (policy).
pub const GIT_DENIED: &str = "git.denied";
/// A pull/merge request was opened for a task's branch.
pub const CR_OPENED: &str = "cr.opened";
/// A pull/merge request was merged.
pub const CR_MERGED: &str = "cr.merged";
/// A pull/merge request was closed without merging.
pub const CR_CLOSED: &str = "cr.closed";
/// The CI of a task's request failed / passed.
pub const CI_FAILED: &str = "ci.failed";
pub const CI_PASSED: &str = "ci.passed";
/// The checks of a watched ref stayed `pending` longer than `runtime.ciPendingSecs`.
pub const CI_STALLED: &str = "ci.stalled";
/// The failed checks of a watched ref were rerun on the host (`genie pr rerun`).
pub const CI_RERUN: &str = "ci.rerun";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: i64,
    pub at: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub actor: String,
    pub actor_role: String,
    pub payload: Value,
}

/// Append an event. Call it inside the transaction that makes the change.
pub fn append(conn: &Connection, kind: &str, subject: Option<&str>, actor: &str, actor_role: &str, payload: Value) -> Result<i64> {
    conn.execute(
        "INSERT INTO events(at, type, subject, actor, actor_role, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![now(), kind, subject, actor, actor_role, payload.to_string()],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Events with id greater than `after`, oldest first, at most `limit`.
pub fn after(conn: &Connection, after: i64, limit: usize) -> Result<Vec<Event>> {
    let mut stmt =
        conn.prepare_cached("SELECT id, at, type, subject, actor, actor_role, payload FROM events WHERE id > ?1 ORDER BY id LIMIT ?2")?;
    let rows = stmt.query_map(params![after, limit as i64], |r| {
        let payload: String = r.get(6)?;
        Ok(Event {
            id: r.get(0)?,
            at: r.get(1)?,
            kind: r.get(2)?,
            subject: r.get(3)?,
            actor: r.get(4)?,
            actor_role: r.get(5)?,
            payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The newest events of one type, newest first, at most `limit`.
pub fn latest_of(conn: &Connection, kind: &str, limit: usize) -> Result<Vec<Event>> {
    let mut stmt = conn
        .prepare_cached("SELECT id, at, type, subject, actor, actor_role, payload FROM events WHERE type = ?1 ORDER BY id DESC LIMIT ?2")?;
    let rows = stmt.query_map(params![kind, limit as i64], |r| {
        let payload: String = r.get(6)?;
        Ok(Event {
            id: r.get(0)?,
            at: r.get(1)?,
            kind: r.get(2)?,
            subject: r.get(3)?,
            actor: r.get(4)?,
            actor_role: r.get(5)?,
            payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Id of the newest event, 0 when the journal is empty.
pub fn last_id(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT COALESCE(MAX(id), 0) FROM events", [], |r| r.get(0))?)
}

/// Last acknowledged event id of a subscriber, 0 for a new subscriber.
pub fn cursor(conn: &Connection, subscriber: &str) -> Result<i64> {
    Ok(conn.query_row("SELECT last_id FROM event_cursors WHERE subscriber = ?1", [subscriber], |r| r.get(0)).optional()?.unwrap_or(0))
}

/// Move a subscriber's cursor forward. Never moves it back, so a late or
/// duplicate acknowledgement cannot make a subscriber see events twice.
pub fn ack(conn: &Connection, subscriber: &str, id: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO event_cursors(subscriber, last_id, updated) VALUES (?1, ?2, ?3)
         ON CONFLICT(subscriber) DO UPDATE SET last_id = MAX(last_id, excluded.last_id), updated = excluded.updated",
        params![subscriber, id, now()],
    )?;
    Ok(())
}
