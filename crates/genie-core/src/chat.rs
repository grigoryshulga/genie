//! Project chat: the messages people and agents exchange in one project, kept in its tracker
//! (`chat_messages`). A post and its `chat.posted` journal event commit in one transaction.
//! Who may post, and what a mention wakes, is decided by the server; this module stores,
//! parses and pages.

use rusqlite::types::Type;
use rusqlite::{OptionalExtension, Row, params};
use serde::Serialize;
use serde_json::json;

use crate::db::now;
use crate::error::{GenieError, Result};
use crate::events;
use crate::model::Role;
use crate::server_db::valid_login;
use crate::tracker::Tracker;

/// Longest message, in characters.
pub const MAX_TEXT_CHARS: usize = 10_000;
/// Default and largest page of [`Chat::list`].
pub const DEFAULT_LIMIT: i64 = 50;
pub const MAX_LIMIT: i64 = 200;

/// The mention target of the project manager, whatever spelling the text used.
pub const MANAGER: &str = "manager";

const SELECT: &str = "SELECT id, at, author, author_role, text, mentions, reply_to FROM chat_messages";

/// A chat message as the API and the web show it.
#[derive(Debug, Clone, PartialEq, Serialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: i64,
    pub at: String,
    pub author: String,
    pub author_role: Role,
    pub text: String,
    /// Mention targets in order of first appearance: `manager` for the project manager, a login otherwise.
    pub mentions: Vec<String>,
    /// The message this one answers.
    pub reply_to: Option<i64>,
}

/// The mention targets of a text. `@pm`, `@manager` and `@менеджер` (any case) name the project
/// manager and come back as `manager`; `@login` names that login, lower-cased. A mention starts
/// a word, so an e-mail address is not one. Each target appears once.
pub fn mentions(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = Vec::new();
    for (i, c) in chars.iter().enumerate() {
        if *c != '@' || chars[..i].last().is_some_and(|p| is_name_char(*p)) {
            continue;
        }
        let token: String = chars[i + 1..].iter().take_while(|c| is_name_char(**c)).collect();
        let token = token.trim_end_matches('.').to_lowercase();
        let target = match token.as_str() {
            "pm" | "manager" | "менеджер" => MANAGER.to_string(),
            other => match valid_login(other) {
                Ok(login) => login,
                Err(_) => continue,
            },
        };
        if !out.contains(&target) {
            out.push(target);
        }
    }
    out
}

/// Letters, digits and the punctuation a login or a mention can carry.
fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// Chat operations of one tracker, reached through [`Tracker::chat`].
pub struct Chat<'a>(&'a Tracker);

impl Tracker {
    pub fn chat(&self) -> Chat<'_> {
        Chat(self)
    }
}

impl Chat<'_> {
    /// Stores a message and journals `chat.posted` (`{id, mentions}`) in one transaction.
    /// `reply_to`, when given, must name an existing message.
    pub fn post(&self, author: &str, role: Role, text: &str, reply_to: Option<i64>) -> Result<ChatMessage> {
        let author = author.trim();
        let text = text.trim();
        if author.is_empty() {
            return Err(GenieError::invalid("a chat message needs an author"));
        }
        if text.is_empty() {
            return Err(GenieError::invalid("a chat message needs text"));
        }
        if text.chars().count() > MAX_TEXT_CHARS {
            return Err(GenieError::invalid(format!("a chat message is at most {MAX_TEXT_CHARS} characters")));
        }
        let mentions = mentions(text);
        let mentions_json = serde_json::to_string(&mentions)?;
        let conn = self.0.conn();
        self.0.tx(|| {
            if let Some(parent) = reply_to {
                let exists = conn.query_row("SELECT 1 FROM chat_messages WHERE id = ?1", [parent], |_| Ok(())).optional()?.is_some();
                if !exists {
                    return Err(GenieError::invalid(format!("reply_to names chat message #{parent}, which does not exist")));
                }
            }
            conn.execute(
                "INSERT INTO chat_messages(at, author, author_role, text, mentions, reply_to) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![now(), author, role, text, mentions_json, reply_to],
            )?;
            let id = conn.last_insert_rowid();
            self.0.append_event(events::CHAT_POSTED, None, author, role.as_str(), json!({ "id": id, "mentions": mentions }))?;
            self.get(id)
        })
    }

    /// The message with `id`; an unknown id is a not-found error.
    pub fn get(&self, id: i64) -> Result<ChatMessage> {
        self.0
            .conn()
            .query_row(&format!("{SELECT} WHERE id = ?1"), [id], message)
            .optional()?
            .ok_or_else(|| GenieError::not_found(format!("chat message #{id} not found")))
    }

    /// When the newest message that mentions `target` was written (RFC 3339), if any.
    pub fn last_mention_at(&self, target: &str) -> Result<Option<String>> {
        // A mention is stored as a JSON string in `mentions`, so the quoted name matches it exactly.
        Ok(self
            .0
            .conn()
            .query_row("SELECT at FROM chat_messages WHERE mentions LIKE ?1 ORDER BY id DESC LIMIT 1", [format!("%\"{target}\"%")], |r| {
                r.get(0)
            })
            .optional()?)
    }

    /// One page of the newest messages older than `before` (the newest of all when `None`),
    /// oldest first. `limit` defaults to [`DEFAULT_LIMIT`] and must be between 1 and [`MAX_LIMIT`].
    pub fn list(&self, before: Option<i64>, limit: Option<i64>) -> Result<Vec<ChatMessage>> {
        let limit = limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(GenieError::invalid(format!("limit must be between 1 and {MAX_LIMIT}")));
        }
        let mut stmt = self.0.conn().prepare_cached(&format!("{SELECT} WHERE id < ?1 ORDER BY id DESC LIMIT ?2"))?;
        let mut page = stmt.query_map(params![before.unwrap_or(i64::MAX), limit], message)?.collect::<rusqlite::Result<Vec<_>>>()?;
        page.reverse();
        Ok(page)
    }
}

