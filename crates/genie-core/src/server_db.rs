//! Server database (`<data>/server.db`): everything that spans projects —
//! users, sessions, tokens, invites, projects and memberships, channel links,
//! notifications and the delivery outbox, automations and their runs, agent
//! turns and jobs, questionnaires and knowledge proposals.
//!
//! Task data stays in one tracker database per project (see `tracker`), which
//! keeps projects isolated and portable.
//!
//! Secrets are never stored: sessions, API tokens, invites and answer links are
//! kept as SHA-256 hashes; passwords as Argon2id hashes. The one exception is a
//! person's own keys for their agents, which must be read back: they are sealed
//! (see `secrets`).

use std::path::Path;

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db::{Db, now};
use crate::error::{GenieError, Result};
use crate::model::Role;

pub const SERVER_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS users (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  login TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  email TEXT,
  password_hash TEXT,
  is_admin INTEGER NOT NULL DEFAULT 0,
  disabled INTEGER NOT NULL DEFAULT 0,
  created TEXT NOT NULL,
  avatar BLOB,
  avatar_type TEXT,
  avatar_at TEXT
);
CREATE TABLE IF NOT EXISTS sessions (
  token_hash TEXT PRIMARY KEY,
  user INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created TEXT NOT NULL,
  expires TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS invites (
  token_hash TEXT PRIMARY KEY,
  created_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
  project TEXT,
  project_role TEXT,
  email TEXT,
  expires TEXT NOT NULL,
  used_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
  used_at TEXT
);
CREATE TABLE IF NOT EXISTS api_tokens (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  token_hash TEXT NOT NULL UNIQUE,
  kind TEXT NOT NULL,
  user INTEGER REFERENCES users(id) ON DELETE CASCADE,
  project TEXT,
  agent_role TEXT,
  role_id TEXT,
  team TEXT,
  member TEXT,
  job INTEGER,
  label TEXT NOT NULL DEFAULT '',
  created TEXT NOT NULL,
  expires TEXT,
  revoked INTEGER NOT NULL DEFAULT 0,
  last_used TEXT
);
CREATE TABLE IF NOT EXISTS projects (
  slug TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  tracker_dir TEXT NOT NULL,
  repo TEXT,
  space TEXT NOT NULL,
  autonomy TEXT NOT NULL DEFAULT 'autonomous',
  integration TEXT NOT NULL DEFAULT '',
  created TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS memberships (
  project TEXT NOT NULL REFERENCES projects(slug) ON DELETE CASCADE,
  user INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role TEXT NOT NULL,
  PRIMARY KEY (project, user)
);
CREATE TABLE IF NOT EXISTS channel_links (
  user INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  channel TEXT NOT NULL,
  address TEXT NOT NULL,
  created TEXT NOT NULL,
  PRIMARY KEY (user, channel)
);
CREATE TABLE IF NOT EXISTS link_codes (
  code TEXT PRIMARY KEY,
  user INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  channel TEXT NOT NULL,
  expires TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS notifications (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  user INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  project TEXT,
  task TEXT,
  kind TEXT NOT NULL,
  title TEXT NOT NULL,
  body TEXT NOT NULL DEFAULT '',
  link TEXT,
  dedupe_key TEXT UNIQUE,
  created TEXT NOT NULL,
  read_at TEXT
);
CREATE INDEX IF NOT EXISTS notifications_user ON notifications(user, read_at);
CREATE TABLE IF NOT EXISTS outbox (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  dedupe_key TEXT UNIQUE,
  user INTEGER REFERENCES users(id) ON DELETE CASCADE,
  channel TEXT NOT NULL,
  address TEXT NOT NULL,
  subject TEXT NOT NULL DEFAULT '',
  body TEXT NOT NULL,
  payload TEXT NOT NULL DEFAULT '{}',
  status TEXT NOT NULL DEFAULT 'pending',
  attempts INTEGER NOT NULL DEFAULT 0,
  next_at TEXT NOT NULL,
  last_error TEXT,
  external_ref TEXT,
  created TEXT NOT NULL,
  sent_at TEXT
);
CREATE INDEX IF NOT EXISTS outbox_due ON outbox(status, next_at);
CREATE TABLE IF NOT EXISTS automations (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project TEXT NOT NULL REFERENCES projects(slug) ON DELETE CASCADE,
  name TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  dry_run INTEGER NOT NULL DEFAULT 0,
  version INTEGER NOT NULL DEFAULT 1,
  spec TEXT NOT NULL,
  created_by TEXT NOT NULL,
  created TEXT NOT NULL,
  updated TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS automation_runs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  automation INTEGER NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
  version INTEGER NOT NULL,
  project TEXT NOT NULL,
  trigger_key TEXT NOT NULL,
  trigger TEXT NOT NULL,
  depth INTEGER NOT NULL DEFAULT 0,
  status TEXT NOT NULL,
  started TEXT NOT NULL,
  finished TEXT,
  error TEXT,
  UNIQUE (automation, trigger_key)
);
CREATE INDEX IF NOT EXISTS automation_runs_status ON automation_runs(status);
CREATE TABLE IF NOT EXISTS run_steps (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  run INTEGER NOT NULL REFERENCES automation_runs(id) ON DELETE CASCADE,
  idx INTEGER NOT NULL,
  step_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  status TEXT NOT NULL,
  attempt INTEGER NOT NULL DEFAULT 0,
  input TEXT NOT NULL DEFAULT '{}',
  output TEXT,
  wait TEXT,
  started TEXT,
  finished TEXT,
  error TEXT,
  UNIQUE (run, idx)
);
CREATE TABLE IF NOT EXISTS agent_jobs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project TEXT NOT NULL,
  task TEXT,
  run_step INTEGER REFERENCES run_steps(id) ON DELETE SET NULL,
  role TEXT NOT NULL,
  model TEXT,
  goal TEXT NOT NULL,
  inputs TEXT NOT NULL DEFAULT '{}',
  output_schema TEXT,
  workspace TEXT NOT NULL DEFAULT 'none',
  status TEXT NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  output TEXT,
  error TEXT,
  created TEXT NOT NULL,
  finished TEXT,
  initiator TEXT
);
CREATE TABLE IF NOT EXISTS turns (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project TEXT NOT NULL,
  agent TEXT NOT NULL,
  team TEXT,
  member TEXT,
  job INTEGER,
  mail TEXT NOT NULL DEFAULT '[]',
  status TEXT NOT NULL,
  pid INTEGER,
  started TEXT NOT NULL,
  finished TEXT,
  exit_code INTEGER,
  error TEXT,
  log TEXT
);
CREATE INDEX IF NOT EXISTS turns_status ON turns(status);
CREATE INDEX IF NOT EXISTS turns_agent ON turns(project, agent, id);
CREATE TABLE IF NOT EXISTS questionnaires (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project TEXT NOT NULL,
  task TEXT,
  asked_by TEXT NOT NULL,
  recipient INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  channel TEXT NOT NULL,
  status TEXT NOT NULL,
  token_hash TEXT NOT NULL UNIQUE,
  run_step INTEGER REFERENCES run_steps(id) ON DELETE SET NULL,
  remind_at TEXT,
  due TEXT,
  created TEXT NOT NULL,
  closed TEXT
);
CREATE TABLE IF NOT EXISTS questions (
  questionnaire INTEGER NOT NULL REFERENCES questionnaires(id) ON DELETE CASCADE,
  n INTEGER NOT NULL,
  text TEXT NOT NULL,
  why TEXT NOT NULL DEFAULT '',
  options TEXT NOT NULL DEFAULT '[]',
  answer TEXT,
  answered_at TEXT,
  answered_via TEXT,
  message_ref TEXT,
  PRIMARY KEY (questionnaire, n)
);
CREATE TABLE IF NOT EXISTS proposals (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  path TEXT NOT NULL,
  base_hash TEXT,
  content TEXT NOT NULL,
  author TEXT NOT NULL,
  author_kind TEXT NOT NULL,
  project TEXT,
  task TEXT,
  note TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL,
  created TEXT NOT NULL,
  decided_by TEXT,
  decided_at TEXT,
  decision_note TEXT
);
CREATE TABLE IF NOT EXISTS locks (
  name TEXT PRIMARY KEY,
  holder TEXT NOT NULL,
  expires TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS config_changes (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  at TEXT NOT NULL,
  user TEXT NOT NULL,
  item TEXT NOT NULL,
  path TEXT NOT NULL,
  before TEXT,
  after TEXT
);
CREATE INDEX IF NOT EXISTS config_changes_item ON config_changes(item, id);
-- Repositories of a project (docs/platform/git-repositories.md).
CREATE TABLE IF NOT EXISTS project_repos (
  project TEXT NOT NULL REFERENCES projects(slug) ON DELETE CASCADE,
  name TEXT NOT NULL,
  host TEXT NOT NULL,
  remote TEXT NOT NULL,
  mount TEXT NOT NULL DEFAULT '.',
  default_branch TEXT NOT NULL DEFAULT '',
  access TEXT NOT NULL DEFAULT 'write',
  policy TEXT NOT NULL DEFAULT '{}',
  created TEXT NOT NULL,
  PRIMARY KEY (project, name)
);
-- What a task does with a repository: its access and how delivery stands.
CREATE TABLE IF NOT EXISTS task_repos (
  project TEXT NOT NULL REFERENCES projects(slug) ON DELETE CASCADE,
  task TEXT NOT NULL,
  repo TEXT NOT NULL,
  access TEXT NOT NULL DEFAULT 'write',
  branch TEXT NOT NULL DEFAULT '',
  state TEXT NOT NULL DEFAULT 'pending',
  cr_number INTEGER,
  cr_url TEXT,
  cr_state TEXT,
  ci_state TEXT,
  -- The ref whose checks are watched (`genie/S-1`, or the target branch after a merge),
  -- the commit those checks belong to, and when waiting for that commit's checks began
  -- (empty once they settled). One watched ref per task and repository.
  ci_ref TEXT NOT NULL DEFAULT '',
  ci_sha TEXT,
  ci_since TEXT NOT NULL DEFAULT '',
  -- The commit whose failed checks were last rerun, and how many reruns were started for this
  -- row. They are history: a re-arm (`reset_ci`) must not clear them.
  ci_rerun_sha TEXT NOT NULL DEFAULT '',
  ci_reruns INTEGER NOT NULL DEFAULT 0,
  head_sha TEXT,
  -- The host's timestamp of the newest comment already passed on to the task.
  seen_at TEXT NOT NULL DEFAULT '',
  updated TEXT NOT NULL,
  PRIMARY KEY (project, task, repo)
);
-- A person's own agent session acting as a project's orchestrator (`genie orchestrate`).
CREATE TABLE IF NOT EXISTS consoles (
  project TEXT PRIMARY KEY,
  user TEXT NOT NULL,
  token_hash TEXT NOT NULL,
  taken TEXT NOT NULL,
  until TEXT NOT NULL
);
"#;

/// Columns added after the first server release; applied to existing databases on open.
const SERVER_COLUMN_MIGRATIONS: &[(&str, &str, &str)] = &[
    // Agent tokens name the configured role the agent acts in.
    ("api_tokens", "role_id", "ALTER TABLE api_tokens ADD COLUMN role_id TEXT"),
    // How a project's finished work gets integrated unless a task says otherwise.
    ("projects", "integration", "ALTER TABLE projects ADD COLUMN integration TEXT NOT NULL DEFAULT ''"),
    // A person's photo: the image itself, its media type and when it changed (the cache key).
    ("users", "avatar", "ALTER TABLE users ADD COLUMN avatar BLOB"),
    ("users", "avatar_type", "ALTER TABLE users ADD COLUMN avatar_type TEXT"),
    ("users", "avatar_at", "ALTER TABLE users ADD COLUMN avatar_at TEXT"),
    // When a personal token was last presented, so its owner can tell which ones are stale.
    ("api_tokens", "last_used", "ALTER TABLE api_tokens ADD COLUMN last_used TEXT"),
    // The person a job runs on behalf of (whose LiteLLM key it uses).
    ("agent_jobs", "initiator", "ALTER TABLE agent_jobs ADD COLUMN initiator TEXT"),
    // The watched CI ref of a delivery, its commit and since when its checks are awaited.
    ("task_repos", "ci_ref", "ALTER TABLE task_repos ADD COLUMN ci_ref TEXT NOT NULL DEFAULT ''"),
    ("task_repos", "ci_sha", "ALTER TABLE task_repos ADD COLUMN ci_sha TEXT"),
    ("task_repos", "ci_since", "ALTER TABLE task_repos ADD COLUMN ci_since TEXT NOT NULL DEFAULT ''"),
    // The commit whose checks were last rerun and how many reruns this delivery has used.
    ("task_repos", "ci_rerun_sha", "ALTER TABLE task_repos ADD COLUMN ci_rerun_sha TEXT NOT NULL DEFAULT ''"),
    ("task_repos", "ci_reruns", "ALTER TABLE task_repos ADD COLUMN ci_reruns INTEGER NOT NULL DEFAULT 0"),
];

pub const SESSION_DAYS: i64 = 30;

/// Project membership role (distinct from the workflow `Role` of agents).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectRole {
    Viewer,
    Member,
    Admin,
    Owner,
}

impl ProjectRole {
    pub fn as_str(self) -> &'static str {
        match self {
            ProjectRole::Viewer => "viewer",
            ProjectRole::Member => "member",
            ProjectRole::Admin => "admin",
            ProjectRole::Owner => "owner",
        }
    }
    pub fn parse(s: &str) -> Result<ProjectRole> {
        Ok(match s {
            "viewer" | "guest" => ProjectRole::Viewer,
            "member" => ProjectRole::Member,
            "admin" => ProjectRole::Admin,
            "owner" => ProjectRole::Owner,
            _ => return Err(GenieError::invalid(format!("unknown project role {s}"))),
        })
    }
    pub fn can_write(self) -> bool {
        self >= ProjectRole::Member
    }
    pub fn can_admin(self) -> bool {
        self >= ProjectRole::Admin
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: i64,
    pub login: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub is_admin: bool,
    pub disabled: bool,
    pub created: String,
    /// Where the person's photo is served, versioned by when it changed; none without a photo.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
}

impl User {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<User> {
        let id: i64 = r.get("id")?;
        let avatar_at: Option<String> = r.get("avatar_at")?;
        Ok(User {
            id,
            login: r.get("login")?,
            name: r.get("name")?,
            email: r.get("email")?,
            is_admin: r.get::<_, i64>("is_admin")? != 0,
            disabled: r.get::<_, i64>("disabled")? != 0,
            created: r.get("created")?,
            avatar: avatar_at.map(|at| format!("/api/users/{id}/avatar?v={}", &hash_secret(&at)[..10])),
        })
    }
}

/// A personal token as its owner sees it: never the secret, only what tells tokens apart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserToken {
    pub id: i64,
    pub label: String,
    pub created: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<String>,
}

/// Photo formats a person may upload, by their leading bytes.
pub fn avatar_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// The largest photo the server keeps; the web shrinks pictures to 256×256 before sending.
pub const AVATAR_MAX_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub slug: String,
    pub name: String,
    pub tracker_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Vault space (top-level folder) of the project.
    pub space: String,
    /// `autonomous` — the orchestrator works and closes tasks itself; `assisted`
    /// — it works, people close tasks; `manual` — no server orchestrator.
    pub autonomy: String,
    /// How finished work gets integrated unless a task says otherwise
    /// (the task's `mergeStrategy`), e.g. "the owner reviews the branch and merges it".
    pub integration: String,
    pub created: String,
}

