//! People-facing delivery: the web notification center, the outbox that channels
//! (Telegram, e-mail) drain with retries, channel links, and questionnaires —
//! questions from agents to a person, answered in the web, Telegram or by mail.

use chrono::Duration as ChronoDuration;
use rusqlite::{OptionalExtension, Row, params};
use serde::Serialize;
use serde_json::{Value, json};

use crate::db::now;
use crate::error::{GenieError, Result};
use crate::server_db::{ServerDb, hash_secret, new_code, new_secret, time_in};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    pub id: i64,
    pub user: i64,
    pub project: Option<String>,
    pub task: Option<String>,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub link: Option<String>,
    pub created: String,
    pub read_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxItem {
    pub id: i64,
    pub user: Option<i64>,
    pub channel: String,
    pub address: String,
    pub subject: String,
    pub body: String,
    pub payload: Value,
    pub status: String,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub external_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    pub n: i64,
    pub text: String,
    pub why: String,
    pub options: Vec<String>,
    pub answer: Option<String>,
    pub answered_at: Option<String>,
    pub answered_via: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Questionnaire {
    pub id: i64,
    pub project: String,
    pub task: Option<String>,
    pub asked_by: String,
    pub recipient: i64,
    pub channel: String,
    pub status: String,
    pub run_step: Option<i64>,
    pub remind_at: Option<String>,
    pub due: Option<String>,
    pub created: String,
    pub closed: Option<String>,
    pub questions: Vec<Question>,
}

#[derive(Debug, Clone, Default)]
pub struct NewQuestion {
    pub text: String,
    pub why: String,
    pub options: Vec<String>,
}

pub const MAX_DELIVERY_ATTEMPTS: i64 = 8;

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS questionnaire_tokens (
  token_hash TEXT PRIMARY KEY,
  questionnaire INTEGER NOT NULL REFERENCES questionnaires(id) ON DELETE CASCADE
);
INSERT OR IGNORE INTO questionnaire_tokens SELECT token_hash, id FROM questionnaires;
CREATE TABLE IF NOT EXISTS questionnaire_records (
  id TEXT PRIMARY KEY,
  questionnaire INTEGER NOT NULL UNIQUE REFERENCES questionnaires(id) ON DELETE CASCADE,
  actor TEXT NOT NULL,
  body TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  attempts INTEGER NOT NULL DEFAULT 0,
  next_at TEXT NOT NULL,
  last_error TEXT
);
CREATE INDEX IF NOT EXISTS questionnaire_records_due ON questionnaire_records(status, next_at);
CREATE TABLE IF NOT EXISTS questionnaire_sends (
  questionnaire INTEGER NOT NULL REFERENCES questionnaires(id) ON DELETE CASCADE,
  reminder INTEGER NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  attempts INTEGER NOT NULL DEFAULT 0,
  next_at TEXT NOT NULL,
  last_error TEXT,
  PRIMARY KEY(questionnaire,reminder)
);
CREATE INDEX IF NOT EXISTS questionnaire_sends_due ON questionnaire_sends(status,next_at);
"#;

/// Accepted answers awaiting their idempotent task-side effect. The actor and text are
/// frozen when the answers commit; a later rename cannot change a replay's identity.
#[derive(Debug, Clone)]
pub struct AnswerRecord {
    pub id: String,
    pub questionnaire: i64,
    pub actor: String,
    pub body: String,
}

impl ServerDb {
    // --- notification center -------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn add_notification(
        &self,
        user: i64,
        project: Option<&str>,
        task: Option<&str>,
        kind: &str,
        title: &str,
        body: &str,
        link: Option<&str>,
        dedupe_key: Option<&str>,
    ) -> Result<Option<i64>> {
        // A repeated `dedupe_key` is ignored (returns `None`).
        let n = self.conn().execute(
            "INSERT OR IGNORE INTO notifications(user, project, task, kind, title, body, link, dedupe_key, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![user, project, task, kind, title, body, link, dedupe_key, now()],
        )?;
        Ok((n > 0).then(|| self.conn().last_insert_rowid()))
    }

    pub fn notifications(&self, user: i64, unread_only: bool, limit: i64) -> Result<Vec<Notification>> {
        let mut stmt =
            self.conn().prepare("SELECT * FROM notifications WHERE user = ?1 AND (?2 = 0 OR read_at IS NULL) ORDER BY id DESC LIMIT ?3")?;
        let rows = stmt.query_map(params![user, unread_only as i64, limit], |r| {
            Ok(Notification {
                id: r.get("id")?,
                user: r.get("user")?,
                project: r.get("project")?,
                task: r.get("task")?,
                kind: r.get("kind")?,
                title: r.get("title")?,
                body: r.get("body")?,
                link: r.get("link")?,
                created: r.get("created")?,
                read_at: r.get("read_at")?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn unread_count(&self, user: i64) -> Result<i64> {
        Ok(self.conn().query_row("SELECT COUNT(*) FROM notifications WHERE user = ?1 AND read_at IS NULL", [user], |r| r.get(0))?)
    }

    pub fn mark_read(&self, user: i64, id: Option<i64>) -> Result<()> {
        self.conn().execute(
            "UPDATE notifications SET read_at = ?1 WHERE user = ?2 AND read_at IS NULL AND (?3 IS NULL OR id = ?3)",
            params![now(), user, id],
        )?;
        Ok(())
    }

    // --- channel links ---------------------------------------------------------

    pub fn channel_address(&self, user: i64, channel: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT address FROM channel_links WHERE user = ?1 AND channel = ?2", params![user, channel], |r| r.get(0))
            .optional()?)
    }

    pub fn link_channel(&self, user: i64, channel: &str, address: &str) -> Result<()> {
        self.tx(|| {
            if (channel == "telegram" || channel.starts_with("telegram:")) && self.channel_claimed_by_other(user, channel, address)? {
                return Err(GenieError::Denied("this Telegram chat is already linked to another account".into()));
            }
            self.conn().execute(
                "INSERT INTO channel_links(user, channel, address, created) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(user, channel) DO UPDATE SET address = excluded.address",
                params![user, channel, address, now()],
            )?;
            Ok(())
        })
    }

    pub(crate) fn channel_claimed_by_other(&self, user: i64, channel: &str, address: &str) -> Result<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS(SELECT 1 FROM channel_links WHERE channel=?1 AND address=?2 AND user!=?3)",
            params![channel, address, user],
            |r| r.get(0),
        )?)
    }

    pub fn unlink_channel(&self, user: i64, channel: &str) -> Result<()> {
        self.conn().execute("DELETE FROM channel_links WHERE user = ?1 AND channel = ?2", params![user, channel])?;
        Ok(())
    }

    pub fn user_by_channel(&self, channel: &str, address: &str) -> Result<Option<i64>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT MIN(user) FROM channel_links WHERE channel = ?1 AND address = ?2 HAVING COUNT(*) = 1",
                params![channel, address],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn channel_links(&self, user: i64) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn().prepare("SELECT channel, address FROM channel_links WHERE user = ?1 ORDER BY channel")?;
        Ok(stmt.query_map([user], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?)
    }

    /// One-time code a person sends to the bot (`/start CODE`) to link the chat.
    pub fn create_link_code(&self, user: i64, channel: &str) -> Result<String> {
        let code = new_code();
        self.conn().execute("DELETE FROM link_codes WHERE user = ?1 AND channel = ?2", params![user, channel])?;
        self.conn().execute(
            "INSERT INTO link_codes(code, user, channel, expires) VALUES (?1, ?2, ?3, ?4)",
            params![code, user, channel, time_in(ChronoDuration::minutes(30))],
        )?;
        Ok(code)
    }

    pub fn redeem_link_code(&self, code: &str, channel: &str, address: &str) -> Result<Option<i64>> {
        self.tx(|| {
            let user: Option<i64> = self
                .conn()
                .query_row(
                    "SELECT user FROM link_codes WHERE code = ?1 AND channel = ?2 AND expires > ?3",
                    params![code.trim().to_uppercase(), channel, now()],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(u) = user {
                if self.user(u)?.disabled || self.channel_claimed_by_other(u, channel, address)? {
                    return Ok(None);
                }
                self.conn().execute("DELETE FROM link_codes WHERE code = ?1", [code.trim().to_uppercase()])?;
                self.link_channel(u, channel, address)?;
            }
            Ok(user)
        })
    }

    // --- outbox -------------------------------------------------------------------

    /// Queue a message for a channel. A repeated `dedupe_key` is ignored.
    #[allow(clippy::too_many_arguments)]
    pub fn enqueue(
        &self,
        dedupe_key: Option<&str>,
        user: Option<i64>,
        channel: &str,
        address: &str,
        subject: &str,
        body: &str,
        payload: &Value,
    ) -> Result<Option<i64>> {
        self.tx(|| {
        let n = self.conn().execute(
            "INSERT OR IGNORE INTO outbox(dedupe_key, user, channel, address, subject, body, payload, next_at, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![dedupe_key, user, channel, address, subject, body, payload.to_string(), now()],
        )?;
        let id = (n > 0).then(|| self.conn().last_insert_rowid());
        if let (Some(id), Some(user), Some(project), Some(reference), Some(version)) = (id,user,payload["project"].as_str(),payload["interaction"]["id"].as_str(),payload["interaction"]["version"].as_str()) {
            self.register_interaction_delivery(id,project,reference,version,user)?;
        }
        Ok(id)
        })
    }

    pub fn due_outbox(&self, limit: i64) -> Result<Vec<OutboxItem>> {
        let mut stmt =
            self.conn().prepare("SELECT * FROM outbox WHERE status IN ('pending','delivering') AND next_at <= ?1 ORDER BY id LIMIT ?2")?;
        let rows = stmt.query_map(params![now(), limit], outbox_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// When the earliest pending message falls due (`None`: nothing is pending): a failed
    /// delivery waits for its backoff, and nobody wakes the dispatcher when it ends.
    pub fn next_outbox_at(&self) -> Result<Option<String>> {
        Ok(self.conn().query_row("SELECT MIN(next_at) FROM outbox WHERE status IN ('pending','delivering')", [], |r| r.get(0))?)
    }

    pub fn outbox_item(&self, id: i64) -> Result<OutboxItem> {
        Ok(self.conn().query_row("SELECT * FROM outbox WHERE id = ?1", [id], outbox_row)?)
    }

    pub fn outbox_by_key(&self, key: &str) -> Result<Option<OutboxItem>> {
        Ok(self.conn().query_row("SELECT * FROM outbox WHERE dedupe_key = ?1", [key], outbox_row).optional()?)
    }

    /// A raised fallback threshold continues the existing preferred-channel
    /// attempts. Keep its backoff/lease and make this copy wait for that outcome.
    pub fn defer_outbox(&self, id: i64, preferred: i64) -> Result<()> {
        self.tx(|| {
            self.conn().execute(
                "UPDATE outbox SET status='pending' WHERE id=?1 AND status='failed' AND attempts<?2",
                params![preferred, MAX_DELIVERY_ATTEMPTS],
            )?;
            self.conn().execute(
                "UPDATE outbox SET status='pending',next_at=(SELECT next_at FROM outbox WHERE id=?2) WHERE id=?1",
                params![id, preferred],
            )?;
            Ok(())
        })
    }

    /// Each network operation has a 60-second timeout or less. A two-minute lease prevents
    /// concurrent dispatch, and an interrupted send becomes retryable after restart.
    pub fn claim_outbox(&self, id: i64) -> Result<bool> {
        Ok(self.conn().execute(
            "UPDATE outbox SET status = 'delivering', next_at = ?2 WHERE id = ?1 AND status IN ('pending','delivering') AND next_at <= ?3",
            params![id, time_in(ChronoDuration::seconds(120)), now()],
        )? > 0)
    }

    pub fn outbox_cancelled(&self, id: i64, reason: &str) -> Result<()> {
        self.conn().execute("UPDATE outbox SET status = 'cancelled', last_error = ?2 WHERE id = ?1", params![id, reason])?;
        Ok(())
    }

    pub fn outbox_sent(&self, id: i64, external_ref: Option<&str>) -> Result<()> {
        self.conn().execute(
            "UPDATE outbox SET status = 'sent', sent_at = ?1, attempts = attempts + 1, external_ref = ?2 WHERE id = ?3",
            params![now(), external_ref, id],
        )?;
        self.conn().execute(
            "UPDATE outbox SET next_at=?2 WHERE channel='interaction_sync' AND status='pending' AND json_extract(payload,'$.original')=?1",
            params![id, now()],
        )?;
        Ok(())
    }

    /// A failed delivery is retried with exponential backoff, then given up.
    pub fn outbox_failed(&self, id: i64, error: &str) -> Result<()> {
        let attempts: i64 = self.conn().query_row("SELECT attempts FROM outbox WHERE id = ?1", [id], |r| r.get(0))?;
        let attempts = attempts + 1;
        let status = if attempts >= MAX_DELIVERY_ATTEMPTS { "failed" } else { "pending" };
        let delay = ChronoDuration::seconds((30i64 << (attempts - 1).min(8)).min(6 * 3600));
        self.conn().execute(
            "UPDATE outbox SET status = ?1, attempts = ?2, last_error = ?3, next_at = ?4 WHERE id = ?5",
            params![status, attempts, error, time_in(delay), id],
        )?;
        Ok(())
    }

    pub fn outbox_by_ref(&self, channel: &str, external_ref: &str) -> Result<Option<Value>> {
        Ok(self
            .conn()
            .query_row("SELECT payload FROM outbox WHERE channel = ?1 AND external_ref = ?2", params![channel, external_ref], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .and_then(|p| serde_json::from_str(&p).ok()))
    }

    // --- questionnaires --------------------------------------------------------

    /// Create a questionnaire; returns it with the secret for the web answer link.
    #[allow(clippy::too_many_arguments)]
    pub fn create_questionnaire(
        &self,
        project: &str,
        task: Option<&str>,
        asked_by: &str,
        recipient: i64,
        channel: &str,
        questions: &[NewQuestion],
        run_step: Option<i64>,
        remind_after: Option<ChronoDuration>,
        timeout: Option<ChronoDuration>,
    ) -> Result<(Questionnaire, String)> {
        if questions.is_empty() {
            return Err(GenieError::invalid("a questionnaire needs at least one question"));
        }
        let secret = new_secret();
        let id = self.tx(|| {
            self.conn().execute(
                "INSERT INTO questionnaires(project, task, asked_by, recipient, channel, status, token_hash, run_step, remind_at, due, created)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'open', ?6, ?7, ?8, ?9, ?10)",
                params![project, task, asked_by, recipient, channel, hash_secret(&secret), run_step, remind_after.map(time_in), timeout.map(time_in), now()],
            )?;
            let id = self.conn().last_insert_rowid();
            self.conn().execute("INSERT INTO questionnaire_tokens VALUES (?1, ?2)", params![hash_secret(&secret), id])?;
            for (i, q) in questions.iter().enumerate() {
                self.conn().execute(
                    "INSERT INTO questions(questionnaire, n, text, why, options) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![id, i as i64 + 1, q.text.trim(), q.why.trim(), serde_json::to_string(&q.options)?],
                )?;
            }
            Ok(id)
        })?;
        Ok((self.questionnaire(id)?, secret))
    }

    pub fn questionnaire(&self, id: i64) -> Result<Questionnaire> {
        let mut qn = self
            .conn()
            .query_row("SELECT * FROM questionnaires WHERE id = ?1", [id], questionnaire_row)
            .optional()?
            .ok_or_else(|| GenieError::not_found(format!("questionnaire {id} not found")))?;
        let mut stmt = self.conn().prepare("SELECT * FROM questions WHERE questionnaire = ?1 ORDER BY n")?;
        qn.questions = stmt
            .query_map([id], |r| {
                let options: String = r.get("options")?;
                Ok(Question {
                    n: r.get("n")?,
                    text: r.get("text")?,
                    why: r.get("why")?,
                    options: serde_json::from_str(&options).unwrap_or_default(),
                    answer: r.get("answer")?,
                    answered_at: r.get("answered_at")?,
                    answered_via: r.get("answered_via")?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(qn)
    }

    pub fn questionnaire_by_secret(&self, secret: &str) -> Result<Option<Questionnaire>> {
        let id: Option<i64> = self
            .conn()
            .query_row("SELECT questionnaire FROM questionnaire_tokens WHERE token_hash = ?1", [hash_secret(secret)], |r| r.get(0))
            .optional()?;
        id.map(|id| self.questionnaire(id)).transpose()
    }

    /// Reminders add credentials for the same questions; existing in-flight links stay valid.
    pub fn questionnaire_link(&self, id: i64) -> Result<String> {
        self.questionnaire(id)?;
        let secret = new_secret();
        self.conn().execute("INSERT INTO questionnaire_tokens VALUES (?1, ?2)", params![hash_secret(&secret), id])?;
        Ok(secret)
    }

    pub fn questionnaires_for(&self, user: i64, open_only: bool) -> Result<Vec<Questionnaire>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT id FROM questionnaires WHERE recipient = ?1 AND (?2 = 0 OR status = 'open') ORDER BY id DESC LIMIT 100")?;
        let ids = stmt.query_map(params![user, open_only as i64], |r| r.get::<_, i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter().map(|id| self.questionnaire(id)).collect()
    }

    pub fn open_questionnaires(&self) -> Result<Vec<Questionnaire>> {
        let mut stmt = self.conn().prepare_cached("SELECT id FROM questionnaires WHERE status = 'open' ORDER BY id")?;
        let ids = stmt.query_map([], |r| r.get::<_, i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter().map(|id| self.questionnaire(id)).collect()
    }

    /// Questionnaires about a task that still wait for answers.
    pub fn open_questionnaires_for_task(&self, project: &str, task: &str) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM questionnaires WHERE project = ?1 AND task = ?2 AND status = 'open'",
            params![project, task],
            |r| r.get(0),
        )?)
    }

    /// Record an answer. Returns the questionnaire and whether this answer completed it.
    pub fn answer_question(&self, id: i64, n: i64, answer: &str, via: &str) -> Result<(Questionnaire, bool)> {
        self.answer_questions(id, &[(n, answer.to_string())], via)
    }

    /// Validate a whole submission before writing any answer. The first valid answer wins;
    /// an identical retry preserves its original timestamp and channel, including after completion.
    pub fn answer_questions(&self, id: i64, answers: &[(i64, String)], via: &str) -> Result<(Questionnaire, bool)> {
        if answers.is_empty() || via.trim().is_empty() {
            return Err(GenieError::invalid("an answer submission needs answers and a source"));
        }
        self.tx(|| {
            let qn = self.questionnaire(id)?;
            if !matches!(qn.status.as_str(), "open" | "answered") || (qn.status == "open" && qn.due.as_ref().is_some_and(|d| *d <= now())) {
                return Err(GenieError::invalid(format!("questionnaire {id} is {}", qn.status)));
            }
            let mut seen = std::collections::BTreeSet::new();
            for (n, answer) in answers {
                if answer.trim().is_empty() || !seen.insert(n) {
                    return Err(GenieError::invalid("answers must be nonempty and question numbers unique"));
                }
                let q = qn.questions.iter().find(|q| q.n == *n)
                    .ok_or_else(|| GenieError::not_found(format!("questionnaire {id} has no question {n}")))?;
                if q.answer.as_deref().is_some_and(|old| old != answer.trim()) {
                    return Err(GenieError::invalid(format!("questionnaire {id} is {}: question {n} already has a different answer", qn.status)));
                }
            }
            for (n, answer) in answers {
                self.conn().execute(
                    "UPDATE questions SET answer = ?1, answered_at = ?2, answered_via = ?3 WHERE questionnaire = ?4 AND n = ?5 AND answer IS NULL",
                    params![answer.trim(), now(), via, id, n],
                )?;
            }
            let open: i64 =
                self.conn().query_row("SELECT COUNT(*) FROM questions WHERE questionnaire = ?1 AND answer IS NULL", [id], |r| r.get(0))?;
            let completed = open == 0 && qn.status == "open";
            if completed {
                self.conn().execute("UPDATE questionnaires SET status = 'answered', closed = ?1 WHERE id = ?2", params![now(), id])?;
            }
            Ok((self.questionnaire(id)?, completed))
        })
    }

    /// Call inside the transaction accepting the final answer.
    pub fn queue_answer_record(&self, id: i64, actor: &str, body: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO questionnaire_records(id, questionnaire, actor, body, next_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![new_secret(), id, actor, body, now()],
        )?;
        Ok(())
    }

    pub fn due_answer_records(&self) -> Result<Vec<AnswerRecord>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT * FROM questionnaire_records WHERE status = 'pending' AND next_at <= ?1 ORDER BY next_at LIMIT 50")?;
        Ok(stmt
            .query_map([now()], |r| {
                Ok(AnswerRecord { id: r.get("id")?, questionnaire: r.get("questionnaire")?, actor: r.get("actor")?, body: r.get("body")? })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn next_answer_record_at(&self) -> Result<Option<String>> {
        Ok(self.conn().query_row("SELECT MIN(next_at) FROM questionnaire_records WHERE status = 'pending'", [], |r| r.get(0))?)
    }

    pub fn answer_recorded(&self, id: &str) -> Result<()> {
        self.conn().execute("UPDATE questionnaire_records SET status = 'done', last_error = NULL WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn answer_record_failed(&self, id: &str, error: &str) -> Result<()> {
        let attempts: i64 = self.conn().query_row("SELECT attempts FROM questionnaire_records WHERE id = ?1", [id], |r| r.get(0))?;
        let delay = ChronoDuration::seconds((30i64 << attempts.min(8)).min(6 * 3600));
        self.conn().execute(
            "UPDATE questionnaire_records SET attempts = attempts + 1, last_error = ?2, next_at = ?3 WHERE id = ?1",
            params![id, error, time_in(delay)],
        )?;
        Ok(())
    }

    pub fn answer_record_status(&self, id: i64) -> Result<Option<String>> {
        Ok(self.conn().query_row("SELECT status FROM questionnaire_records WHERE questionnaire=?1", [id], |r| r.get(0)).optional()?)
    }

    /// Queue in the same transaction as creating the questions or consuming the reminder.
    pub fn queue_questionnaire_send(&self, id: i64, reminder: bool) -> Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO questionnaire_sends(questionnaire,reminder,next_at) VALUES (?1,?2,?3)",
            params![id, reminder, now()],
        )?;
        Ok(())
    }

    pub fn due_questionnaire_sends(&self) -> Result<Vec<(i64, bool)>> {
        let mut stmt = self.conn().prepare(
            "SELECT questionnaire,reminder FROM questionnaire_sends WHERE status='pending' AND next_at<=?1 ORDER BY next_at LIMIT 50",
        )?;
        Ok(stmt.query_map([now()], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn next_questionnaire_send_at(&self) -> Result<Option<String>> {
        Ok(self.conn().query_row("SELECT MIN(next_at) FROM questionnaire_sends WHERE status='pending'", [], |r| r.get(0))?)
    }

    pub fn questionnaire_sent(&self, id: i64, reminder: bool, error: Option<&str>) -> Result<()> {
        if let Some(error) = error {
            let attempts: i64 = self.conn().query_row(
                "SELECT attempts FROM questionnaire_sends WHERE questionnaire=?1 AND reminder=?2",
                params![id, reminder],
                |r| r.get(0),
            )?;
            self.conn().execute(
                "UPDATE questionnaire_sends SET attempts=attempts+1,last_error=?3,next_at=?4 WHERE questionnaire=?1 AND reminder=?2",
                params![id, reminder, error, time_in(ChronoDuration::seconds((30i64 << attempts.min(8)).min(6 * 3600)))],
            )?;
        } else {
            self.conn().execute(
                "UPDATE questionnaire_sends SET status='done',last_error=NULL WHERE questionnaire=?1 AND reminder=?2",
                params![id, reminder],
            )?;
        }
        Ok(())
    }

    pub fn close_questionnaire(&self, id: i64, status: &str) -> Result<()> {
        self.conn()
            .execute("UPDATE questionnaires SET status = ?1, closed = ?2 WHERE id = ?3 AND status = 'open'", params![status, now(), id])?;
        Ok(())
    }

    pub fn clear_reminder(&self, id: i64) -> Result<()> {
        self.conn().execute("UPDATE questionnaires SET remind_at = NULL WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Link an outgoing message to a question, so a reply to it answers that question.
    pub fn set_question_message(&self, id: i64, n: i64, message_ref: &str) -> Result<()> {
        self.conn().execute("UPDATE questions SET message_ref = ?1 WHERE questionnaire = ?2 AND n = ?3", params![message_ref, id, n])?;
        Ok(())
    }

    pub fn question_by_message(&self, message_ref: &str) -> Result<Option<(i64, i64)>> {
        Ok(self
            .conn()
            .query_row("SELECT questionnaire, n FROM questions WHERE message_ref = ?1", [message_ref], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?)
    }
}

fn outbox_row(r: &Row<'_>) -> rusqlite::Result<OutboxItem> {
    let payload: String = r.get("payload")?;
    Ok(OutboxItem {
        id: r.get("id")?,
        user: r.get("user")?,
        channel: r.get("channel")?,
        address: r.get("address")?,
        subject: r.get("subject")?,
        body: r.get("body")?,
        payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
        status: r.get("status")?,
        attempts: r.get("attempts")?,
        last_error: r.get("last_error")?,
        external_ref: r.get("external_ref")?,
    })
}

fn questionnaire_row(r: &Row<'_>) -> rusqlite::Result<Questionnaire> {
    Ok(Questionnaire {
        id: r.get("id")?,
        project: r.get("project")?,
        task: r.get("task")?,
        asked_by: r.get("asked_by")?,
        recipient: r.get("recipient")?,
        channel: r.get("channel")?,
        status: r.get("status")?,
        run_step: r.get("run_step")?,
        remind_at: r.get("remind_at")?,
        due: r.get("due")?,
        created: r.get("created")?,
        closed: r.get("closed")?,
        questions: Vec::new(),
    })
}

/// JSON of answers for automation steps: `[{ "question", "answer" }]`.
pub fn answers_json(q: &Questionnaire) -> Value {
    json!(q.questions.iter().map(|x| json!({ "n": x.n, "question": x.text, "answer": x.answer })).collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbox_retries_then_gives_up_and_dedupes() {
        let dir = tempfile::tempdir().unwrap();
        let db = ServerDb::open(&dir.path().join("s.db")).unwrap();
        let id = db.enqueue(Some("k1"), None, "email", "a@b.c", "s", "b", &json!({})).unwrap().unwrap();
        assert!(db.enqueue(Some("k1"), None, "email", "a@b.c", "s", "b", &json!({})).unwrap().is_none());
        assert_eq!(db.due_outbox(10).unwrap().len(), 1);
        db.outbox_failed(id, "smtp down").unwrap();
        assert!(db.due_outbox(10).unwrap().is_empty(), "retried later, not immediately");
        // The dispatcher sleeps until the retry falls due.
        assert!(db.next_outbox_at().unwrap().unwrap() > now());
        for _ in 1..MAX_DELIVERY_ATTEMPTS {
            db.outbox_failed(id, "smtp down").unwrap();
        }
        let status: String = db.conn().query_row("SELECT status FROM outbox WHERE id = ?1", [id], |r| r.get(0)).unwrap();
        assert_eq!(status, "failed");
        assert_eq!(db.next_outbox_at().unwrap(), None, "nothing pending, nothing to wake up for");
    }

    #[test]
    fn questionnaires_complete_when_every_question_is_answered() {
        let dir = tempfile::tempdir().unwrap();
        let db = ServerDb::open(&dir.path().join("s.db")).unwrap();
        let u = db.create_user("pm", "", None, None, false).unwrap();
        let qs = vec![
            NewQuestion { text: "Формат?".into(), why: "".into(), options: vec!["CSV".into(), "XLSX".into()] },
            NewQuestion { text: "Кто получает?".into(), ..Default::default() },
        ];
        let (qn, secret) = db.create_questionnaire("shop", Some("G-1"), "sherlock", u.id, "telegram", &qs, None, None, None).unwrap();
        assert_eq!(db.questionnaire_by_secret(&secret).unwrap().unwrap().id, qn.id);
        let (_, done) = db.answer_question(qn.id, 1, "CSV", "telegram").unwrap();
        assert!(!done);
        let (q, done) = db.answer_question(qn.id, 2, "бухгалтерия", "web").unwrap();
        assert!(done && q.status == "answered");
        assert!(db.answer_question(qn.id, 2, "x", "web").is_err(), "closed questionnaires take no answers");
        assert_eq!(answers_json(&q)[0]["answer"], "CSV");
        let code = db.create_link_code(u.id, "telegram").unwrap();
        assert_eq!(db.redeem_link_code(&code.to_lowercase(), "telegram", "12345").unwrap(), Some(u.id));
        assert_eq!(db.user_by_channel("telegram", "12345").unwrap(), Some(u.id));
        assert!(db.redeem_link_code(&code, "telegram", "999").unwrap().is_none(), "codes work once");
    }

    #[test]
    fn answer_batches_are_atomic_and_first_valid_answer_survives_retries_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.db");
        let db = ServerDb::open(&path).unwrap();
        let u = db.create_user("pm", "", None, None, false).unwrap();
        let qs =
            [NewQuestion { text: "Format?".into(), ..Default::default() }, NewQuestion { text: "Recipient?".into(), ..Default::default() }];
        let (qn, original) = db.create_questionnaire("shop", None, "analyst", u.id, "web", &qs, None, None, None).unwrap();
        let reminder = db.questionnaire_link(qn.id).unwrap();
        assert!(db.answer_questions(qn.id, &[(1, "CSV".into()), (99, "bad".into())], "web").is_err());
        assert!(db.questionnaire(qn.id).unwrap().questions.iter().all(|q| q.answer.is_none()));
        db.answer_question(qn.id, 1, "CSV", "telegram").unwrap();
        let first = db.questionnaire(qn.id).unwrap().questions[0].clone();
        assert!(db.answer_questions(qn.id, &[(2, "Anna".into()), (1, "XLSX".into())], "web").is_err());
        assert!(db.questionnaire(qn.id).unwrap().questions[1].answer.is_none());
        assert!(db.answer_questions(qn.id, &[(2, "Anna".into()), (2, "Bob".into())], "web").is_err());
        drop(db);
        let db = ServerDb::open(&path).unwrap();
        for link in [&original, &reminder] {
            assert_eq!(db.questionnaire_by_secret(link).unwrap().unwrap().id, qn.id);
        }
        db.answer_question(qn.id, 2, "Anna", "web").unwrap();
        let (again, newly_completed) = db.answer_question(qn.id, 1, " CSV ", "api").unwrap();
        assert!(!newly_completed);
        assert_eq!(again.questions[0].answered_at, first.answered_at);
        assert_eq!(again.questions[0].answered_via, first.answered_via);
        assert!(db.answer_question(qn.id, 1, "XLSX", "web").is_err());
    }

    #[test]
    fn concurrent_channels_can_only_commit_one_different_answer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.db");
        let db = ServerDb::open(&path).unwrap();
        let u = db.create_user("pm", "", None, None, false).unwrap();
        let (qn, _) = db
            .create_questionnaire(
                "shop",
                None,
                "analyst",
                u.id,
                "web",
                &[NewQuestion { text: "When?".into(), ..Default::default() }],
                None,
                None,
                None,
            )
            .unwrap();
        drop(db);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = ["today", "tomorrow"]
            .into_iter()
            .map(|text| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let db = ServerDb::open(&path).unwrap();
                    barrier.wait();
                    db.answer_question(qn.id, 1, text, "api").is_ok()
                })
            })
            .collect();
        assert_eq!(handles.into_iter().map(|h| h.join().unwrap() as usize).sum::<usize>(), 1);
    }

    #[test]
    fn schema_12_keeps_existing_answer_links_without_backfilling_completed_effects() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.db");
        let db = ServerDb::open(&path).unwrap();
        let u = db.create_user("pm", "", None, None, false).unwrap();
        let (qn, secret) = db
            .create_questionnaire(
                "shop",
                Some("G-1"),
                "analyst",
                u.id,
                "web",
                &[NewQuestion { text: "When?".into(), ..Default::default() }],
                None,
                None,
                None,
            )
            .unwrap();
        db.answer_question(qn.id, 1, "today", "web").unwrap();
        db.conn()
            .execute_batch(
                "DROP TABLE questionnaire_tokens; DROP TABLE questionnaire_records; DROP TABLE questionnaire_sends; DROP TABLE delivery_policies; DROP TABLE interaction_deliveries; UPDATE meta SET value = '12' WHERE key = 'schema';",
            )
            .unwrap();
        drop(db);
        let db = ServerDb::open(&path).unwrap();
        assert_eq!(db.questionnaire_by_secret(&secret).unwrap().unwrap().status, "answered");
        assert!(db.due_answer_records().unwrap().is_empty(), "historical comments must not be repeated during upgrade");
    }

    #[test]
    fn telegram_links_cannot_be_claimed_twice_and_ambiguous_old_links_do_not_authenticate() {
        let dir = tempfile::tempdir().unwrap();
        let db = ServerDb::open(&dir.path().join("s.db")).unwrap();
        let anna = db.create_user("anna", "", None, None, false).unwrap();
        let bob = db.create_user("bob", "", None, None, false).unwrap();
        db.link_channel(anna.id, "telegram", "42").unwrap();
        let code = db.create_link_code(bob.id, "telegram").unwrap();
        assert_eq!(db.redeem_link_code(&code, "telegram", "42").unwrap(), None);
        assert!(db.link_channel(bob.id, "telegram", "42").is_err());
        db.conn().execute("INSERT INTO channel_links VALUES (?1,'telegram','42',?2)", params![bob.id, now()]).unwrap();
        assert_eq!(db.user_by_channel("telegram", "42").unwrap(), None, "do not choose one historical claimant arbitrarily");
    }

    #[test]
    fn delivery_leases_serialize_dispatch_and_recover_an_interrupted_send() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.db");
        let db = ServerDb::open(&path).unwrap();
        let id = db.enqueue(Some("send"), None, "email", "recipient@example.test", "subject", "body", &json!({})).unwrap().unwrap();
        assert!(db.claim_outbox(id).unwrap());
        assert!(!db.claim_outbox(id).unwrap());
        assert!(db.due_outbox(10).unwrap().is_empty());
        drop(db);
        let db = ServerDb::open(&path).unwrap();
        db.conn().execute("UPDATE outbox SET next_at='2000-01-01' WHERE id=?1", [id]).unwrap();
        assert_eq!(db.due_outbox(10).unwrap().len(), 1);
        assert!(db.claim_outbox(id).unwrap());
        db.outbox_sent(id, Some("receipt")).unwrap();
        assert!(!db.claim_outbox(id).unwrap());
        assert_eq!(db.outbox_item(id).unwrap().external_ref.as_deref(), Some("receipt"));
    }
}
