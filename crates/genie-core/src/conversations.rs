//! Durable incoming conversations and agent responses, independent of a transport or process.
//!
//! A response is one activation, not a task's lifetime. Conversation ownership and a single
//! active response make continuation deterministic. Explicit idempotency keys are scoped to
//! the principal in this project. The application checks access before reads and mutations,
//! and checks a proposal's version against the task before applying its decision in one
//! enclosing tracker transaction. Private text stays out of the project-wide event journal.

use rusqlite::types::Type;
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{GenieError, Result, Tracker};

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS conversations (
  id TEXT PRIMARY KEY,
  principal TEXT NOT NULL,
  initiator TEXT,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS conversation_responses (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  id TEXT NOT NULL UNIQUE,
  conversation_id TEXT NOT NULL REFERENCES conversations(id),
  principal TEXT NOT NULL,
  previous_response_id TEXT REFERENCES conversation_responses(id),
  input TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  idempotency_key TEXT,
  status TEXT NOT NULL CHECK(status IN ('queued', 'in_progress', 'completed', 'failed', 'cancelled')),
  output TEXT NOT NULL DEFAULT '',
  error TEXT,
  created_at INTEGER NOT NULL,
  completed_at INTEGER,
  UNIQUE(principal, idempotency_key)
);
CREATE INDEX IF NOT EXISTS conversation_responses_history ON conversation_responses(conversation_id, seq);
CREATE INDEX IF NOT EXISTS conversation_responses_queue ON conversation_responses(status, seq);
CREATE UNIQUE INDEX IF NOT EXISTS conversation_one_active_response ON conversation_responses(conversation_id)
  WHERE status IN ('queued', 'in_progress');
CREATE TABLE IF NOT EXISTS conversation_tasks (
  task TEXT PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
  conversation_id TEXT NOT NULL REFERENCES conversations(id)
);
CREATE INDEX IF NOT EXISTS conversation_tasks_conversation ON conversation_tasks(conversation_id);
CREATE TABLE IF NOT EXISTS conversation_proposals (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  id TEXT NOT NULL UNIQUE,
  conversation_id TEXT NOT NULL REFERENCES conversations(id),
  task TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  action TEXT NOT NULL CHECK(action IN ('dispatch', 'accept')),
  version TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending', 'approved', 'rejected')),
  created_at INTEGER NOT NULL,
  decided_at INTEGER,
  UNIQUE(conversation_id, task, action, version)
);
CREATE INDEX IF NOT EXISTS conversation_proposals_conversation ON conversation_proposals(conversation_id, seq);
"#;

/// Bounds apply to every submission, including callers that do not use HTTP.
pub const MAX_INPUT_MESSAGES: usize = 200;
pub const MAX_INPUT_BYTES: usize = 512 * 1024;
pub const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
pub const MAX_IDEMPOTENCY_KEY_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct Submission {
    pub principal: String,
    pub initiator: Option<String>,
    pub conversation_id: Option<String>,
    pub previous_response_id: Option<String>,
    pub input: Vec<Message>,
    pub idempotency_key: Option<String>,
    /// The adapter's canonical request fingerprint, including the continuation and protocol.
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub principal: String,
    pub initiator: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    Queued,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

impl ResponseStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub id: String,
    pub conversation_id: String,
    pub principal: String,
    pub initiator: Option<String>,
    pub previous_response_id: Option<String>,
    pub input: Vec<Message>,
    pub status: ResponseStatus,
    pub output: String,
    pub error: Option<String>,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalAction {
    Dispatch,
    Accept,
}

impl ProposalAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dispatch => "dispatch",
            Self::Accept => "accept",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Pending,
    Approved,
    Rejected,
}

impl ProposalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub conversation_id: String,
    pub task: String,
    pub action: ProposalAction,
    /// An opaque snapshot or digest checked by the application's decision operation.
    pub version: String,
    pub status: ProposalStatus,
    pub created_at: i64,
    pub decided_at: Option<i64>,
}

pub struct Conversations<'a>(&'a Tracker);

