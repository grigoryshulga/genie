//! Connections of people to a git provider (`git_connections` of the server database).
//!
//! A connection is one installation of the provider's application (a GitHub App) that a
//! person completed and the server verified. It belongs to that person; a project reaches
//! it only through the repositories bound to it (`project_repos.connection`), so the
//! project's grant is exactly the list of those repositories.
//!
//! The row holds no credential: installation tokens are minted for one operation and live
//! in memory only (`genie::git::tokens`). This module keeps the facts and the state:
//! who owns the connection, what the provider last said about it, and whether operations on
//! it are allowed (`Connection::blocked`).

use rusqlite::{OptionalExtension, Row, params};
use serde::Serialize;

use crate::db::now;
use crate::error::{GenieError, Result};
use crate::server_db::ServerDb;

pub const SCHEMA: &str = r#"
-- One installation of a provider's application, completed by a person.
CREATE TABLE IF NOT EXISTS git_connections (
  id TEXT PRIMARY KEY,
  provider TEXT NOT NULL,
  host TEXT NOT NULL,
  owner INTEGER REFERENCES users(id) ON DELETE SET NULL,
  account_login TEXT NOT NULL,
  account_id INTEGER NOT NULL,
  account_type TEXT NOT NULL DEFAULT '',
  installation_id INTEGER NOT NULL,
  selection TEXT NOT NULL DEFAULT 'selected',
  permissions TEXT NOT NULL DEFAULT '{}',
  status TEXT NOT NULL DEFAULT 'ok',
  status_note TEXT NOT NULL DEFAULT '',
  checked TEXT NOT NULL DEFAULT '',
  created TEXT NOT NULL,
  updated TEXT NOT NULL,
  UNIQUE (provider, host, installation_id)
);
"#;

/// How a connection stands. `OwnerRemoved` is never stored: it is worked out from the owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    /// The provider accepted the last call.
    Ok,
    /// A call failed in a way that may pass (network, rate limit): operations still try.
    NeedsAttention,
    /// The installation was removed or the person disconnected it.
    Revoked,
    /// The owner of the account suspended the installation.
    Suspended,
    /// The installation lacks a permission an operation needs.
    PermissionMissing,
    /// The person who owns the connection is gone or disabled; an administrator reassigns it.
    OwnerRemoved,
}

impl Health {
    pub fn as_str(self) -> &'static str {
        match self {
            Health::Ok => "ok",
            Health::NeedsAttention => "needs_attention",
            Health::Revoked => "revoked",
            Health::Suspended => "suspended",
            Health::PermissionMissing => "permission_missing",
            Health::OwnerRemoved => "owner_removed",
        }
    }

    fn parse(s: &str) -> Health {
        match s {
            "needs_attention" => Health::NeedsAttention,
            "revoked" => Health::Revoked,
            "suspended" => Health::Suspended,
            "permission_missing" => Health::PermissionMissing,
            "owner_removed" => Health::OwnerRemoved,
            _ => Health::Ok,
        }
    }

    /// States in which the server makes no new calls with the connection.
    pub fn blocks(self) -> bool {
        matches!(self, Health::Revoked | Health::Suspended | Health::OwnerRemoved)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub provider: String,
    /// The host of `git.json` the installation lives on.
    pub host: String,
    /// The person who owns it; `None` once that person is deleted.
    pub owner: Option<i64>,
    pub owner_login: Option<String>,
    pub account_login: String,
    pub account_id: i64,
    pub account_type: String,
    pub installation_id: i64,
    /// `all` or `selected`: what the installer chose on the provider.
    pub selection: String,
    pub permissions: serde_json::Value,
    pub health: Health,
    pub status_note: String,
    pub checked: String,
    pub created: String,
    pub updated: String,
    /// How many repositories of projects use it.
    pub repos: i64,
}

