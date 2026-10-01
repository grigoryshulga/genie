//! SQLite access: connection setup, schema, migrations and write transactions.
//!
//! The core opens an existing `.genie/genie.db` in place: the schema is created
//! if missing and older databases get the columns they lack.

use std::cell::Cell;
use std::path::Path;
use std::time::Duration;

use chrono::{SecondsFormat, Utc};
use rusqlite::Connection;

use crate::error::Result;

pub const SCHEMA_VERSION: i64 = 5;

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS tasks (
  id TEXT PRIMARY KEY,
  seq INTEGER NOT NULL UNIQUE,
  title TEXT NOT NULL,
  type TEXT NOT NULL,
  status TEXT NOT NULL,
  priority INTEGER NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  plan TEXT NOT NULL DEFAULT '',
  notes TEXT NOT NULL DEFAULT '',
  parent TEXT REFERENCES tasks(id),
  labels TEXT NOT NULL DEFAULT '[]',
  assignees TEXT NOT NULL DEFAULT '[]',
  team TEXT,
  worktree TEXT,
  blocked TEXT,
  needs_owner TEXT,
  merge_strategy TEXT NOT NULL DEFAULT '',
  assignee TEXT NOT NULL DEFAULT '',
  created TEXT NOT NULL,
  updated TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS tasks_status ON tasks(status);
CREATE INDEX IF NOT EXISTS tasks_parent ON tasks(parent);
CREATE TABLE IF NOT EXISTS deps (
  task TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  dep TEXT NOT NULL REFERENCES tasks(id),
  PRIMARY KEY (task, dep)
);
CREATE TABLE IF NOT EXISTS acceptance (
  task TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  n INTEGER NOT NULL,
  text TEXT NOT NULL,
  done INTEGER NOT NULL DEFAULT 0,
  checked_by TEXT,
  checked_at TEXT,
  PRIMARY KEY (task, n)
);
CREATE TABLE IF NOT EXISTS comments (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  task TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  at TEXT NOT NULL,
  author TEXT NOT NULL,
  role TEXT NOT NULL,
  kind TEXT NOT NULL,
  text TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS comments_task ON comments(task);
CREATE TABLE IF NOT EXISTS artifacts (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  task TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  n INTEGER NOT NULL,
  at TEXT NOT NULL,
  author TEXT NOT NULL,
  role TEXT NOT NULL,
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  note TEXT,
  size INTEGER NOT NULL,
  content BLOB NOT NULL,
  UNIQUE (task, n)
);
CREATE TABLE IF NOT EXISTS history (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  task TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  at TEXT NOT NULL,
  actor TEXT NOT NULL,
  role TEXT NOT NULL,
  event TEXT NOT NULL,
  from_status TEXT,
  to_status TEXT,
  note TEXT
);
CREATE INDEX IF NOT EXISTS history_task ON history(task);
CREATE TABLE IF NOT EXISTS teams (
  id TEXT PRIMARY KEY,
  task TEXT NOT NULL,
  template TEXT,
  cwd TEXT NOT NULL,
  worktree TEXT,
  state TEXT NOT NULL,
  created TEXT NOT NULL,
  updated TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS members (
  team TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  role TEXT NOT NULL,
  model TEXT,
  thinking TEXT,
  instructions TEXT,
  status TEXT NOT NULL,
  status_at TEXT NOT NULL,
  state TEXT NOT NULL,
  activity TEXT NOT NULL DEFAULT 'idle',
  activity_at TEXT,
  runtime TEXT,
  session_file TEXT,
  ord INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (team, name)
);
CREATE TABLE IF NOT EXISTS mail (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  team TEXT,
  at TEXT NOT NULL,
  sender TEXT NOT NULL,
  sender_role TEXT NOT NULL,
  recipient TEXT NOT NULL,
  text TEXT NOT NULL,
  urgent INTEGER NOT NULL DEFAULT 0,
  level TEXT NOT NULL DEFAULT 'normal',
  intent TEXT,
  kind TEXT NOT NULL,
  task TEXT,
  delivered_at TEXT
);
CREATE INDEX IF NOT EXISTS mail_pending ON mail(recipient, delivered_at);
CREATE TABLE IF NOT EXISTS deliveries (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  team TEXT,
  recipient TEXT NOT NULL,
  created TEXT NOT NULL,
  acked_at TEXT,
  released_at TEXT,
  mail TEXT NOT NULL DEFAULT '[]'
);
CREATE TABLE IF NOT EXISTS log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  team TEXT NOT NULL,
  at TEXT NOT NULL,
  event TEXT NOT NULL,
  data TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS log_team ON log(team);
CREATE TABLE IF NOT EXISTS events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  at TEXT NOT NULL,
  type TEXT NOT NULL,
  subject TEXT,
  actor TEXT NOT NULL,
  actor_role TEXT NOT NULL,
  payload TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS events_subject ON events(subject, id);
CREATE TABLE IF NOT EXISTS event_cursors (
  subscriber TEXT PRIMARY KEY,
  last_id INTEGER NOT NULL,
  updated TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS usage (
  day TEXT NOT NULL,
  agent TEXT NOT NULL,
  task TEXT NOT NULL DEFAULT '',
  model TEXT NOT NULL,
  calls INTEGER NOT NULL DEFAULT 0,
  input INTEGER NOT NULL DEFAULT 0,
  output INTEGER NOT NULL DEFAULT 0,
  cache_read INTEGER NOT NULL DEFAULT 0,
  cache_write INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (day, agent, task, model)
);
CREATE INDEX IF NOT EXISTS usage_task ON usage(task);
CREATE INDEX IF NOT EXISTS usage_agent ON usage(agent);
"#;

/// Columns added after the first release; applied to existing databases on open.
const COLUMN_MIGRATIONS: &[(&str, &str, &str)] = &[
    ("members", "heartbeat_at", "ALTER TABLE members ADD COLUMN heartbeat_at TEXT"),
    ("teams", "stop_reason", "ALTER TABLE teams ADD COLUMN stop_reason TEXT"),
    ("mail", "level", "ALTER TABLE mail ADD COLUMN level TEXT NOT NULL DEFAULT 'normal'"),
    ("mail", "intent", "ALTER TABLE mail ADD COLUMN intent TEXT"),
    // Rust runtime: mail leased to an agent turn; delivered only when the turn succeeds.
    ("mail", "lease", "ALTER TABLE mail ADD COLUMN lease INTEGER"),
    // Live agent sessions: mail handed to a running session at a step boundary
    // (`deliveries`), superseding by topic, and ask/reply threads.
    ("mail", "delivery", "ALTER TABLE mail ADD COLUMN delivery INTEGER"),
    ("mail", "topic", "ALTER TABLE mail ADD COLUMN topic TEXT"),
    ("mail", "reply_to", "ALTER TABLE mail ADD COLUMN reply_to INTEGER"),
    ("mail", "awaits", "ALTER TABLE mail ADD COLUMN awaits INTEGER NOT NULL DEFAULT 0"),
    ("mail", "superseded_by", "ALTER TABLE mail ADD COLUMN superseded_by INTEGER"),
    // Configurable roles and templates: the template snapshot a team was assembled from.
    ("teams", "spec", "ALTER TABLE teams ADD COLUMN spec TEXT"),
    // The person responsible for a task (a login of the server), next to the team working on it.
    ("tasks", "assignee", "ALTER TABLE tasks ADD COLUMN assignee TEXT NOT NULL DEFAULT ''"),
];

/// Current time as an ISO 8601 UTC timestamp with milliseconds (`2026-09-29T12:00:00.000Z`).
pub fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub struct Db {
    conn: Connection,
    depth: Cell<u32>,
}

impl Db {
    /// Open the task tracker database: tracker schema plus column migrations.
    pub fn open(path: &Path) -> Result<Db> {
        let db = Db::open_with_schema(path, SCHEMA)?;
        db.migrate()?;
        Ok(db)
    }

    /// Open any genie SQLite file: WAL, busy timeout, foreign keys, then `schema`.
    pub fn open_with_schema(path: &Path, schema: &str) -> Result<Db> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(10))?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get::<_, String>(0))?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
        conn.execute_batch(schema)?;
        Ok(Db { conn, depth: Cell::new(0) })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    fn migrate(&self) -> Result<()> {
        for (table, column, ddl) in COLUMN_MIGRATIONS {
            let mut stmt = self.conn.prepare(&format!("PRAGMA table_info({table})"))?;
            let has = stmt.query_map([], |r| r.get::<_, String>(1))?.filter_map(|c| c.ok()).any(|c| c == *column);
            // A "duplicate column" error means another process migrated concurrently.
            if !has
                && let Err(err) = self.conn.execute_batch(ddl)
                && !err.to_string().contains("duplicate column")
            {
                return Err(err.into());
            }
        }
        // Indexes on migrated columns (an old database gets the columns just above).
        self.conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS mail_delivery ON mail(delivery);
             CREATE INDEX IF NOT EXISTS mail_reply ON mail(reply_to);",
        )?;
        // Legacy rows only knew `urgent`; normalise them to the level vocabulary.
        self.conn.execute("UPDATE mail SET level = 'high' WHERE urgent = 1 AND level <> 'high'", [])?;
        self.conn.execute("UPDATE meta SET value = ?1 WHERE key = 'schema' AND CAST(value AS INTEGER) < ?1", [SCHEMA_VERSION])?;
        Ok(())
    }

    /// Write transaction. `BEGIN IMMEDIATE` takes the write lock up front so
    /// concurrent writers queue instead of failing; nested calls join the outer one.
    pub fn tx<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        if self.depth.get() > 0 {
            return f();
        }
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        self.depth.set(1);
        let out = f();
        self.depth.set(0);
        match out {
            Ok(v) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
}