impl Project {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<Project> {
        Ok(Project {
            slug: r.get("slug")?,
            name: r.get("name")?,
            tracker_dir: r.get("tracker_dir")?,
            repo: r.get("repo")?,
            space: r.get("space")?,
            autonomy: r.get("autonomy")?,
            integration: r.get("integration")?,
            created: r.get("created")?,
        })
    }
}

/// An edit of the agent configuration made through the server (roles, templates, skills, MCP).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigChange {
    pub id: i64,
    pub at: String,
    pub user: String,
    /// `role:<id>`, `team:<id>`, `skill:<name>` or `mcp`.
    pub item: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

/// The orchestrator console of a project: someone's own agent session acting as
/// its orchestrator while the server's orchestrator waits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Console {
    pub project: String,
    /// The login of the person at the console.
    pub user: String,
    pub taken: String,
    /// The console lapses at this time unless renewed.
    pub until: String,
}

/// The secret part of a token (without its `gnu_` / `gna_` prefix).
fn secret_of(token: &str) -> &str {
    token.strip_prefix("gnu_").or_else(|| token.strip_prefix("gna_")).unwrap_or(token)
}

/// Who an API token speaks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Principal {
    /// A person, via session cookie or personal token.
    User { user: User },
    /// An agent: orchestrator, team member or one-shot job, bound to one project.
    /// `role` is the class; `role_id` the configured role it acts in.
    Agent { project: String, role: Role, role_id: Option<String>, name: String, team: Option<String>, job: Option<i64> },
}

