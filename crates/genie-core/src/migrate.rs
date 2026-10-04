//! Versioned migrations: what a database records is read before the file is touched; a
//! pending migration is rehearsed on a copy (`VACUUM INTO`) and checked there before the
//! live file is opened for migration.
//!
//! Nothing here writes to the database it is given. `check` opens it read-only, puts the
//! copy in the system temp directory (never next to the database: a tracker can live inside
//! a repository) and removes it on every path, errors included. Data recorded as *newer*
//! than the binary understands is refused — so from this release on a binary rolled back
//! without a restore fails loudly instead of writing into data it does not know.
//!
//! What counts as pending: the recorded version is behind the target **or** a column the
//! schema expects is missing. The second half catches a database restored from an old copy,
//! an interrupted `ALTER` or a tracker from before the version column existed.
//!
//! The rehearsal is meant to be honest, not strict for its own sake: a migration that
//! deletes or merges rows must say so here, since the check refuses a changed row count
//! (no migration today moves rows), and a foreign-key violation the database already had
//! does not block the update — only one the migration *adds* does (the copy's violations
//! must be a subset of the source's, compared row by row, not by count).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;

use crate::error::{GenieError, Result};

/// A column migration: its table, the column and the DDL that adds it.
pub type Column = (&'static str, &'static str, &'static str);

/// What one database records, needs and was checked for.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// The database file.
    pub path: PathBuf,
    /// The version it records (`0` when it has no `meta`: a tracker from before the column).
    pub from: i64,
    /// The version this binary understands.
    pub to: i64,
    /// Columns the schema expects and the database lacks, `table.column`.
    pub pending: Vec<String>,
    /// A copy was made and the migration rehearsed and checked on it.
    pub rehearsed: bool,
    /// Tables in the rehearsed copy.
    pub tables: usize,
    /// Rows in it (all tables; the ones that existed before kept their counts). `None` when
    /// nothing was rehearsed and no row was counted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<u64>,
}

impl Report {
    /// Something has to be applied to this database.
    pub fn pending(&self) -> bool {
        self.from != self.to || !self.pending.is_empty()
    }
}

/// The preflight of every versioned open: check the database (on a copy when something is
/// pending) and remember it for the process; the caller migrates the live file itself.
pub fn preflight(path: &Path, target: i64, columns: &[Column], apply: &dyn Fn(&Path) -> Result<()>) -> Result<()> {
    let report = check(path, target, columns, apply)?;
    if report.rehearsed {
        remember(report);
    }
    Ok(())
}

/// Check one database without changing it: read what it records, refuse data from a newer
/// genie, and when a migration is pending rehearse it on a copy and check the copy —
/// integrity, foreign keys, the tables, the row counts and the version.
pub fn check(path: &Path, target: i64, columns: &[Column], apply: &dyn Fn(&Path) -> Result<()>) -> Result<Report> {
    let current = |tables: usize| Report {
        path: path.to_path_buf(),
        from: target,
        to: target,
        pending: Vec::new(),
        rehearsed: false,
        tables,
        rows: None,
    };
    if !path.exists() {
        // A database that is not there yet: nothing to check, the schema creates it.
        return Ok(current(0));
    }
    let conn = read_only(path).map_err(|e| context(path, e))?;
    let before = survey(&conn, columns).map_err(|e| context(path, e))?;
    if before.version > target {
        return Err(GenieError::invalid(refused(path, before.version, target)));
    }
    if before.version == target && before.missing.is_empty() {
        // The normal case: nothing pending, no copy is made.
        return Ok(current(before.tables.len()));
    }
    let rows = row_counts(&conn, &before.tables).map_err(|e| context(path, e))?;
    let foreign = foreign_key_violations(&conn).map_err(|e| context(path, e))?;
    drop(conn);
    let (tables, rows) = rehearse(path, target, columns, apply, &rows, &foreign)?;
    Ok(Report {
        path: path.to_path_buf(),
        from: before.version,
        to: target,
        pending: before.missing,
        rehearsed: true,
        tables,
        rows: Some(rows),
    })
}

/// Check a tracker database (`genie migrate --check`).
pub fn check_tracker(path: &Path) -> Result<Report> {
    check(path, crate::db::SCHEMA_VERSION, crate::db::COLUMN_MIGRATIONS, &|p| crate::db::Db::open_migrated(p).map(|_| ()))
}

/// Check `server.db` (`genie migrate --check`).
pub fn check_server(path: &Path) -> Result<Report> {
    check(path, crate::server_db::SERVER_SCHEMA_VERSION, crate::server_db::SERVER_COLUMN_MIGRATIONS, &|p| {
        crate::server_db::open_migrated(p).map(|_| ())
    })
}

/// The schema version a database records: `None` when the file is not there, `0` when it
/// has no `meta` table (a tracker from before the version column).
pub fn version(path: &Path) -> Result<Option<i64>> {
    if !path.exists() {
        return Ok(None);
    }
    let conn = read_only(path)?;
    if !table_names(&conn)?.iter().any(|t| t == "meta") {
        return Ok(Some(0));
    }
    Ok(Some(recorded(&conn)?))
}