impl Tracker {
    pub fn conversations(&self) -> Conversations<'_> {
        Conversations(self)
    }
}

const RESPONSE_SELECT: &str = "SELECT r.id, r.conversation_id, r.principal, c.initiator, r.previous_response_id, r.input,
    r.status, r.output, r.error, r.created_at, r.completed_at
    FROM conversation_responses r JOIN conversations c ON c.id = r.conversation_id";
const PROPOSAL_SELECT: &str = "SELECT id, conversation_id, task, action, version, status, created_at, decided_at
    FROM conversation_proposals";

impl Conversations<'_> {
    /// Commit admission and its event together. A retry is checked before the conversation's
    /// active-response guard, so the same accepted request is returned even while it is running.
    pub fn submit(&self, request: Submission) -> Result<Response> {
        validate(&request)?;
        self.0.tx(|| {
            if let Some(key) = &request.idempotency_key {
                let existing = self
                    .0
                    .conn()
                    .query_row(
                        "SELECT id, fingerprint FROM conversation_responses WHERE principal = ?1 AND idempotency_key = ?2",
                        params![request.principal, key],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                    )
                    .optional()?;
                if let Some((id, fingerprint)) = existing {
                    if fingerprint != request.fingerprint {
                        return Err(GenieError::invalid("idempotency key already belongs to a different request"));
                    }
                    return self.get(&id);
                }
            }
            let previous = request.previous_response_id.as_deref().map(|id| self.get(id)).transpose()?;
            if let Some(previous) = &previous {
                if previous.principal != request.principal {
                    return Err(GenieError::Denied("the previous response belongs to another principal".into()));
                }
                if !previous.status.is_terminal() {
                    return Err(GenieError::invalid("the previous response is still active"));
                }
                if request.conversation_id.as_ref().is_some_and(|id| id != &previous.conversation_id) {
                    return Err(GenieError::invalid("conversation and previous response do not match"));
                }
            }
            let conversation_id = request.conversation_id.as_deref().or_else(|| previous.as_ref().map(|r| r.conversation_id.as_str()));
            let conversation = match conversation_id {
                Some(id) => {
                    let conversation = self.conversation(id)?;
                    if conversation.principal != request.principal {
                        return Err(GenieError::Denied("the conversation belongs to another principal".into()));
                    }
                    // The stable principal owns the conversation. The login is
                    // historical display metadata and can change on the account.
                    conversation
                }
                None => {
                    let id = self.id("conv")?;
                    self.0.conn().execute(
                        "INSERT INTO conversations(id, principal, initiator, created_at) VALUES (?1, ?2, ?3, ?4)",
                        params![id, request.principal, request.initiator, timestamp()],
                    )?;
                    self.event("conversation.created", json!({"conversation_id": id}))?;
                    self.conversation(&id)?
                }
            };
            let latest = self
                .0
                .conn()
                .query_row(
                    "SELECT id, status FROM conversation_responses WHERE conversation_id = ?1 ORDER BY seq DESC LIMIT 1",
                    [&conversation.id],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )
                .optional()?;
            if let Some((latest, status)) = &latest {
                if matches!(status.as_str(), "queued" | "in_progress") {
                    return Err(GenieError::invalid("the conversation already has an active response"));
                }
                if previous.as_ref().is_some_and(|previous| &previous.id != latest) {
                    return Err(GenieError::invalid("previous_response_id must name the latest response in the conversation"));
                }
            }
            let previous_response_id = latest.map(|(id, _)| id);
            let id = self.id("resp")?;
            self.0.conn().execute(
                "INSERT INTO conversation_responses(id, conversation_id, principal, previous_response_id, input,
                    fingerprint, idempotency_key, status, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'queued', ?8)",
                params![
                    id,
                    conversation.id,
                    request.principal,
                    previous_response_id,
                    serde_json::to_string(&request.input)?,
                    request.fingerprint,
                    request.idempotency_key,
                    timestamp()
                ],
            )?;
            let response = self.get(&id)?;
            self.response_event(&response)?;
            Ok(response)
        })
    }

    pub fn get(&self, id: &str) -> Result<Response> {
        self.0
            .conn()
            .query_row(&format!("{RESPONSE_SELECT} WHERE r.id = ?1"), [id], response)
            .optional()?
            .ok_or_else(|| GenieError::not_found("response not found"))
    }

    pub fn conversation(&self, id: &str) -> Result<Conversation> {
        self.0
            .conn()
            .query_row("SELECT id, principal, initiator, created_at FROM conversations WHERE id = ?1", [id], |r| {
                Ok(Conversation { id: r.get(0)?, principal: r.get(1)?, initiator: r.get(2)?, created_at: r.get(3)? })
            })
            .optional()?
            .ok_or_else(|| GenieError::not_found("conversation not found"))
    }

    /// In accepted order, including the active response if there is one.
    pub fn history(&self, conversation: &str) -> Result<Vec<Response>> {
        self.conversation(conversation)?;
        let mut q = self.0.conn().prepare(&format!("{RESPONSE_SELECT} WHERE r.conversation_id = ?1 ORDER BY r.seq"))?;
        Ok(q.query_map([conversation], response)?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn queued(&self) -> Result<Vec<Response>> {
        let mut q = self.0.conn().prepare(&format!("{RESPONSE_SELECT} WHERE r.status = 'queued' ORDER BY r.seq"))?;
        Ok(q.query_map([], response)?.collect::<rusqlite::Result<_>>()?)
    }

    /// A process takes only an unclaimed response; a second scheduler cannot also take it.
    pub fn claim(&self, id: &str) -> Result<bool> {
        self.0.tx(|| {
            let r = self.get(id)?;
            if r.status != ResponseStatus::Queued {
                return Ok(false);
            }
            self.0.conn().execute("UPDATE conversation_responses SET status = 'in_progress' WHERE id = ?1", [id])?;
            self.response_event(&self.get(id)?)?;
            Ok(true)
        })
    }

    /// An adapter may finish a deterministic command while it is still queued. An agent
    /// completes its claimed response. Neither may replace a cancelled or different result.
    pub fn complete(&self, id: &str, text: &str) -> Result<Response> {
        if text.len() > MAX_OUTPUT_BYTES {
            return Err(GenieError::invalid(format!("response output exceeds {MAX_OUTPUT_BYTES} bytes")));
        }
        self.finish(id, ResponseStatus::Completed, text, None)
    }

    pub fn fail(&self, id: &str, error: &str) -> Result<Response> {
        if error.len() > MAX_OUTPUT_BYTES {
            return Err(GenieError::invalid("response error is too long"));
        }
        self.finish(id, ResponseStatus::Failed, "", Some(error))
    }

    /// Cancellation is idempotent and leaves an already terminal response unchanged.
    pub fn cancel(&self, id: &str) -> Result<Response> {
        self.0.tx(|| {
            let current = self.get(id)?;
            if current.status.is_terminal() {
                return Ok(current);
            }
            self.finish(id, ResponseStatus::Cancelled, "", None)
        })
    }

    /// Called after obsolete workers have been stopped on server startup. The same response
    /// is retried, retaining its input, links and idempotency receipt; no new task is implied.
    pub fn recover(&self) -> Result<usize> {
        self.0.tx(|| {
            let mut q = self.0.conn().prepare("SELECT id FROM conversation_responses WHERE status = 'in_progress' ORDER BY seq")?;
            let ids = q.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            for id in &ids {
                self.0.conn().execute("UPDATE conversation_responses SET status = 'queued' WHERE id = ?1", [id])?;
                self.response_event(&self.get(id)?)?;
            }
            Ok(ids.len())
        })
    }

    /// A task has one originating conversation. Repeating its association is harmless;
    /// associating an existing task with a different private conversation is refused.
    pub fn link_task(&self, conversation: &str, task: &str) -> Result<()> {
        self.0.tx(|| {
            self.conversation(conversation)?;
            let task = self.0.get(task)?.id;
            if let Some(existing) = self.conversation_for_task(&task)? {
                return if existing == conversation {
                    Ok(())
                } else {
                    Err(GenieError::invalid("task is already linked to another conversation"))
                };
            }
            self.0.conn().execute("INSERT INTO conversation_tasks(task, conversation_id) VALUES (?1, ?2)", params![task, conversation])?;
            self.event("conversation.task_linked", json!({"conversation_id": conversation, "task": task}))
        })
    }

    pub fn tasks(&self, conversation: &str) -> Result<Vec<String>> {
        self.conversation(conversation)?;
        let mut q = self.0.conn().prepare("SELECT task FROM conversation_tasks WHERE conversation_id = ?1 ORDER BY task")?;
        Ok(q.query_map([conversation], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn conversation_for_task(&self, task: &str) -> Result<Option<String>> {
        let task = self.0.normalize_id(task)?;
        Ok(self.0.conn().query_row("SELECT conversation_id FROM conversation_tasks WHERE task = ?1", [task], |r| r.get(0)).optional()?)
    }

    pub fn propose(&self, conversation: &str, task: &str, action: ProposalAction, version: &str) -> Result<Proposal> {
        if version.is_empty() || version.len() > MAX_OUTPUT_BYTES {
            return Err(GenieError::invalid("a proposal needs a bounded non-empty version"));
        }
        self.0.tx(|| {
            self.conversation(conversation)?;
            let task = self.0.get(task)?.id;
            if self.conversation_for_task(&task)?.as_deref() != Some(conversation) {
                return Err(GenieError::invalid("proposal task is not linked to this conversation"));
            }
            let existing = self.0.conn().query_row(
                "SELECT id FROM conversation_proposals WHERE conversation_id = ?1 AND task = ?2 AND action = ?3 AND version = ?4",
                params![conversation, task, action.as_str(), version], |r| r.get::<_, String>(0),
            ).optional()?;
            if let Some(id) = existing {
                return self.proposal(&id);
            }
            let id = self.id("proposal")?;
            self.0.conn().execute(
                "INSERT INTO conversation_proposals(id, conversation_id, task, action, version, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, conversation, task, action.as_str(), version, timestamp()],
            )?;
            self.event("conversation.proposal_created", json!({"conversation_id": conversation, "proposal_id": id, "task": task}))?;
            self.proposal(&id)
        })
    }

    pub fn proposal(&self, id: &str) -> Result<Proposal> {
        self.0
            .conn()
            .query_row(&format!("{PROPOSAL_SELECT} WHERE id = ?1"), [id], proposal)
            .optional()?
            .ok_or_else(|| GenieError::not_found("proposal not found"))
    }

    pub fn proposals(&self, conversation: &str) -> Result<Vec<Proposal>> {
        self.conversation(conversation)?;
        let mut q = self.0.conn().prepare(&format!("{PROPOSAL_SELECT} WHERE conversation_id = ?1 ORDER BY seq"))?;
        Ok(q.query_map([conversation], proposal)?.collect::<rusqlite::Result<_>>()?)
    }

    /// Persist the decision in the same outer transaction as the application's checked
    /// action. Repeating the same decision returns its receipt; reversing it is a conflict.
    pub fn decide(&self, id: &str, approved: bool) -> Result<Proposal> {
        self.0.tx(|| {
            let current = self.proposal(id)?;
            let status = if approved { ProposalStatus::Approved } else { ProposalStatus::Rejected };
            if current.status == status {
                return Ok(current);
            }
            if current.status != ProposalStatus::Pending {
                return Err(GenieError::invalid("proposal already has a different decision"));
            }
            self.0.conn().execute(
                "UPDATE conversation_proposals SET status = ?1, decided_at = ?2 WHERE id = ?3",
                params![status.as_str(), timestamp(), id],
            )?;
            self.event(
                "conversation.proposal_decided",
                json!({"conversation_id": current.conversation_id,
                "proposal_id": id, "task": current.task, "status": status}),
            )?;
            self.proposal(id)
        })
    }

    fn finish(&self, id: &str, status: ResponseStatus, output: &str, error: Option<&str>) -> Result<Response> {
        self.0.tx(|| {
            let current = self.get(id)?;
            if current.status == status && current.output == output && current.error.as_deref() == error {
                return Ok(current);
            }
            if current.status.is_terminal() {
                return Err(GenieError::invalid("response is already terminal"));
            }
            self.0.conn().execute(
                "UPDATE conversation_responses SET status = ?1, output = ?2, error = ?3, completed_at = ?4 WHERE id = ?5",
                params![status.as_str(), output, error, timestamp(), id],
            )?;
            let response = self.get(id)?;
            self.response_event(&response)?;
            Ok(response)
        })
    }

    fn id(&self, prefix: &str) -> Result<String> {
        let random: String = self.0.conn().query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?;
        Ok(format!("{prefix}_{random}"))
    }

    fn response_event(&self, response: &Response) -> Result<()> {
        self.event(
            "conversation.response_changed",
            json!({"conversation_id": response.conversation_id,
            "response_id": response.id, "status": response.status}),
        )
    }

    fn event(&self, kind: &str, payload: serde_json::Value) -> Result<()> {
        self.0.append_event(kind, None, "genie", "orchestrator", payload).map(|_| ())
    }
}

fn timestamp() -> i64 {
    chrono::Utc::now().timestamp()
}

fn validate(request: &Submission) -> Result<()> {
    if request.principal.trim().is_empty() || request.fingerprint.is_empty() {
        return Err(GenieError::invalid("a submission needs a principal and request fingerprint"));
    }
    if request.input.is_empty() || request.input.len() > MAX_INPUT_MESSAGES {
        return Err(GenieError::invalid(format!("input must contain between 1 and {MAX_INPUT_MESSAGES} messages")));
    }
    let mut bytes = 0usize;
    for message in &request.input {
        if !matches!(message.role.as_str(), "system" | "developer" | "user" | "assistant") {
            return Err(GenieError::invalid("only text system, developer, user and assistant messages are supported"));
        }
        bytes = bytes.saturating_add(message.content.len());
    }
    if bytes == 0 || bytes > MAX_INPUT_BYTES {
        return Err(GenieError::invalid(format!("input must contain between 1 and {MAX_INPUT_BYTES} bytes of text")));
    }
    if request.idempotency_key.as_ref().is_some_and(|key| key.is_empty() || key.len() > MAX_IDEMPOTENCY_KEY_BYTES) {
        return Err(GenieError::invalid(format!("idempotency key must contain between 1 and {MAX_IDEMPOTENCY_KEY_BYTES} bytes")));
    }
    Ok(())
}

fn stored<T: serde::de::DeserializeOwned>(r: &Row<'_>, index: usize) -> rusqlite::Result<T> {
    let value: String = r.get(index)?;
    serde_json::from_str(&value).map_err(|e| rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(e)))
}

fn enum_value<T: serde::de::DeserializeOwned>(r: &Row<'_>, index: usize) -> rusqlite::Result<T> {
    let value: String = r.get(index)?;
    serde_json::from_value(json!(value)).map_err(|e| rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(e)))
}

fn response(r: &Row<'_>) -> rusqlite::Result<Response> {
    Ok(Response {
        id: r.get(0)?,
        conversation_id: r.get(1)?,
        principal: r.get(2)?,
        initiator: r.get(3)?,
        previous_response_id: r.get(4)?,
        input: stored(r, 5)?,
        status: enum_value(r, 6)?,
        output: r.get(7)?,
        error: r.get(8)?,
        created_at: r.get(9)?,
        completed_at: r.get(10)?,
    })
}

fn proposal(r: &Row<'_>) -> rusqlite::Result<Proposal> {
    Ok(Proposal {
        id: r.get(0)?,
        conversation_id: r.get(1)?,
        task: r.get(2)?,
        action: enum_value(r, 3)?,
        version: r.get(4)?,
        status: enum_value(r, 5)?,
        created_at: r.get(6)?,
        decided_at: r.get(7)?,
    })
}