/// Random secret: 32 bytes, hex.
pub fn new_secret() -> String {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).expect("OS random source");
    hex::encode(buf)
}

/// Short human-typable code (link Telegram and similar), 8 characters.
pub fn new_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut buf = [0u8; 8];
    getrandom::fill(&mut buf).expect("OS random source");
    buf.iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect()
}

pub fn hash_secret(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

pub fn hash_password(password: &str) -> Result<String> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).expect("OS random source");
    let salt = SaltString::encode_b64(&salt).map_err(|e| GenieError::invalid(e.to_string()))?;
    Ok(Argon2::default().hash_password(password.as_bytes(), &salt).map_err(|e| GenieError::invalid(e.to_string()))?.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).map(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok()).unwrap_or(false)
}

pub fn time_in(duration: ChronoDuration) -> String {
    (Utc::now() + duration).to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// A project slug: lowercase latin letters, digits and dashes (it names directories).
/// A login as stored: trimmed, lower case, latin letters, digits, dot, dash or underscore.
pub fn valid_login(login: &str) -> Result<String> {
    let login = login.trim().to_lowercase();
    if login.is_empty() || !login.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        return Err(GenieError::invalid("login must be latin letters, digits, dot, dash or underscore"));
    }
    Ok(login)
}

pub fn valid_slug(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !s.starts_with('-')
}

pub struct ServerDb {
    db: Db,
    /// The key people's secrets are sealed with (`secrets`).
    pub(crate) secrets_key: std::path::PathBuf,
}