/// A row of [`SELECT`].
fn message(r: &Row<'_>) -> rusqlite::Result<ChatMessage> {
    let raw: String = r.get(5)?;
    let mentions: Vec<String> =
        serde_json::from_str(&raw).map_err(|e| rusqlite::Error::FromSqlConversionFailure(5, Type::Text, Box::new(e)))?;
    Ok(ChatMessage {
        id: r.get(0)?,
        at: r.get(1)?,
        author: r.get(2)?,
        author_role: r.get(3)?,
        text: r.get(4)?,
        mentions,
        reply_to: r.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SCHEMA_VERSION;
    use crate::events::Event;
    use crate::migrate;
    use crate::model::Actor;
    use crate::tracker::CreateInput;

    fn fresh() -> (tempfile::TempDir, Tracker) {
        let dir = tempfile::tempdir().unwrap();
        let t = Tracker::init(dir.path().join(".genie"), None, None).unwrap();
        (dir, t)
    }

    fn post_text(t: &Tracker, text: &str) -> ChatMessage {
        t.chat().post("alice", Role::Human, text, None).unwrap()
    }

    fn chat_events(t: &Tracker) -> Vec<Event> {
        t.events_after(0, 1_000).unwrap().into_iter().filter(|e| e.kind == events::CHAT_POSTED).collect()
    }

    fn texts(messages: &[ChatMessage]) -> Vec<&str> {
        messages.iter().map(|m| m.text.as_str()).collect()
    }

    #[test]
    fn post_stores_the_message_and_journals_it_with_its_mentions() {
        let (_dir, t) = fresh();
        let m = t.chat().post("alice", Role::Human, "  ping @pm and @bob  ", None).unwrap();
        assert_eq!(m.id, 1);
        assert_eq!(m.text, "ping @pm and @bob", "the text is stored trimmed");
        assert_eq!(m.author, "alice");
        assert_eq!(m.author_role, Role::Human);
        assert_eq!(m.mentions, ["manager", "bob"]);
        assert_eq!(m.reply_to, None);
        assert_eq!(t.chat().get(1).unwrap(), m);

        let journal = chat_events(&t);
        assert_eq!(journal.len(), 1, "one chat.posted per message");
        assert_eq!(journal[0].actor, "alice");
        assert_eq!(journal[0].actor_role, "human");
        assert_eq!(journal[0].payload, json!({ "id": 1, "mentions": ["manager", "bob"] }));
    }

    #[test]
    fn a_rejected_post_writes_neither_a_message_nor_an_event() {
        let (_dir, t) = fresh();
        let missing = t.chat().post("alice", Role::Human, "hi", Some(42)).unwrap_err();
        assert!(matches!(missing, GenieError::Invalid(_)), "{missing}");
        assert!(matches!(t.chat().post("alice", Role::Human, "   ", None), Err(GenieError::Invalid(_))));
        assert!(matches!(t.chat().post(" ", Role::Human, "hi", None), Err(GenieError::Invalid(_))));
        let long = "x".repeat(MAX_TEXT_CHARS + 1);
        assert!(matches!(t.chat().post("alice", Role::Human, &long, None), Err(GenieError::Invalid(_))));

        assert!(t.chat().list(None, None).unwrap().is_empty());
        assert!(chat_events(&t).is_empty());
    }

    #[test]
    fn reply_to_names_an_existing_message() {
        let (_dir, t) = fresh();
        let root = post_text(&t, "is the build green?");
        let answer = t.chat().post("project-manager", Role::Orchestrator, "yes", Some(root.id)).unwrap();
        assert_eq!(answer.reply_to, Some(root.id));
        assert_eq!(answer.author_role, Role::Orchestrator);
        assert_eq!(t.chat().get(answer.id).unwrap().reply_to, Some(root.id));
    }

    #[test]
    fn list_pages_back_from_the_newest_message_and_returns_oldest_first() {
        let (_dir, t) = fresh();
        for text in ["m1", "m2", "m3", "m4", "m5"] {
            post_text(&t, text);
        }
        let newest = t.chat().list(None, Some(2)).unwrap();
        assert_eq!(texts(&newest), ["m4", "m5"]);
        let older = t.chat().list(Some(newest[0].id), Some(2)).unwrap();
        assert_eq!(texts(&older), ["m2", "m3"]);
        let oldest = t.chat().list(Some(older[0].id), Some(2)).unwrap();
        assert_eq!(texts(&oldest), ["m1"]);
        assert!(t.chat().list(Some(oldest[0].id), Some(2)).unwrap().is_empty(), "nothing is older than the first");
        assert_eq!(texts(&t.chat().list(None, None).unwrap()), ["m1", "m2", "m3", "m4", "m5"], "the default page holds them all");
    }

    #[test]
    fn list_refuses_a_page_size_out_of_range_and_get_names_a_missing_message() {
        let (_dir, t) = fresh();
        post_text(&t, "hi");
        assert!(matches!(t.chat().list(None, Some(0)), Err(GenieError::Invalid(_))));
        assert!(matches!(t.chat().list(None, Some(MAX_LIMIT + 1)), Err(GenieError::Invalid(_))));
        assert!(matches!(t.chat().get(99), Err(GenieError::NotFound(_))));
    }

    #[test]
    fn the_last_mention_of_a_target_is_found_by_its_exact_name() {
        let (_dir, t) = fresh();
        assert_eq!(t.chat().last_mention_at(MANAGER).unwrap(), None);
        let first = t.chat().post("alice", Role::Human, "@pm please", None).unwrap();
        t.chat().post("alice", Role::Human, "@manager2 and @bob", None).unwrap();
        assert_eq!(t.chat().last_mention_at(MANAGER).unwrap(), Some(first.at.clone()));
        assert!(t.chat().last_mention_at("bob").unwrap().is_some());
        assert!(t.chat().last_mention_at("manager2").unwrap().is_some());
        assert_eq!(t.chat().last_mention_at("ma").unwrap(), None, "a part of a name is not a mention");
    }

    #[test]
    fn mentions_name_the_manager_in_any_spelling_and_logins_otherwise() {
        assert_eq!(mentions("@PM, @manager, @Менеджер!"), ["manager"]);
        assert_eq!(mentions("@Alice.Smith @bob-2 @bob-2."), ["alice.smith", "bob-2"]);
        assert_eq!(mentions("@pm @manager @PM"), ["manager"], "each target once");
        assert_eq!(mentions("@pmx @manager2 @менеджерам"), ["pmx", "manager2"], "a mention is the whole word");
        assert_eq!(mentions("write ivan@pm.example or ivan.petrov@manager.dev"), Vec::<String>::new(), "e-mail addresses are not mentions");
        assert_eq!(mentions("a @ b, @@pm"), ["manager"]);
    }

    #[test]
    fn a_schema_8_tracker_upgrades_to_schema_9_with_the_chat_table() {
        assert_eq!(SCHEMA_VERSION, 9);
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(".genie");
        let t = Tracker::init(&root, None, None).unwrap();
        let task = t.create(&Actor::new("alice", Role::Human), CreateInput { title: "Legacy".into(), ..Default::default() }).unwrap().id;
        // Shape a schema-8 tracker: no chat table and the old version recorded.
        t.conn().execute_batch("DROP TABLE chat_messages;").unwrap();
        t.conn().execute("UPDATE meta SET value = '8' WHERE key = 'schema'", []).unwrap();
        drop(t);
        let file = root.join("genie.db");
        assert_eq!(migrate::version(&file).unwrap(), Some(8));

        // The rehearsal runs the upgrade on a copy and reports it; the live file stays at 8.
        let report = migrate::check_tracker(&file).unwrap();
        assert_eq!((report.from, report.to), (8, SCHEMA_VERSION));
        assert!(report.rehearsed);
        assert_eq!(migrate::version(&file).unwrap(), Some(8), "checking does not write");

        let t = Tracker::open(&root).unwrap();
        assert_eq!(migrate::version(&file).unwrap(), Some(SCHEMA_VERSION), "the upgrade records schema 9");
        assert_eq!(t.get(&task).unwrap().title, "Legacy", "existing tasks survive the upgrade");
        let posted = t.chat().post("alice", Role::Human, "after the upgrade", None).unwrap();
        assert_eq!(posted.id, 1);
        drop(t);

        // A tracker recording a newer schema is refused and left as it is.
        let conn = rusqlite::Connection::open(&file).unwrap();
        conn.execute("UPDATE meta SET value = ?1 WHERE key = 'schema'", [SCHEMA_VERSION + 1]).unwrap();
        drop(conn);
        let err = Tracker::open(&root).err().expect("a newer schema is refused").to_string();
        assert!(err.contains(&format!("records schema {}", SCHEMA_VERSION + 1)), "{err}");
        assert_eq!(migrate::version(&file).unwrap(), Some(SCHEMA_VERSION + 1), "the newer database is left as it was");
    }
}
