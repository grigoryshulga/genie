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
        self.conn().execute(
            "INSERT INTO channel_links(user, channel, address, created) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(user, channel) DO UPDATE SET address = excluded.address",
            params![user, channel, address, now()],
        )?;
        Ok(())
    }

    pub fn unlink_channel(&self, user: i64, channel: &str) -> Result<()> {
        self.conn().execute("DELETE FROM channel_links WHERE user = ?1 AND channel = ?2", params![user, channel])?;
        Ok(())
    }

    pub fn user_by_channel(&self, channel: &str, address: &str) -> Result<Option<i64>> {
        Ok(self
            .conn()
            .query_row("SELECT user FROM channel_links WHERE channel = ?1 AND address = ?2", params![channel, address], |r| r.get(0))
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
        let n = self.conn().execute(
            "INSERT OR IGNORE INTO outbox(dedupe_key, user, channel, address, subject, body, payload, next_at, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![dedupe_key, user, channel, address, subject, body, payload.to_string(), now()],
        )?;
        Ok((n > 0).then(|| self.conn().last_insert_rowid()))
    }

    pub fn due_outbox(&self, limit: i64) -> Result<Vec<OutboxItem>> {
        let mut stmt = self.conn().prepare("SELECT * FROM outbox WHERE status = 'pending' AND next_at <= ?1 ORDER BY id LIMIT ?2")?;
        let rows = stmt.query_map(params![now(), limit], |r| {
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
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// When the earliest pending message falls due (`None`: nothing is pending): a failed
    /// delivery waits for its backoff, and nobody wakes the dispatcher when it ends.
    pub fn next_outbox_at(&self) -> Result<Option<String>> {
        Ok(self.conn().query_row("SELECT MIN(next_at) FROM outbox WHERE status = 'pending'", [], |r| r.get(0))?)
    }

    pub fn outbox_sent(&self, id: i64, external_ref: Option<&str>) -> Result<()> {
        self.conn().execute(
            "UPDATE outbox SET status = 'sent', sent_at = ?1, attempts = attempts + 1, external_ref = ?2 WHERE id = ?3",
            params![now(), external_ref, id],
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
        let id: Option<i64> =
            self.conn().query_row("SELECT id FROM questionnaires WHERE token_hash = ?1", [hash_secret(secret)], |r| r.get(0)).optional()?;
        id.map(|id| self.questionnaire(id)).transpose()
    }

    pub fn questionnaires_for(&self, user: i64, open_only: bool) -> Result<Vec<Questionnaire>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT id FROM questionnaires WHERE recipient = ?1 AND (?2 = 0 OR status = 'open') ORDER BY id DESC LIMIT 100")?;
        let ids = stmt.query_map(params![user, open_only as i64], |r| r.get::<_, i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter().map(|id| self.questionnaire(id)).collect()
    }

    pub fn open_questionnaires(&self) -> Result<Vec<Questionnaire>> {
        let mut stmt = self.conn().prepare("SELECT id FROM questionnaires WHERE status = 'open' ORDER BY id")?;
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
        let answer = answer.trim();
        if answer.is_empty() {
            return Err(GenieError::invalid("the answer is empty"));
        }
        self.tx(|| {
            let qn = self.questionnaire(id)?;
            if qn.status != "open" {
                return Err(GenieError::invalid(format!("questionnaire {id} is {}", qn.status)));
            }
            let changed = self.conn().execute(
                "UPDATE questions SET answer = ?1, answered_at = ?2, answered_via = ?3 WHERE questionnaire = ?4 AND n = ?5",
                params![answer, now(), via, id, n],
            )?;
            if changed == 0 {
                return Err(GenieError::not_found(format!("questionnaire {id} has no question {n}")));
            }
            let open: i64 =
                self.conn().query_row("SELECT COUNT(*) FROM questions WHERE questionnaire = ?1 AND answer IS NULL", [id], |r| r.get(0))?;
            if open == 0 {
                self.conn().execute("UPDATE questionnaires SET status = 'answered', closed = ?1 WHERE id = ?2", params![now(), id])?;
            }
            Ok((self.questionnaire(id)?, open == 0))
        })
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
}