impl ServerDb {
    pub fn open(path: &Path) -> Result<ServerDb> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let db = Db::open_with_schema(path, SERVER_SCHEMA)?;
        db.conn().execute_batch(crate::secrets::SECRETS_SCHEMA)?;
        db.conn().execute("INSERT OR IGNORE INTO meta(key, value) VALUES ('schema', '1')", [])?;
        for (table, column, ddl) in SERVER_COLUMN_MIGRATIONS {
            let has = db
                .conn()
                .prepare(&format!("PRAGMA table_info({table})"))?
                .query_map([], |r| r.get::<_, String>(1))?
                .filter_map(|c| c.ok())
                .any(|c| c == *column);
            if !has
                && let Err(e) = db.conn().execute_batch(ddl)
                && !e.to_string().contains("duplicate column")
            {
                return Err(e.into());
            }
        }
        Ok(ServerDb { db, secrets_key: crate::secrets::key_path(path) })
    }

    pub fn conn(&self) -> &Connection {
        self.db.conn()
    }

    pub fn tx<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.db.tx(f)
    }

    // --- users ---------------------------------------------------------------

    pub fn user_count(&self) -> Result<i64> {
        Ok(self.conn().query_row("SELECT COUNT(*) FROM users WHERE disabled = 0", [], |r| r.get(0))?)
    }

    pub fn create_user(&self, login: &str, name: &str, email: Option<&str>, password: Option<&str>, is_admin: bool) -> Result<User> {
        let login = valid_login(login)?;
        if let Some(p) = password
            && p.chars().count() < 8
        {
            return Err(GenieError::invalid("password must be at least 8 characters"));
        }
        let hash = password.map(hash_password).transpose()?;
        let name = if name.trim().is_empty() { login.clone() } else { name.trim().to_string() };
        let res = self.conn().execute(
            "INSERT INTO users(login, name, email, password_hash, is_admin, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![login, name, email.map(str::trim).filter(|e| !e.is_empty()), hash, is_admin as i64, now()],
        );
        match res {
            Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => {
                Err(GenieError::invalid(format!("user {login} already exists")))
            }
            other => {
                other?;
                self.user(self.conn().last_insert_rowid())
            }
        }
    }

    pub fn user(&self, id: i64) -> Result<User> {
        self.conn()
            .query_row("SELECT * FROM users WHERE id = ?1", [id], User::from_row)
            .optional()?
            .ok_or_else(|| GenieError::not_found(format!("user {id} not found")))
    }

    pub fn user_by_login(&self, login: &str) -> Result<Option<User>> {
        Ok(self.conn().query_row("SELECT * FROM users WHERE login = ?1", [login.trim().to_lowercase()], User::from_row).optional()?)
    }

    pub fn users(&self) -> Result<Vec<User>> {
        let mut stmt = self.conn().prepare("SELECT * FROM users ORDER BY login")?;
        Ok(stmt.query_map([], User::from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    /// Give a person a new login. Sessions, tokens and memberships follow the account;
    /// the caller moves what names the person by login elsewhere (tasks, consoles).
    pub fn rename_user(&self, user: i64, login: &str) -> Result<User> {
        let login = valid_login(login)?;
        let current = self.user(user)?;
        if current.login == login {
            return Ok(current);
        }
        match self.conn().execute("UPDATE users SET login = ?1 WHERE id = ?2", params![login, user]) {
            Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == rusqlite::ErrorCode::ConstraintViolation => {
                return Err(GenieError::invalid(format!("login {login} is taken")));
            }
            other => other?,
        };
        self.conn().execute("UPDATE consoles SET user = ?1 WHERE user = ?2", params![login, current.login])?;
        self.user(user)
    }

    /// Set a person's photo (checked: a PNG, JPEG or WebP within [`AVATAR_MAX_BYTES`]).
    pub fn set_avatar(&self, user: i64, bytes: &[u8]) -> Result<User> {
        if bytes.len() > AVATAR_MAX_BYTES {
            return Err(GenieError::invalid("the photo is larger than 512 KB"));
        }
        let kind = avatar_type(bytes).ok_or_else(|| GenieError::invalid("the photo must be PNG, JPEG or WebP"))?;
        self.conn().execute(
            "UPDATE users SET avatar = ?1, avatar_type = ?2, avatar_at = ?3 WHERE id = ?4",
            params![bytes, kind, format!("{}#{}", now(), new_code()), user],
        )?;
        self.user(user)
    }

    pub fn clear_avatar(&self, user: i64) -> Result<User> {
        self.conn().execute("UPDATE users SET avatar = NULL, avatar_type = NULL, avatar_at = NULL WHERE id = ?1", [user])?;
        self.user(user)
    }

    /// A person's photo and its media type.
    pub fn avatar(&self, user: i64) -> Result<Option<(Vec<u8>, String)>> {
        Ok(self
            .conn()
            .query_row("SELECT avatar, avatar_type FROM users WHERE id = ?1 AND avatar IS NOT NULL", [user], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?)
    }

    pub fn set_password(&self, user: i64, password: &str) -> Result<()> {
        if password.chars().count() < 8 {
            return Err(GenieError::invalid("password must be at least 8 characters"));
        }
        self.conn().execute("UPDATE users SET password_hash = ?1 WHERE id = ?2", params![hash_password(password)?, user])?;
        self.conn().execute("DELETE FROM sessions WHERE user = ?1", [user])?;
        Ok(())
    }

    pub fn update_user(
        &self,
        user: i64,
        name: Option<&str>,
        email: Option<Option<&str>>,
        is_admin: Option<bool>,
        disabled: Option<bool>,
    ) -> Result<User> {
        if let Some(n) = name.map(str::trim).filter(|n| !n.is_empty()) {
            self.conn().execute("UPDATE users SET name = ?1 WHERE id = ?2", params![n, user])?;
        }
        if let Some(e) = email {
            self.conn().execute("UPDATE users SET email = ?1 WHERE id = ?2", params![e.map(str::trim).filter(|e| !e.is_empty()), user])?;
        }
        if let Some(a) = is_admin {
            self.conn().execute("UPDATE users SET is_admin = ?1 WHERE id = ?2", params![a as i64, user])?;
        }
        if let Some(d) = disabled {
            self.conn().execute("UPDATE users SET disabled = ?1 WHERE id = ?2", params![d as i64, user])?;
            if d {
                self.conn().execute("DELETE FROM sessions WHERE user = ?1", [user])?;
                self.conn().execute("UPDATE api_tokens SET revoked = 1 WHERE user = ?1", [user])?;
            }
        }
        self.user(user)
    }

    /// Check a login and password; disabled users and users without a password cannot log in.
    pub fn authenticate(&self, login: &str, password: &str) -> Result<Option<User>> {
        let row: Option<(i64, Option<String>, i64)> = self
            .conn()
            .query_row("SELECT id, password_hash, disabled FROM users WHERE login = ?1", [login.trim().to_lowercase()], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        let Some((id, Some(hash), 0)) = row else {
            // Spend comparable time for unknown users so logins cannot be enumerated by timing.
            static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
            let _ = verify_password(password, DUMMY.get_or_init(|| hash_password("genie-timing-dummy").unwrap_or_default()));
            return Ok(None);
        };
        Ok(if verify_password(password, &hash) { Some(self.user(id)?) } else { None })
    }

    // --- sessions and tokens -------------------------------------------------

    /// New browser session; returns the secret for the cookie.
    pub fn create_session(&self, user: i64) -> Result<String> {
        let secret = new_secret();
        self.conn().execute(
            "INSERT INTO sessions(token_hash, user, created, expires) VALUES (?1, ?2, ?3, ?4)",
            params![hash_secret(&secret), user, now(), time_in(ChronoDuration::days(SESSION_DAYS))],
        )?;
        Ok(secret)
    }

    pub fn session_user(&self, secret: &str) -> Result<Option<User>> {
        let user: Option<i64> = self
            .conn()
            .query_row(
                "SELECT s.user FROM sessions s JOIN users u ON u.id = s.user WHERE s.token_hash = ?1 AND s.expires > ?2 AND u.disabled = 0",
                params![hash_secret(secret), now()],
                |r| r.get(0),
            )
            .optional()?;
        user.map(|id| self.user(id)).transpose()
    }

    pub fn delete_session(&self, secret: &str) -> Result<()> {
        self.conn().execute("DELETE FROM sessions WHERE token_hash = ?1", [hash_secret(secret)])?;
        Ok(())
    }

    /// Personal API token for the CLI; returns the secret once.
    pub fn create_user_token(&self, user: i64, label: &str) -> Result<String> {
        let secret = new_secret();
        self.conn().execute(
            "INSERT INTO api_tokens(token_hash, kind, user, label, created) VALUES (?1, 'user', ?2, ?3, ?4)",
            params![hash_secret(&secret), user, label, now()],
        )?;
        Ok(format!("gnu_{secret}"))
    }

    /// A person's live personal tokens, newest first.
    pub fn user_tokens(&self, user: i64) -> Result<Vec<UserToken>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, label, created, last_used FROM api_tokens WHERE kind = 'user' AND user = ?1 AND revoked = 0 ORDER BY created DESC, id DESC",
        )?;
        Ok(stmt
            .query_map([user], |r| Ok(UserToken { id: r.get(0)?, label: r.get(1)?, created: r.get(2)?, last_used: r.get(3)? }))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Revoke one of a person's own tokens; false when they have no such live token.
    pub fn revoke_user_token(&self, user: i64, id: i64) -> Result<bool> {
        Ok(self.conn().execute(
            "UPDATE api_tokens SET revoked = 1 WHERE id = ?1 AND kind = 'user' AND user = ?2 AND revoked = 0",
            params![id, user],
        )? > 0)
    }

    /// Token for an agent run: bound to a project and a workflow role; expires.
    pub fn create_agent_token(
        &self,
        project: &str,
        role: Role,
        name: &str,
        team: Option<&str>,
        job: Option<i64>,
        ttl: ChronoDuration,
    ) -> Result<String> {
        self.create_role_token(project, role, None, name, team, job, ttl)
    }

    /// Token for an agent acting in a configured role (`role_id`) of class `role`.
    #[allow(clippy::too_many_arguments)]
    pub fn create_role_token(
        &self,
        project: &str,
        role: Role,
        role_id: Option<&str>,
        name: &str,
        team: Option<&str>,
        job: Option<i64>,
        ttl: ChronoDuration,
    ) -> Result<String> {
        let secret = new_secret();
        self.conn().execute(
            "INSERT INTO api_tokens(token_hash, kind, project, agent_role, role_id, team, member, job, label, created, expires)
             VALUES (?1, 'agent', ?2, ?3, ?4, ?5, ?6, ?7, ?6, ?8, ?9)",
            params![hash_secret(&secret), project, role, role_id, team, name, job, now(), time_in(ttl)],
        )?;
        Ok(format!("gna_{secret}"))
    }

    pub fn resolve_token(&self, token: &str) -> Result<Option<Principal>> {
        let secret = secret_of(token);
        type Row = (
            String,
            Option<i64>,
            Option<String>,
            Option<Role>,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<String>,
            i64,
            Option<String>,
        );
        let row: Option<Row> = self
            .conn()
            .query_row(
                "SELECT kind, user, project, agent_role, team, member, job, role_id, id, last_used FROM api_tokens
                 WHERE token_hash = ?1 AND revoked = 0 AND (expires IS NULL OR expires > ?2)",
                params![hash_secret(secret), now()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?)),
            )
            .optional()?;
        Ok(match row {
            Some((kind, Some(user), .., id, last_used)) if kind == "user" => {
                let u = self.user(user)?;
                // Note the use at most once a minute, so a busy CLI does not write on every call.
                let t = now();
                if last_used.as_deref().is_none_or(|l| l < time_in(ChronoDuration::minutes(-1)).as_str()) {
                    self.conn().execute("UPDATE api_tokens SET last_used = ?1 WHERE id = ?2", params![t, id])?;
                }
                (!u.disabled).then_some(Principal::User { user: u })
            }
            Some((kind, _, Some(project), Some(role), team, Some(name), job, role_id, ..)) if kind == "agent" => {
                Some(Principal::Agent { project, role, role_id, name, team, job })
            }
            _ => None,
        })
    }

    /// Revoke one token by its secret (an agent's per-turn token when the turn ends).
    pub fn revoke_token(&self, token: &str) -> Result<()> {
        let secret = secret_of(token);
        self.conn().execute("UPDATE api_tokens SET revoked = 1 WHERE token_hash = ?1", [hash_secret(secret)])?;
        Ok(())
    }

    pub fn revoke_agent_tokens(&self, project: &str, team: Option<&str>, job: Option<i64>) -> Result<()> {
        self.conn().execute(
            "UPDATE api_tokens SET revoked = 1 WHERE kind = 'agent' AND project = ?1 AND (?2 IS NULL OR team = ?2) AND (?3 IS NULL OR job = ?3)",
            params![project, team, job],
        )?;
        Ok(())
    }

    /// Drop expired sessions, agent tokens and link codes.
    pub fn sweep(&self) -> Result<()> {
        let t = now();
        self.conn().execute("DELETE FROM sessions WHERE expires <= ?1", [&t])?;
        self.conn().execute("DELETE FROM api_tokens WHERE kind = 'agent' AND expires <= ?1", [&t])?;
        self.conn().execute("DELETE FROM link_codes WHERE expires <= ?1", [&t])?;
        Ok(())
    }

    // --- invites -------------------------------------------------------------

    pub fn create_invite(&self, by: Option<i64>, project: Option<&str>, role: ProjectRole, email: Option<&str>) -> Result<String> {
        let secret = new_secret();
        self.conn().execute(
            "INSERT INTO invites(token_hash, created_by, project, project_role, email, expires) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![hash_secret(&secret), by, project, role.as_str(), email, time_in(ChronoDuration::days(7))],
        )?;
        Ok(secret)
    }

    /// Accept an invite: create the user and the membership in one transaction.
    pub fn accept_invite(&self, secret: &str, login: &str, name: &str, password: &str) -> Result<User> {
        self.tx(|| {
            let row: Option<(Option<String>, Option<String>, Option<String>)> = self
                .conn()
                .query_row(
                    "SELECT project, project_role, email FROM invites WHERE token_hash = ?1 AND used_at IS NULL AND expires > ?2",
                    params![hash_secret(secret), now()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((project, role, email)) = row else {
                return Err(GenieError::invalid("the invitation is invalid, used or expired"));
            };
            let user = self.create_user(login, name, email.as_deref(), Some(password), false)?;
            if let (Some(p), Some(r)) = (project, role) {
                self.set_membership(&p, user.id, ProjectRole::parse(&r)?)?;
            }
            self.conn().execute(
                "UPDATE invites SET used_by = ?1, used_at = ?2 WHERE token_hash = ?3",
                params![user.id, now(), hash_secret(secret)],
            )?;
            Ok(user)
        })
    }

    // --- projects and memberships -------------------------------------------

    pub fn create_project(&self, slug: &str, name: &str, tracker_dir: &str, repo: Option<&str>, space: Option<&str>) -> Result<Project> {
        let slug = slug.trim().to_lowercase();
        if !valid_slug(&slug) {
            return Err(GenieError::invalid("project slug must be lowercase latin letters, digits and dashes"));
        }
        if self.project_opt(&slug)?.is_some() {
            return Err(GenieError::invalid(format!("project {slug} already exists")));
        }
        self.conn().execute(
            "INSERT INTO projects(slug, name, tracker_dir, repo, space, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                slug,
                if name.trim().is_empty() { slug.as_str() } else { name.trim() },
                tracker_dir,
                repo,
                space.unwrap_or(&slug),
                now()
            ],
        )?;
        self.project(&slug)
    }

    pub fn project_opt(&self, slug: &str) -> Result<Option<Project>> {
        Ok(self.conn().query_row("SELECT * FROM projects WHERE slug = ?1", [slug], Project::from_row).optional()?)
    }

    pub fn project(&self, slug: &str) -> Result<Project> {
        self.project_opt(slug)?.ok_or_else(|| GenieError::not_found(format!("project {slug} not found")))
    }

    pub fn projects(&self) -> Result<Vec<Project>> {
        let mut stmt = self.conn().prepare("SELECT * FROM projects ORDER BY created, slug")?;
        Ok(stmt.query_map([], Project::from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_autonomy(&self, slug: &str, autonomy: &str) -> Result<()> {
        if !matches!(autonomy, "manual" | "assisted" | "autonomous") {
            return Err(GenieError::invalid("autonomy must be manual, assisted or autonomous"));
        }
        self.conn().execute("UPDATE projects SET autonomy = ?1 WHERE slug = ?2", params![autonomy, slug])?;
        Ok(())
    }

    /// Change a project's name and default integration (`None` keeps a field).
    pub fn update_project(&self, slug: &str, name: Option<&str>, integration: Option<&str>) -> Result<Project> {
        self.project(slug)?;
        if let Some(name) = name {
            if name.trim().is_empty() {
                return Err(GenieError::invalid("project name must not be empty"));
            }
            self.conn().execute("UPDATE projects SET name = ?1 WHERE slug = ?2", params![name.trim(), slug])?;
        }
        if let Some(i) = integration {
            self.conn().execute("UPDATE projects SET integration = ?1 WHERE slug = ?2", params![i.trim(), slug])?;
        }
        self.project(slug)
    }

    pub fn set_membership(&self, project: &str, user: i64, role: ProjectRole) -> Result<()> {
        self.conn().execute(
            "INSERT INTO memberships(project, user, role) VALUES (?1, ?2, ?3) ON CONFLICT(project, user) DO UPDATE SET role = excluded.role",
            params![project, user, role.as_str()],
        )?;
        Ok(())
    }

    pub fn remove_membership(&self, project: &str, user: i64) -> Result<()> {
        self.conn().execute("DELETE FROM memberships WHERE project = ?1 AND user = ?2", params![project, user])?;
        Ok(())
    }

    /// Effective role: server admins own every project.
    pub fn project_role(&self, project: &str, user: &User) -> Result<Option<ProjectRole>> {
        if user.is_admin {
            return Ok(Some(ProjectRole::Owner));
        }
        let role: Option<String> = self
            .conn()
            .query_row("SELECT role FROM memberships WHERE project = ?1 AND user = ?2", params![project, user.id], |r| r.get(0))
            .optional()?;
        role.map(|r| ProjectRole::parse(&r)).transpose()
    }

    pub fn members_of(&self, project: &str) -> Result<Vec<(User, ProjectRole)>> {
        let mut stmt = self.conn().prepare(
            "SELECT u.*, m.role AS prole FROM memberships m JOIN users u ON u.id = m.user WHERE m.project = ?1 ORDER BY u.login",
        )?;
        let rows = stmt.query_map([project], |r| Ok((User::from_row(r)?, r.get::<_, String>("prole")?)))?;
        rows.map(|r| {
            let (u, role) = r?;
            Ok((u, ProjectRole::parse(&role)?))
        })
        .collect()
    }

    // --- agent configuration history ------------------------------------------

    pub fn record_config_change(&self, user: &str, item: &str, path: &str, before: Option<&str>, after: Option<&str>) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO config_changes(at, user, item, path, before, after) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![now(), user, item, path, before, after],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    /// Newest first; `item` narrows to one role, template, skill or `mcp`.
    pub fn config_changes(&self, item: Option<&str>, limit: i64) -> Result<Vec<ConfigChange>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, at, user, item, path, before, after FROM config_changes WHERE (?1 IS NULL OR item = ?1) ORDER BY id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![item, limit], |r| {
            Ok(ConfigChange {
                id: r.get(0)?,
                at: r.get(1)?,
                user: r.get(2)?,
                item: r.get(3)?,
                path: r.get(4)?,
                before: r.get(5)?,
                after: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // --- the orchestrator console -----------------------------------------------

    /// The console of a project, while someone holds it.
    pub fn console(&self, project: &str) -> Result<Option<Console>> {
        Ok(self
            .conn()
            .query_row("SELECT project, user, taken, until FROM consoles WHERE project = ?1 AND until > ?2", params![project, now()], |r| {
                Ok(Console { project: r.get(0)?, user: r.get(1)?, taken: r.get(2)?, until: r.get(3)? })
            })
            .optional()?)
    }

    /// Take the orchestrator console of a project: a fresh orchestrator token for
    /// the person's session, valid as long as the console is held (`ttl`, then
    /// renewals). Someone else's console is refused unless `force`; a replaced
    /// console's token stops working.
    pub fn take_console(
        &self,
        project: &str,
        user: &str,
        role_id: Option<&str>,
        ttl: ChronoDuration,
        force: bool,
    ) -> Result<(Console, String)> {
        self.tx(|| {
            if let Some(c) = self.console(project)?
                && c.user != user
                && !force
            {
                return Err(GenieError::invalid(format!(
                    "the orchestrator console of {project} is held by {} since {}; take it over with force",
                    c.user, c.taken
                )));
            }
            let old: Option<String> =
                self.conn().query_row("SELECT token_hash FROM consoles WHERE project = ?1", [project], |r| r.get(0)).optional()?;
            if let Some(hash) = old {
                self.conn().execute("UPDATE api_tokens SET revoked = 1 WHERE token_hash = ?1", [hash])?;
            }
            let token = self.create_role_token(project, Role::Orchestrator, role_id, crate::team::ORCHESTRATOR, None, None, ttl)?;
            let console = Console { project: project.to_string(), user: user.to_string(), taken: now(), until: time_in(ttl) };
            self.conn().execute(
                "INSERT INTO consoles(project, user, token_hash, taken, until) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(project) DO UPDATE SET user = excluded.user, token_hash = excluded.token_hash, taken = excluded.taken, until = excluded.until",
                params![project, user, hash_secret(secret_of(&token)), console.taken, console.until],
            )?;
            Ok((console, token))
        })
    }

    /// Whether `token` is the token of the project's console, held now.
    pub fn is_console_token(&self, project: &str, token: &str) -> Result<bool> {
        let n: i64 = self.conn().query_row(
            "SELECT COUNT(*) FROM consoles WHERE project = ?1 AND token_hash = ?2 AND until > ?3",
            params![project, hash_secret(secret_of(token)), now()],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// Keep the console another `ttl`, its token too. Only its own token renews it.
    pub fn renew_console(&self, project: &str, token: &str, ttl: ChronoDuration) -> Result<Console> {
        self.tx(|| {
            if !self.is_console_token(project, token)? {
                return Err(GenieError::invalid(format!("this session no longer holds the orchestrator console of {project}")));
            }
            let (until, hash) = (time_in(ttl), hash_secret(secret_of(token)));
            self.conn().execute("UPDATE consoles SET until = ?1 WHERE project = ?2", params![until, project])?;
            self.conn().execute("UPDATE api_tokens SET expires = ?1 WHERE token_hash = ?2", params![until, hash])?;
            self.console(project)?.ok_or_else(|| GenieError::not_found("the console lapsed"))
        })
    }

    /// Give the console back — by its own token, or anyone's (`None`, an admin).
    /// Its token stops working. Returns whether a console was released.
    pub fn release_console(&self, project: &str, token: Option<&str>) -> Result<bool> {
        self.tx(|| {
            let hash: Option<String> =
                self.conn().query_row("SELECT token_hash FROM consoles WHERE project = ?1", [project], |r| r.get(0)).optional()?;
            let Some(hash) = hash else { return Ok(false) };
            if token.is_some_and(|t| hash_secret(secret_of(t)) != hash) {
                return Err(GenieError::invalid(format!("this session does not hold the orchestrator console of {project}")));
            }
            self.conn().execute("UPDATE api_tokens SET revoked = 1 WHERE token_hash = ?1", [&hash])?;
            self.conn().execute("DELETE FROM consoles WHERE project = ?1", [project])?;
            Ok(true)
        })
    }

    // --- leases --------------------------------------------------------------

    /// Take or renew a named lease (e.g. the orchestrator of a project). Returns
    /// false when someone else holds an unexpired lease.
    pub fn acquire_lock(&self, name: &str, holder: &str, ttl: ChronoDuration) -> Result<bool> {
        let changed = self.conn().execute(
            "INSERT INTO locks(name, holder, expires) VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET holder = excluded.holder, expires = excluded.expires
             WHERE locks.holder = excluded.holder OR locks.expires <= ?4",
            params![name, holder, time_in(ttl), now()],
        )?;
        Ok(changed > 0)
    }

    pub fn release_lock(&self, name: &str, holder: &str) -> Result<()> {
        self.conn().execute("DELETE FROM locks WHERE name = ?1 AND holder = ?2", params![name, holder])?;
        Ok(())
    }

    pub fn lock_holder(&self, name: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT holder FROM locks WHERE name = ?1 AND expires > ?2", params![name, now()], |r| r.get(0))
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_console_is_held_renewed_taken_over_and_given_back() {
        let (_d, db) = db();
        let ttl = ChronoDuration::seconds(60);
        let (c, anna) = db.take_console("shop", "anna", Some("orchestrator"), ttl, false).unwrap();
        assert_eq!((c.project.as_str(), c.user.as_str()), ("shop", "anna"));
        assert!(
            matches!(db.resolve_token(&anna).unwrap(), Some(Principal::Agent { role: Role::Orchestrator, ref name, .. }) if name == "orchestrator")
        );
        assert!(db.is_console_token("shop", &anna).unwrap());
        let e = db.take_console("shop", "bob", None, ttl, false).unwrap_err().to_string();
        assert!(e.contains("held by anna"), "{e}");
        db.renew_console("shop", &anna, ChronoDuration::seconds(120)).unwrap();

        // Taken over: the earlier session's token stops working and cannot renew.
        let (_, bob) = db.take_console("shop", "bob", None, ttl, true).unwrap();
        assert!(db.resolve_token(&anna).unwrap().is_none(), "the replaced console's token is revoked");
        assert!(db.renew_console("shop", &anna, ttl).is_err());
        assert!(db.release_console("shop", Some(&anna)).is_err(), "only its own token gives it back");
        assert!(db.release_console("shop", Some(&bob)).unwrap());
        assert!(db.console("shop").unwrap().is_none() && db.resolve_token(&bob).unwrap().is_none());
        assert!(!db.release_console("shop", None).unwrap(), "nothing to give back");

        // A console nobody renews lapses.
        db.take_console("shop", "anna", None, ChronoDuration::seconds(-1), false).unwrap();
        assert!(db.console("shop").unwrap().is_none());
        db.take_console("shop", "bob", None, ttl, false).unwrap();
    }

    fn db() -> (tempfile::TempDir, ServerDb) {
        let dir = tempfile::tempdir().unwrap();
        let db = ServerDb::open(&dir.path().join("server.db")).unwrap();
        (dir, db)
    }

    #[test]
    fn users_passwords_sessions() {
        let (_d, db) = db();
        let u = db.create_user("Anna", "Анна", Some("anna@example.com"), Some("correct horse"), true).unwrap();
        assert_eq!(u.login, "anna");
        assert!(db.create_user("anna", "", None, Some("another pass"), false).unwrap_err().to_string().contains("already exists"));
        assert!(db.create_user("bob", "", None, Some("short"), false).is_err());
        assert!(db.authenticate("anna", "wrong password").unwrap().is_none());
        assert!(db.authenticate("nobody", "correct horse").unwrap().is_none());
        assert_eq!(db.authenticate("ANNA", "correct horse").unwrap().unwrap().id, u.id);
        let s = db.create_session(u.id).unwrap();
        assert_eq!(db.session_user(&s).unwrap().unwrap().login, "anna");
        db.set_password(u.id, "new password!").unwrap();
        assert!(db.session_user(&s).unwrap().is_none(), "changing the password ends sessions");
        let s = db.create_session(u.id).unwrap();
        db.update_user(u.id, None, None, None, Some(true)).unwrap();
        assert!(db.session_user(&s).unwrap().is_none(), "disabled users lose their sessions");
        assert!(db.authenticate("anna", "new password!").unwrap().is_none());
    }

    #[test]
    fn tokens_resolve_to_principals_and_expire() {
        let (_d, db) = db();
        let u = db.create_user("pm", "PM", None, Some("password1"), false).unwrap();
        let personal = db.create_user_token(u.id, "cli").unwrap();
        assert!(matches!(db.resolve_token(&personal).unwrap(), Some(Principal::User { user }) if user.login == "pm"));
        db.create_project("shop", "Shop", "/tmp/x", None, None).unwrap();
        let agent = db.create_agent_token("shop", Role::Executor, "bender", Some("G-1"), None, ChronoDuration::hours(1)).unwrap();
        match db.resolve_token(&agent).unwrap() {
            Some(Principal::Agent { project, role, name, team, job, .. }) => {
                assert_eq!(
                    (project.as_str(), role, name.as_str(), team.as_deref(), job),
                    ("shop", Role::Executor, "bender", Some("G-1"), None)
                );
            }
            other => panic!("unexpected {other:?}"),
        }
        let expired = db.create_agent_token("shop", Role::Analyst, "x", None, None, ChronoDuration::seconds(-1)).unwrap();
        assert!(db.resolve_token(&expired).unwrap().is_none());
        db.revoke_agent_tokens("shop", Some("G-1"), None).unwrap();
        assert!(db.resolve_token(&agent).unwrap().is_none());
        assert!(db.resolve_token("gnu_nope").unwrap().is_none());
    }

    #[test]
    fn personal_tokens_are_listed_and_revoked_by_their_owner() {
        let (_d, db) = db();
        let anna = db.create_user("anna", "Anna", None, Some("password1"), false).unwrap();
        let boris = db.create_user("boris", "Boris", None, Some("password1"), false).unwrap();
        let laptop = db.create_user_token(anna.id, "laptop").unwrap();
        db.create_user_token(anna.id, "ci").unwrap();
        let list = db.user_tokens(anna.id).unwrap();
        assert_eq!(list.iter().map(|t| t.label.as_str()).collect::<Vec<_>>(), ["ci", "laptop"]);
        assert!(list.iter().all(|t| t.last_used.is_none()));
        db.resolve_token(&laptop).unwrap().unwrap();
        let used = db.user_tokens(anna.id).unwrap();
        assert!(used.iter().find(|t| t.label == "laptop").unwrap().last_used.is_some(), "a presented token notes its use");
        let id = used.iter().find(|t| t.label == "laptop").unwrap().id;
        assert!(!db.revoke_user_token(boris.id, id).unwrap(), "nobody revokes someone else's token");
        assert!(db.revoke_user_token(anna.id, id).unwrap());
        assert!(db.resolve_token(&laptop).unwrap().is_none());
        assert_eq!(db.user_tokens(anna.id).unwrap().len(), 1);
        assert!(!db.revoke_user_token(anna.id, id).unwrap(), "a revoked token is gone");
    }

    #[test]
    fn a_login_changes_and_the_account_follows() {
        let (_d, db) = db();
        let anna = db.create_user("anna", "Anna", None, Some("password1"), false).unwrap();
        db.create_user("boris", "Boris", None, Some("password1"), false).unwrap();
        let token = db.create_user_token(anna.id, "cli").unwrap();
        assert!(db.rename_user(anna.id, "boris").is_err(), "a taken login");
        assert!(db.rename_user(anna.id, "анна").is_err(), "latin only");
        let renamed = db.rename_user(anna.id, " Anna.N ").unwrap();
        assert_eq!(renamed.login, "anna.n");
        assert!(db.authenticate("anna.n", "password1").unwrap().is_some());
        assert!(db.authenticate("anna", "password1").unwrap().is_none());
        assert!(matches!(db.resolve_token(&token).unwrap(), Some(Principal::User { user }) if user.login == "anna.n"));
    }

    #[test]
    fn a_photo_is_an_image_and_its_address_changes_with_it() {
        let (_d, db) = db();
        let anna = db.create_user("anna", "Anna", None, Some("password1"), false).unwrap();
        assert!(anna.avatar.is_none());
        assert!(db.set_avatar(anna.id, b"<svg></svg>").is_err(), "no SVG or other formats");
        assert!(db.set_avatar(anna.id, &[0xFF; AVATAR_MAX_BYTES + 1]).is_err(), "too large");
        let png = b"\x89PNG\r\n\x1a\n rest of the picture";
        let first = db.set_avatar(anna.id, png).unwrap().avatar.unwrap();
        assert!(first.starts_with(&format!("/api/users/{}/avatar?v=", anna.id)));
        assert_eq!(db.avatar(anna.id).unwrap(), Some((png.to_vec(), "image/png".into())));
        let second = db.set_avatar(anna.id, &[0xFF, 0xD8, 0xFF, 0xE0]).unwrap().avatar.unwrap();
        assert_ne!(first, second, "a new photo is a new address, so caches refresh");
        assert!(db.clear_avatar(anna.id).unwrap().avatar.is_none());
        assert!(db.avatar(anna.id).unwrap().is_none());
    }

    #[test]
    fn invites_create_members() {
        let (_d, db) = db();
        db.create_project("shop", "Shop", "/tmp/x", None, None).unwrap();
        let secret = db.create_invite(None, Some("shop"), ProjectRole::Member, Some("pm@example.com")).unwrap();
        let u = db.accept_invite(&secret, "pm", "Product", "password1").unwrap();
        assert_eq!(u.email.as_deref(), Some("pm@example.com"));
        assert_eq!(db.project_role("shop", &u).unwrap(), Some(ProjectRole::Member));
        assert!(db.accept_invite(&secret, "pm2", "", "password1").is_err(), "an invite works once");
        assert!(db.user_by_login("pm2").unwrap().is_none(), "the failed accept rolled back");
    }

    #[test]
    fn projects_roles_and_leases() {
        let (_d, db) = db();
        assert!(db.create_project("Bad Slug", "", "/x", None, None).is_err());
        let p = db.create_project("shop", "", "/x", None, None).unwrap();
        assert_eq!((p.name.as_str(), p.space.as_str(), p.autonomy.as_str()), ("shop", "shop", "autonomous"));
        let admin = db.create_user("root", "", None, None, true).unwrap();
        let viewer = db.create_user("v", "", None, None, false).unwrap();
        assert_eq!(db.project_role("shop", &admin).unwrap(), Some(ProjectRole::Owner));
        assert_eq!(db.project_role("shop", &viewer).unwrap(), None);
        db.set_membership("shop", viewer.id, ProjectRole::Viewer).unwrap();
        assert!(!db.project_role("shop", &viewer).unwrap().unwrap().can_write());
        assert!(db.acquire_lock("orch:shop", "server", ChronoDuration::seconds(30)).unwrap());
        assert!(!db.acquire_lock("orch:shop", "cockpit", ChronoDuration::seconds(30)).unwrap());
        assert!(db.acquire_lock("orch:shop", "server", ChronoDuration::seconds(30)).unwrap(), "the holder renews");
        db.release_lock("orch:shop", "server").unwrap();
        assert!(db.acquire_lock("orch:shop", "cockpit", ChronoDuration::seconds(30)).unwrap());
    }
}