impl Connection {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<Connection> {
        let stored = Health::parse(&r.get::<_, String>("status")?);
        let owner: Option<i64> = r.get("owner")?;
        let owner_disabled: Option<i64> = r.get("owner_disabled")?;
        let health = if owner.is_none() || owner_disabled == Some(1) { Health::OwnerRemoved } else { stored };
        let permissions: String = r.get("permissions")?;
        Ok(Connection {
            id: r.get("id")?,
            provider: r.get("provider")?,
            host: r.get("host")?,
            owner,
            owner_login: r.get("owner_login")?,
            account_login: r.get("account_login")?,
            account_id: r.get("account_id")?,
            account_type: r.get("account_type")?,
            installation_id: r.get("installation_id")?,
            selection: r.get("selection")?,
            permissions: serde_json::from_str(&permissions).unwrap_or_default(),
            health,
            status_note: r.get("status_note")?,
            checked: r.get("checked")?,
            created: r.get("created")?,
            updated: r.get("updated")?,
            repos: r.get("repos")?,
        })
    }

    /// Why operations on the connection are refused, when they are.
    pub fn blocked(&self) -> Option<String> {
        if !self.health.blocks() {
            return None;
        }
        let why = match self.health {
            Health::OwnerRemoved => "its owner was removed or disabled; an administrator must reassign the connection".to_string(),
            Health::Revoked => format!("it was revoked ({}); connect the account again", self.status_note),
            Health::Suspended => "the installation is suspended on the provider".to_string(),
            _ => String::new(),
        };
        Some(format!("the connection to {} ({}) cannot be used: {why}", self.account_login, self.provider))
    }
}

/// A verified installation on its way in.
#[derive(Debug, Clone)]
pub struct NewConnection {
    pub provider: String,
    pub host: String,
    pub owner: i64,
    pub account_login: String,
    pub account_id: i64,
    pub account_type: String,
    pub installation_id: i64,
    pub selection: String,
    pub permissions: serde_json::Value,
}

const SELECT: &str = "SELECT c.*, u.login AS owner_login, u.disabled AS owner_disabled,
       (SELECT COUNT(*) FROM project_repos p WHERE p.connection = c.id) AS repos
  FROM git_connections c LEFT JOIN users u ON u.id = c.owner";

fn new_id() -> String {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).expect("OS random source");
    format!("gc_{}", hex::encode(b))
}

impl ServerDb {
    /// Record a verified installation. A new one is stored for `owner`; one that already
    /// exists is taken over only by its own owner (an administrator reassigns otherwise).
    pub fn upsert_connection(&self, new: NewConnection) -> Result<Connection> {
        let existing: Option<(String, Option<i64>)> = self
            .conn()
            .query_row(
                "SELECT id, owner FROM git_connections WHERE provider = ?1 AND host = ?2 AND installation_id = ?3",
                params![new.provider, new.host, new.installation_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let id = match existing {
            Some((id, owner)) => {
                if owner.is_some_and(|o| o != new.owner) {
                    return Err(GenieError::invalid(
                        "this installation is already connected by another person; an administrator can reassign it",
                    ));
                }
                self.conn().execute(
                    "UPDATE git_connections SET owner = ?1, account_login = ?2, account_id = ?3, account_type = ?4, selection = ?5,
                            permissions = ?6, status = 'ok', status_note = '', checked = ?7, updated = ?7 WHERE id = ?8",
                    params![
                        new.owner,
                        new.account_login,
                        new.account_id,
                        new.account_type,
                        new.selection,
                        new.permissions.to_string(),
                        now(),
                        id
                    ],
                )?;
                id
            }
            None => {
                let id = new_id();
                self.conn().execute(
                    "INSERT INTO git_connections(id, provider, host, owner, account_login, account_id, account_type, installation_id,
                                                 selection, permissions, status, checked, created, updated)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'ok', ?11, ?11, ?11)",
                    params![
                        id,
                        new.provider,
                        new.host,
                        new.owner,
                        new.account_login,
                        new.account_id,
                        new.account_type,
                        new.installation_id,
                        new.selection,
                        new.permissions.to_string(),
                        now()
                    ],
                )?;
                id
            }
        };
        self.connection(&id)
    }