/// The trackers a data directory holds, read from `server.db` **without** opening it for
/// migration: `(slug, path of genie.db)`. Empty when `server.db` cannot be read.
pub fn trackers(data: &Path) -> Vec<(String, PathBuf)> {
    let Ok(conn) = read_only(&data.join("server.db")) else { return Vec::new() };
    let Ok(mut stmt) = conn.prepare("SELECT slug, tracker_dir FROM projects ORDER BY slug") else { return Vec::new() };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) else { return Vec::new() };
    rows.filter_map(|r| r.ok()).map(|(slug, dir)| (slug, Path::new(&dir).join("genie.db"))).collect()
}

/// One line for a report, named relative to the data directory it belongs to.
pub fn line(report: &Report, data: &Path) -> String {
    let name = report.path.strip_prefix(data).unwrap_or(&report.path).display();
    if !report.rehearsed && report.pending.is_empty() {
        return format!("{name}: schema {}, up to date", report.to);
    }
    let mut out = format!("{name}: schema {} → {}", report.from, report.to);
    if !report.pending.is_empty() {
        out.push_str(&format!(" ({})", report.pending.join(", ")));
    }
    if report.rehearsed {
        out.push_str(&format!(
            ", checked on a copy: ok (integrity ok, foreign keys ok, {} tables, {} rows unchanged)",
            report.tables,
            report.rows.unwrap_or(0)
        ));
    }
    out
}

/// [`line`] for every report, one per line.
pub fn lines(reports: &[Report], data: &Path) -> String {
    reports.iter().map(|r| line(r, data)).collect::<Vec<_>>().join("\n")
}

/// Remember what an open migrated, for the process (`serve` logs one line per database).
/// One entry per file: a later check of the same database replaces the earlier one.
pub fn remember(report: Report) {
    let mut all = reports().lock().unwrap_or_else(|e| e.into_inner());
    all.retain(|r| r.path != report.path);
    all.push(report);
}

/// Take (and clear) what this process migrated.
pub fn take_reports() -> Vec<Report> {
    std::mem::take(&mut *reports().lock().unwrap_or_else(|e| e.into_inner()))
}

/// Forget what this process migrated.
pub fn clear_reports() {
    reports().lock().unwrap_or_else(|e| e.into_inner()).clear();
}

fn reports() -> &'static Mutex<Vec<Report>> {
    static REPORTS: OnceLock<Mutex<Vec<Report>>> = OnceLock::new();
    REPORTS.get_or_init(|| Mutex::new(Vec::new()))
}

/// What a database looks like before or after a migration.
struct Survey {
    version: i64,
    tables: Vec<String>,
    /// Columns the target schema expects and the database lacks.
    missing: Vec<String>,
}

fn survey(conn: &Connection, columns: &[Column]) -> Result<Survey> {
    let tables = table_names(conn)?;
    let version = recorded(conn)?;
    let mut missing = Vec::new();
    for (table, column, _) in columns {
        // A table that is not there at all is created by the schema, with the column in it.
        if !tables.iter().any(|t| t == table) {
            continue;
        }
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let has = stmt.query_map([], |r| r.get::<_, String>(1))?.filter_map(|c| c.ok()).any(|c| c == *column);
        if !has {
            missing.push(format!("{table}.{column}"));
        }
    }
    Ok(Survey { version, tables, missing })
}

/// The version a database records (`0` when there is no `meta` table or no row).
fn recorded(conn: &Connection) -> Result<i64> {
    let has_meta =
        conn.query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meta'", [], |_| Ok(())).optional()?.is_some();
    if !has_meta {
        return Ok(0);
    }
    Ok(conn.query_row("SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'schema'", [], |r| r.get(0)).optional()?.unwrap_or(0))
}

fn table_names(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?;
    Ok(stmt.query_map([], |r| r.get::<_, String>(0))?.filter_map(|r| r.ok()).collect())
}

fn row_counts(conn: &Connection, tables: &[String]) -> Result<BTreeMap<String, i64>> {
    let mut out = BTreeMap::new();
    for table in tables {
        let quoted = table.replace('"', "\"\"");
        let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM \"{quoted}\""), [], |r| r.get(0))?;
        out.insert(table.clone(), n);
    }
    Ok(out)
}

fn foreign_key_violations(conn: &Connection) -> Result<BTreeSet<String>> {
    let mut stmt = conn.prepare("PRAGMA foreign_key_check")?;
    let mut rows = stmt.query([])?;
    let mut out = BTreeSet::new();
    while let Some(row) = rows.next()? {
        // table, rowid, parent table, index of the foreign key.
        let table: String = row.get(0)?;
        let rowid: Option<i64> = row.get(1)?;
        let parent: String = row.get(2)?;
        let fk: Option<i64> = row.get(3)?;
        let row = rowid.map_or_else(|| "?".to_string(), |r| r.to_string());
        out.insert(format!("{table} #{row} → {parent} (fk {})", fk.unwrap_or(-1)));
    }
    Ok(out)
}

