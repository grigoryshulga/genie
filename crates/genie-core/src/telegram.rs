//! Project bots use sealed credentials and bot-specific channel links. Subscriptions
//! belong to people, independently of assignment and of their delivery channel.
use crate::secrets::{Secret, load_key, seal, unseal};
use crate::server_db::{ServerDb, hash_secret, new_secret};
use crate::{GenieError, Result};
use rusqlite::{OptionalExtension, params};

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS project_telegram (
 project TEXT PRIMARY KEY REFERENCES projects(slug) ON DELETE CASCADE,
 bot_id INTEGER NOT NULL UNIQUE,
 username TEXT NOT NULL,
 sealed TEXT NOT NULL,
 scope TEXT NOT NULL UNIQUE,
 offset INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS task_watches (
 project TEXT NOT NULL REFERENCES projects(slug) ON DELETE CASCADE,
 task TEXT NOT NULL,
 user INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 created TEXT NOT NULL,
 PRIMARY KEY(project, task, user)
);
"#;

#[derive(Clone, Debug)]
pub struct ProjectBot {
    pub project: String,
    pub id: i64,
    pub username: String,
    pub scope: String,
    pub token: Secret,
    pub offset: i64,
}

impl ServerDb {
    pub fn project_bot(&self, project: &str) -> Result<Option<ProjectBot>> {
        let row: Option<(i64, String, String, String, i64)> = self
            .conn()
            .query_row("SELECT bot_id, username, sealed, scope, offset FROM project_telegram WHERE project = ?1", [project], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .optional()?;
        let Some((id, username, sealed, scope, offset)) = row else { return Ok(None) };
        let token = load_key(&self.secrets_key, false)?.and_then(|key| unseal(&key, &scope, &sealed)).unwrap_or_default();
        Ok(Some(ProjectBot { project: project.into(), id, username, scope, token: Secret(token), offset }))
    }

    pub fn project_bots(&self) -> Result<Vec<ProjectBot>> {
        let mut stmt = self.conn().prepare("SELECT project FROM project_telegram")?;
        let projects = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        projects.iter().filter_map(|p| self.project_bot(p).transpose()).collect()
    }

    pub fn set_project_bot(&self, project: &str, id: i64, username: &str, token: &str) -> Result<()> {
        self.tx(|| {
            let other: Option<String> = self.conn().query_row("SELECT project FROM project_telegram WHERE bot_id = ?1 AND project != ?2", params![id, project], |r| r.get(0)).optional()?;
            if other.is_some() { return Err(GenieError::invalid("This bot already belongs to another project")); }
            let old = self.project_bot(project)?;
            // Credential rotation retains chats; changing bot identity requires new consent.
            let scope = if let Some(old) = old.filter(|b| b.id == id) { old.scope } else {
                self.remove_project_bot(project)?;
                format!("telegram:{}", new_secret())
            };
            let key = load_key(&self.secrets_key, true)?.ok_or_else(|| GenieError::invalid("Cannot load secrets key"))?;
            self.conn().execute("INSERT INTO project_telegram(project, bot_id, username, sealed, scope) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(project) DO UPDATE SET username=excluded.username, sealed=excluded.sealed", params![project, id, username, seal(&key, &scope, token)?, scope])?;
            Ok(())
        })
    }

    pub fn remove_project_bot(&self, project: &str) -> Result<()> {
        // Called within set_project_bot's transaction too (tx supports nesting).
        self.tx(|| {
            if let Some(bot) = self.project_bot(project)? {
                self.conn().execute("DELETE FROM channel_links WHERE channel = ?1", [&bot.scope])?;
                self.conn().execute("DELETE FROM link_codes WHERE channel = ?1", [&bot.scope])?;
                self.conn().execute("UPDATE outbox SET status = 'cancelled' WHERE channel = 'telegram' AND json_extract(payload, '$.telegramScope') = ?1 AND status = 'pending'", [&bot.scope])?;
                self.conn().execute("DELETE FROM project_telegram WHERE project = ?1", [project])?;
            }
            Ok(())
        })
    }

    pub fn bot_link_code(&self, bot: &ProjectBot, user: i64) -> Result<String> {
        self.tx(|| {
            let code = new_secret();
            self.conn().execute("DELETE FROM link_codes WHERE user = ?1 AND channel = ?2", params![user, bot.scope])?;
            self.conn().execute(
                "INSERT INTO link_codes(code, user, channel, expires) VALUES (?1, ?2, ?3, ?4)",
                params![hash_secret(&code), user, bot.scope, crate::server_db::time_in(chrono::Duration::minutes(30))],
            )?;
            Ok(code)
        })
    }

    pub fn redeem_bot_code(&self, scope: &str, code: &str, chat: &str) -> Result<bool> {
        self.tx(|| {
            let row: Option<(i64, String)> = self.conn().query_row("SELECT c.user, b.project FROM link_codes c JOIN project_telegram b ON b.scope = c.channel WHERE c.code = ?1 AND c.channel = ?2 AND c.expires > ?3", params![hash_secret(code), scope, crate::db::now()], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
            let Some((user, project)) = row else { return Ok(false) };
            let u = self.user(user)?;
            if u.disabled || self.project_role(&project, &u)?.is_none() { return Ok(false); }
            if self.user_by_channel(scope, chat)?.is_some_and(|id| id != user) { return Ok(false); }
            self.conn().execute("DELETE FROM link_codes WHERE code = ?1", [hash_secret(code)])?;
            self.link_channel(user, scope, chat)?;
            Ok(true)
        })
    }

    pub fn unwatch_bot(&self, scope: &str, user: i64) -> Result<()> {
        self.tx(|| {
            self.unlink_channel(user, scope)?;
            self.conn().execute("DELETE FROM link_codes WHERE user = ?1 AND channel = ?2", params![user, scope])?;
            self.conn().execute("UPDATE outbox SET status = 'cancelled' WHERE user = ?1 AND json_extract(payload, '$.telegramScope') = ?2 AND status = 'pending'", params![user, scope])?;
            Ok(())
        })
    }

    pub fn set_task_watch(&self, project: &str, task: &str, user: i64, watching: bool) -> Result<()> {
        if watching {
            self.conn().execute(
                "INSERT OR IGNORE INTO task_watches(project, task, user, created) VALUES (?1, ?2, ?3, ?4)",
                params![project, task, user, crate::db::now()],
            )?;
        } else {
            self.conn().execute("DELETE FROM task_watches WHERE project = ?1 AND task = ?2 AND user = ?3", params![project, task, user])?;
        }
        Ok(())
    }
    pub fn task_watchers_at(&self, project: &str, task: &str, at: &str) -> Result<Vec<i64>> {
        let mut stmt = self.conn().prepare("SELECT user FROM task_watches WHERE project = ?1 AND task = ?2 AND created <= ?3")?;
        Ok(stmt.query_map(params![project, task, at], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
    }
    pub fn task_watchers(&self, project: &str, task: &str) -> Result<Vec<i64>> {
        let mut stmt = self.conn().prepare("SELECT user FROM task_watches WHERE project = ?1 AND task = ?2")?;
        Ok(stmt.query_map(params![project, task], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_db::ProjectRole;
    fn setup() -> (tempfile::TempDir, ServerDb, i64) {
        let dir = tempfile::tempdir().unwrap();
        let db = ServerDb::open(&dir.path().join("server.db")).unwrap();
        db.create_project("one", "One", "/tmp/one", None, Some("one")).unwrap();
        db.create_project("two", "Two", "/tmp/two", None, Some("two")).unwrap();
        let u = db.create_user("anna", "Anna", None, None, false).unwrap();
        db.set_membership("one", u.id, ProjectRole::Member).unwrap();
        (dir, db, u.id)
    }
    #[test]
    fn codes_are_private_one_time_scoped_and_revocable() {
        let (_dir, db, user) = setup();
        db.set_project_bot("one", 1, "one_bot", "secret-one").unwrap();
        db.set_project_bot("two", 2, "two_bot", "secret-two").unwrap();
        let one = db.project_bot("one").unwrap().unwrap();
        let two = db.project_bot("two").unwrap().unwrap();
        let code = db.bot_link_code(&one, user).unwrap();
        assert!(!db.redeem_bot_code(&two.scope, &code, "42").unwrap());
        assert!(db.redeem_bot_code(&one.scope, &code, "42").unwrap());
        assert!(!db.redeem_bot_code(&one.scope, &code, "43").unwrap());
        assert_eq!(db.channel_address(user, &one.scope).unwrap().as_deref(), Some("42"));
        let code = db.bot_link_code(&one, user).unwrap();
        db.unwatch_bot(&one.scope, user).unwrap();
        assert!(!db.redeem_bot_code(&one.scope, &code, "42").unwrap());
        let code = db.bot_link_code(&one, user).unwrap();
        db.conn().execute("UPDATE link_codes SET expires = '2000-01-01'", []).unwrap();
        assert!(!db.redeem_bot_code(&one.scope, &code, "42").unwrap());
        let code = db.bot_link_code(&two, user).unwrap();
        assert!(!db.redeem_bot_code(&two.scope, &code, "42").unwrap(), "membership required at redemption");
    }
    #[test]
    fn secrets_are_sealed_rotation_preserves_chats_and_replacement_resets_them() {
        let (_dir, db, user) = setup();
        db.set_project_bot("one", 1, "one_bot", "secret-one").unwrap();
        let one = db.project_bot("one").unwrap().unwrap();
        let sealed: String = db.conn().query_row("SELECT sealed FROM project_telegram", [], |r| r.get(0)).unwrap();
        assert!(!sealed.contains("secret-one"));
        assert!(db.has_sealed_secrets().unwrap());
        db.link_channel(user, &one.scope, "42").unwrap();
        db.set_project_bot("one", 1, "renamed_bot", "rotated-secret").unwrap();
        assert_eq!(db.project_bot("one").unwrap().unwrap().token.0, "rotated-secret");
        assert!(db.channel_address(user, &one.scope).unwrap().is_some());
        assert!(db.set_project_bot("two", 1, "renamed_bot", "stolen").is_err());
        let code = db.bot_link_code(&one, user).unwrap();
        db.set_task_watch("one", "ONE-1", user, true).unwrap();
        db.set_project_bot("one", 3, "new_bot", "new-secret").unwrap();
        assert!(db.channel_address(user, &one.scope).unwrap().is_none());
        assert!(!db.redeem_bot_code(&one.scope, &code, "42").unwrap());
        assert_eq!(db.task_watchers("one", "ONE-1").unwrap(), vec![user]);
    }
    #[test]
    fn upgrades_existing_database_and_rejects_newer_versions() {
        let (dir, db, user) = setup();
        db.set_task_watch("one", "ONE-1", user, true).unwrap();
        db.conn().execute_batch("DROP TABLE project_telegram; UPDATE meta SET value = '4' WHERE key = 'schema';").unwrap();
        drop(db);
        let db = ServerDb::open(&dir.path().join("server.db")).unwrap();
        assert!(db.project_bot("one").unwrap().is_none());
        assert_eq!(db.task_watchers("one", "ONE-1").unwrap(), vec![user]);
        db.conn().execute("UPDATE meta SET value = '999' WHERE key = 'schema'", []).unwrap();
        drop(db);
        assert!(ServerDb::open(&dir.path().join("server.db")).is_err());
    }
}