    pub fn connection_opt(&self, id: &str) -> Result<Option<Connection>> {
        Ok(self.conn().query_row(&format!("{SELECT} WHERE c.id = ?1"), [id], Connection::from_row).optional()?)
    }

    pub fn connection(&self, id: &str) -> Result<Connection> {
        self.connection_opt(id)?.ok_or_else(|| GenieError::not_found(format!("no connection {id}")))
    }

    /// Every connection, or only those of one person.
    pub fn connections(&self, owner: Option<i64>) -> Result<Vec<Connection>> {
        let mut stmt = self.conn().prepare(&format!("{SELECT} WHERE (?1 IS NULL OR c.owner = ?1) ORDER BY c.account_login, c.id"))?;
        Ok(stmt.query_map([owner], Connection::from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    /// What the provider last said about the connection (`Health::OwnerRemoved` is not stored).
    pub fn set_connection_health(&self, id: &str, health: Health, note: &str) -> Result<()> {
        if health == Health::OwnerRemoved {
            return Err(GenieError::invalid("owner_removed follows from the owner and is not set"));
        }
        let n = self.conn().execute(
            "UPDATE git_connections SET status = ?1, status_note = ?2, checked = ?3, updated = ?3 WHERE id = ?4",
            params![health.as_str(), note.chars().take(300).collect::<String>(), now(), id],
        )?;
        if n == 0 {
            return Err(GenieError::not_found(format!("no connection {id}")));
        }
        Ok(())
    }

    /// Refresh the facts the provider reported about an installation (its account, selection, permissions).
    pub fn touch_connection(&self, id: &str, selection: &str, permissions: &serde_json::Value) -> Result<()> {
        self.conn().execute(
            "UPDATE git_connections SET selection = ?1, permissions = ?2, checked = ?3 WHERE id = ?4",
            params![selection, permissions.to_string(), now(), id],
        )?;
        Ok(())
    }

    /// Give a connection to another active person (an administrator's action).
    pub fn reassign_connection(&self, id: &str, new_owner: i64) -> Result<Connection> {
        let user = self.user(new_owner)?;
        if user.disabled {
            return Err(GenieError::invalid("the new owner is disabled"));
        }
        self.connection(id)?;
        self.conn().execute("UPDATE git_connections SET owner = ?1, updated = ?2 WHERE id = ?3", params![new_owner, now(), id])?;
        self.connection(id)
    }

    /// Disconnect: operations stop at once, while the repositories, deliveries and links to
    /// requests stay as they are. Connecting the same installation again revives it.
    pub fn revoke_connection(&self, id: &str, note: &str) -> Result<()> {
        self.set_connection_health(id, Health::Revoked, note)
    }

    /// May this person connect an account: server administrators and administrators of some project.
    pub fn administers_a_project(&self, user: &crate::server_db::User) -> Result<bool> {
        if user.is_admin {
            return Ok(true);
        }
        let n: i64 =
            self.conn()
                .query_row("SELECT COUNT(*) FROM memberships WHERE user = ?1 AND role IN ('admin', 'owner')", [user.id], |r| r.get(0))?;
        Ok(n > 0)
    }

    /// The projects whose repositories use a connection, with those repositories.
    pub fn connection_repos(&self, id: &str) -> Result<Vec<(String, String, i64)>> {
        let mut stmt = self.conn().prepare(
            "SELECT project, name, COALESCE(provider_repo_id, 0) FROM project_repos WHERE connection = ?1 ORDER BY project, name",
        )?;
        Ok(stmt.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?)
    }

    /// The connection a repository is bound to.
    pub fn connection_of(&self, project: &str, repo: &str) -> Result<Option<Connection>> {
        let id: Option<Option<String>> = self
            .conn()
            .query_row("SELECT connection FROM project_repos WHERE project = ?1 AND name = ?2", params![project, repo], |r| r.get(0))
            .optional()?;
        match id.flatten() {
            Some(id) => self.connection_opt(&id),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_db::ServerDb;

    fn db() -> (tempfile::TempDir, ServerDb) {
        let dir = tempfile::tempdir().unwrap();
        let db = ServerDb::open(&dir.path().join("server.db")).unwrap();
        (dir, db)
    }

    fn user(db: &ServerDb, login: &str) -> i64 {
        db.create_user(login, login, None, Some("a-long-enough-password"), false).unwrap().id
    }

    fn installation(owner: i64) -> NewConnection {
        NewConnection {
            provider: "github".into(),
            host: "github".into(),
            owner,
            account_login: "acme".into(),
            account_id: 7,
            account_type: "Organization".into(),
            installation_id: 42,
            selection: "selected".into(),
            permissions: serde_json::json!({"contents": "write"}),
        }
    }

    #[test]
    fn a_connection_belongs_to_its_owner_and_is_taken_over_only_by_them() {
        let (_d, db) = db();
        let (ann, bob) = (user(&db, "ann"), user(&db, "bob"));
        let c = db.upsert_connection(installation(ann)).unwrap();
        assert_eq!((c.owner, c.health, c.repos), (Some(ann), Health::Ok, 0));
        assert_eq!(c.owner_login.as_deref(), Some("ann"));
        // The same person connecting again keeps one row.
        let again = db.upsert_connection(installation(ann)).unwrap();
        assert_eq!(again.id, c.id);
        assert_eq!(db.connections(None).unwrap().len(), 1);
        // Another person cannot take it over; an administrator reassigns it.
        assert!(db.upsert_connection(installation(bob)).is_err());
        assert_eq!(db.reassign_connection(&c.id, bob).unwrap().owner, Some(bob));
        assert_eq!(db.connections(Some(ann)).unwrap().len(), 0);
        assert_eq!(db.connections(Some(bob)).unwrap().len(), 1);
    }

    #[test]
    fn operations_stop_when_the_owner_is_removed_or_disabled_until_it_is_reassigned() {
        let (_d, db) = db();
        let (ann, bob) = (user(&db, "ann"), user(&db, "bob"));
        let c = db.upsert_connection(installation(ann)).unwrap();
        assert!(c.blocked().is_none());

        db.delete_user(ann).unwrap();
        let c = db.connection(&c.id).unwrap();
        assert_eq!((c.owner, c.health), (None, Health::OwnerRemoved));
        assert!(c.blocked().unwrap().contains("reassign"));

        let c = db.reassign_connection(&c.id, bob).unwrap();
        assert_eq!(c.health, Health::Ok);

        db.update_user(bob, None, None, None, Some(true)).unwrap();
        assert_eq!(db.connection(&c.id).unwrap().health, Health::OwnerRemoved);
    }

    #[test]
    fn revoking_blocks_and_connecting_again_revives() {
        let (_d, db) = db();
        let ann = user(&db, "ann");
        let c = db.upsert_connection(installation(ann)).unwrap();
        db.revoke_connection(&c.id, "disconnected by ann").unwrap();
        let revoked = db.connection(&c.id).unwrap();
        assert_eq!(revoked.health, Health::Revoked);
        assert!(revoked.blocked().unwrap().contains("disconnected by ann"));
        assert_eq!(db.upsert_connection(installation(ann)).unwrap().health, Health::Ok);
        // A transient problem does not block.
        db.set_connection_health(&c.id, Health::NeedsAttention, "timeout").unwrap();
        assert!(db.connection(&c.id).unwrap().blocked().is_none());
        assert!(db.set_connection_health(&c.id, Health::OwnerRemoved, "").is_err());
    }
}