/// Rehearse the migration on a copy of the database and check the copy. The copy is
/// removed afterwards, whatever happens; the database itself is opened read-only.
fn rehearse(
    path: &Path,
    target: i64,
    columns: &[Column],
    apply: &dyn Fn(&Path) -> Result<()>,
    rows_before: &BTreeMap<String, i64>,
    foreign_before: &BTreeSet<String>,
) -> Result<(usize, u64)> {
    let dir = std::env::temp_dir().join(format!("genie-migrate-{}-{}", std::process::id(), unique()));
    std::fs::create_dir_all(&dir).map_err(|e| {
        GenieError::invalid(format!("cannot make a rehearsal directory {}: {e}; the database was not touched", dir.display()))
    })?;
    let _copy = Copy(dir.clone());
    let copy = dir.join("genie.db");
    // `VACUUM INTO` copies a live WAL database consistently; a damaged one fails here, with the source intact.
    {
        let conn = read_only(path).map_err(|e| context(path, e))?;
        conn.execute("VACUUM INTO ?1", [copy.to_string_lossy().as_ref()]).map_err(|e| {
            GenieError::invalid(format!("{}: cannot copy it for a rehearsal ({e}); the database was not touched", path.display()))
        })?;
    }
    {
        let conn = read_only(&copy).map_err(|e| context(path, e))?;
        let verdict: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0)).map_err(|e| context(path, e))?;
        if verdict != "ok" {
            return Err(GenieError::invalid(format!(
                "{}: its copy is damaged ({verdict}); fix or restore the database first",
                path.display()
            )));
        }
    }
    // Exactly the code the update runs, on the copy.
    apply(&copy)
        .map_err(|e| GenieError::invalid(format!("{}: the rehearsal failed ({e}); the database was not touched", path.display())))?;
    let conn = read_only(&copy).map_err(|e| context(path, e))?;
    let after = survey(&conn, columns).map_err(|e| context(path, e))?;
    if after.version != target {
        return Err(GenieError::invalid(format!(
            "{}: the rehearsal left the copy at schema {}, not {target}",
            path.display(),
            after.version
        )));
    }
    let foreign_after = foreign_key_violations(&conn).map_err(|e| context(path, e))?;
    let new: Vec<&String> = foreign_after.difference(foreign_before).collect();
    if !new.is_empty() {
        let shown: Vec<&str> = new.iter().take(3).map(|v| v.as_str()).collect();
        return Err(GenieError::invalid(format!(
            "{}: the rehearsal would break {} foreign key reference(s) ({}); the database was not touched",
            path.display(),
            new.len(),
            shown.join(", ")
        )));
    }
    let rows_after = row_counts(&conn, &after.tables).map_err(|e| context(path, e))?;
    for (table, expected) in rows_before {
        let got = rows_after.get(table).copied().unwrap_or(0);
        if got != *expected {
            return Err(GenieError::invalid(format!(
                "{}: the rehearsal would change the row count of {table}: {expected} → {got}; the database was not touched",
                path.display()
            )));
        }
    }
    let rows: u64 = rows_after.values().map(|n| (*n).max(0) as u64).sum();
    Ok((after.tables.len(), rows))
}

/// A rehearsal directory, removed when it goes out of scope — errors included.
struct Copy(PathBuf);

impl Drop for Copy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Open a database for reading without writing anything: the command must not touch the data
/// directory (the drill asserts it is byte-identical afterwards). With no `-wal` beside it the
/// file is complete, so it is opened *immutable*: SQLite then creates neither `-shm` nor `-wal`.
/// With a WAL beside it — a running server, or an unclean stop — the ordinary read-only open is
/// used, so the committed WAL is read as well; the sidecar files the server itself uses may then
/// be created, and a `-wal` without its `-shm` can fail with a recovery error instead of a report.
fn read_only(path: &Path) -> Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY;
    let conn = if sibling(path, "-wal").exists() {
        Connection::open_with_flags(path, flags)?
    } else {
        let uri = format!("file:{}?immutable=1", escape_uri(&path.to_string_lossy()));
        Connection::open_with_flags(uri, flags | OpenFlags::SQLITE_OPEN_URI)?
    };
    conn.busy_timeout(Duration::from_secs(30))?;
    Ok(conn)
}

/// `path` with a suffix on the file name (`genie.db` + `-wal`).
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The characters a SQLite URI cannot carry as they are.
fn escape_uri(path: &str) -> String {
    path.replace('%', "%25").replace('?', "%3F").replace('#', "%23")
}

/// An error about a database file, naming it.
fn context(path: &Path, err: impl Into<GenieError>) -> GenieError {
    GenieError::invalid(format!("{}: {}", path.display(), err.into()))
}

fn refused(path: &Path, recorded: i64, known: i64) -> String {
    format!(
        "{} records schema {recorded}; this genie ({}) understands {known}: restore a backup made before the update, or run the newer genie",
        path.display(),
        env!("CARGO_PKG_VERSION")
    )
}

fn unique() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}
