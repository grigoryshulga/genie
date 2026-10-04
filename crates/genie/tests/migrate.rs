//! Versioned migrations and the update/rollback guard: a pending migration is rehearsed on
//! a copy before the live database is touched, data recorded as newer than the binary is
//! refused, and `restore` refuses a backup written by a newer genie.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use genie::config::Config;
use genie_core::Tracker;
use genie_core::db::SCHEMA_VERSION;
use genie_core::migrate;
use genie_core::server_db::{SERVER_SCHEMA_VERSION, ServerDb};
use rusqlite::Connection;

/// Every file under `dir`, by relative path: what "the source is untouched" means.
fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.insert(p.strip_prefix(dir).unwrap().to_string_lossy().into_owned(), std::fs::read(&p).unwrap());
            }
        }
    }
    out
}

/// What changed between two directory snapshots, in words (never the file contents).
fn diff(after: &BTreeMap<String, Vec<u8>>, before: &BTreeMap<String, Vec<u8>>) -> String {
    let added: Vec<_> = after.keys().filter(|k| !before.contains_key(*k)).collect();
    let removed: Vec<_> = before.keys().filter(|k| !after.contains_key(*k)).collect();
    let changed: Vec<_> = after.iter().filter(|(k, v)| before.get(*k).is_some_and(|b| b != *v)).map(|(k, _)| k).collect();
    format!("added {added:?}, removed {removed:?}, changed {changed:?}")
}

/// What version a database records, whatever the code would write.
fn set_version(file: &Path, version: i64) {
    let conn = Connection::open(file).unwrap();
    conn.execute("DELETE FROM meta WHERE key = 'schema'", []).unwrap();
    conn.execute("INSERT INTO meta(key, value) VALUES ('schema', ?1)", [version.to_string()]).unwrap();
}

fn count(file: &Path, sql: &str) -> i64 {
    Connection::open(file).unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
}

/// A tracker with rows, in the shape of schema 4 — the schema before `tasks.assignee` was
/// added, which is exactly the current one without that column.
fn legacy_tracker(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    drop(Tracker::init(dir, Some("T"), Some("legacy")).unwrap());
    let file = dir.join("genie.db");
    let conn = Connection::open(&file).unwrap();
    conn.execute_batch(
        "INSERT INTO tasks(id, seq, title, type, status, priority, created, updated)
           VALUES ('T-1', 1, 'Old task', 'task', 'done', 2, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z'),
                  ('T-2', 2, 'Another', 'bug', 'inbox', 1, '2026-01-02T00:00:00.000Z', '2026-01-02T00:00:00.000Z');
         INSERT INTO comments(task, at, author, role, kind, text)
           VALUES ('T-1', '2026-01-01T01:00:00.000Z', 'someone', 'owner', 'note', 'hello');
         INSERT INTO teams(id, task, cwd, state, created, updated)
           VALUES ('team-1', 'T-1', '/tmp', 'done', '2026-01-01T01:00:00.000Z', '2026-01-01T01:00:00.000Z');
         INSERT INTO mail(at, sender, sender_role, recipient, text, kind)
           VALUES ('2026-01-01T02:00:00.000Z', 'a', 'executor', 'b', 'hi', 'note');
         ALTER TABLE tasks DROP COLUMN assignee;",
    )
    .unwrap();
    drop(conn);
    set_version(&file, 4);
    file
}

#[test]
fn an_old_tracker_migrates_on_a_copy_and_keeps_every_row() {
    let dir = tempfile::tempdir().unwrap();
    let file = legacy_tracker(&dir.path().join("tracker"));
    let tasks_before = count(&file, "SELECT COUNT(*) FROM tasks");
    migrate::clear_reports();
    drop(Tracker::open(file.parent().unwrap()).unwrap());

    assert_eq!(migrate::version(&file).unwrap(), Some(SCHEMA_VERSION));
    assert_eq!(count(&file, "SELECT COUNT(*) FROM tasks"), tasks_before);
    assert_eq!(count(&file, "SELECT COUNT(*) FROM comments"), 1);
    assert_eq!(count(&file, "SELECT COUNT(*) FROM mail"), 1);
    assert_eq!(
        Connection::open(&file).unwrap().query_row("SELECT assignee FROM tasks WHERE id = 'T-1'", [], |r| r.get::<_, String>(0)).unwrap(),
        "",
        "the column is there and empty"
    );

    let reports = migrate::take_reports();
    assert_eq!(reports.len(), 1, "one line per database: {reports:?}");
    let report = &reports[0];
    assert!(report.rehearsed, "the migration was not checked on a copy: {report:?}");
    assert_eq!((report.from, report.to), (4, SCHEMA_VERSION));
    assert!(report.rows.unwrap_or(0) >= 4, "the rows are counted: {report:?}");
}

#[test]
fn checking_writes_nothing_to_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let file = legacy_tracker(&dir.path().join("tracker"));
    let before = tree(dir.path());

    let report = migrate::check_tracker(&file).unwrap();
    assert!(report.rehearsed);
    assert_eq!((report.from, report.to), (4, SCHEMA_VERSION));
    assert_eq!(report.pending, vec!["tasks.assignee"]);

    assert_eq!(tree(dir.path()), before, "the check touched the data directory: {}", diff(&tree(dir.path()), &before));
    assert!(dir.path().join("tracker").read_dir().unwrap().all(|e| !e.unwrap().path().to_string_lossy().contains("genie-migrate")));
    assert_eq!(migrate::version(&file).unwrap(), Some(4), "the check migrated the database");
}

#[test]
fn a_damaged_database_stops_the_update_and_names_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = legacy_tracker(&dir.path().join("tracker"));
    let mut bytes = std::fs::read(&file).unwrap();
    let (from, to) = (bytes.len() / 3, bytes.len() / 2);
    bytes[from..to].fill(0xFF);
    std::fs::write(&file, &bytes).unwrap();
    let before = std::fs::read(&file).unwrap();

    let err = migrate::check_tracker(&file).unwrap_err().to_string();
    assert!(err.contains("genie.db"), "the error names the file: {err}");
    assert_eq!(std::fs::read(&file).unwrap(), before, "the damaged database was written to");
    assert!(Tracker::open(file.parent().unwrap()).is_err(), "opening a damaged tracker must fail, not migrate it");
}

#[test]
fn data_from_a_newer_genie_is_refused_and_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let file = legacy_tracker(&dir.path().join("tracker"));
    set_version(&file, SCHEMA_VERSION + 1);
    let before = tree(dir.path());

    let err = migrate::check_tracker(&file).unwrap_err().to_string();
    assert!(err.contains(&format!("records schema {}", SCHEMA_VERSION + 1)), "{err}");
    assert!(err.contains("restore a backup"), "the error says what to do: {err}");
    assert!(Tracker::open(file.parent().unwrap()).is_err());
    assert_eq!(tree(dir.path()), before, "the newer database was written to: {}", diff(&tree(dir.path()), &before));

    let data = tempfile::tempdir().unwrap();
    let server = data.path().join("server.db");
    drop(ServerDb::open(&server).unwrap());
    set_version(&server, SERVER_SCHEMA_VERSION + 1);
    let before = tree(data.path());
    let err = migrate::check_server(&server).unwrap_err().to_string();
    assert!(err.contains(&format!("records schema {}", SERVER_SCHEMA_VERSION + 1)), "{err}");
    assert!(ServerDb::open(&server).is_err());
    assert_eq!(tree(data.path()), before, "the newer server database was written to: {}", diff(&tree(data.path()), &before));
}

#[test]
fn a_missing_column_with_a_current_version_is_detected_and_repaired() {
    let dir = tempfile::tempdir().unwrap();
    let file = legacy_tracker(&dir.path().join("tracker"));
    // An interrupted migration or an old copy: the version says 5, the column is gone.
    Connection::open(&file).unwrap().execute_batch("ALTER TABLE tasks ADD COLUMN assignee TEXT NOT NULL DEFAULT ''").unwrap();
    set_version(&file, SCHEMA_VERSION);
    Connection::open(&file).unwrap().execute_batch("ALTER TABLE tasks DROP COLUMN assignee").unwrap();
    let tasks_before = count(&file, "SELECT COUNT(*) FROM tasks");

    let report = migrate::check_tracker(&file).unwrap();
    assert_eq!(report.pending, vec!["tasks.assignee"], "{report:?}");
    assert_eq!(report.from, SCHEMA_VERSION, "the version alone would hide it");
    drop(Tracker::open(file.parent().unwrap()).unwrap());
    assert_eq!(migrate::version(&file).unwrap(), Some(SCHEMA_VERSION));
    assert_eq!(count(&file, "SELECT COUNT(*) FROM tasks"), tasks_before);
    assert_eq!(count(&file, "SELECT COUNT(DISTINCT name) FROM pragma_table_info('tasks') WHERE name = 'assignee'"), 1);
}

#[test]
fn a_backup_records_its_schemas_and_restore_refuses_a_newer_one() {
    let data = tempfile::tempdir().unwrap();
    let server = ServerDb::open(&data.path().join("server.db")).unwrap();
    let tracker_dir = data.path().join("projects/shop");
    drop(Tracker::init(&tracker_dir, Some("T"), Some("shop")).unwrap());
    server.create_project("shop", "Shop", tracker_dir.to_str().unwrap(), None, None).unwrap();
    drop(server);

    let backups = tempfile::tempdir().unwrap();
    genie::cli::backup(data.path(), &Config::load(data.path()).unwrap(), backups.path()).unwrap();
    let backup = std::fs::read_dir(backups.path()).unwrap().flatten().map(|e| e.path()).next().unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(backup.join("MANIFEST.json")).unwrap()).unwrap();
    assert_eq!(manifest["schemas"]["server.db"], SERVER_SCHEMA_VERSION, "{manifest}");
    assert_eq!(manifest["schemas"]["projects/shop.db"], SCHEMA_VERSION, "{manifest}");

    // A backup written by a newer genie: refuse before anything is written.
    set_version(&backup.join("projects/shop.db"), SCHEMA_VERSION + 1);
    let target = tempfile::tempdir().unwrap();
    let err = genie::cli::restore(target.path(), &backup, false).unwrap_err();
    assert!(err.contains("newer genie"), "{err}");
    assert!(!target.path().join("server.db").exists(), "restore wrote into the target despite the refusal");
    assert!(!target.path().join("projects").exists(), "restore wrote a tracker despite the refusal");
}

#[test]
fn doctor_reports_the_schema_versions_and_survives_newer_data() {
    let data = tempfile::tempdir().unwrap();
    let server = data.path().join("server.db");
    drop(ServerDb::open(&server).unwrap());
    let cfg = Config::load(data.path()).unwrap();
    let agents = genie::agent_config::AgentConfig::load(data.path(), &cfg, None);

    let (text, _) = genie::doctor::print(&genie::doctor::run(data.path(), &cfg, &agents, None));
    assert!(text.contains(&format!("server.db schema {SERVER_SCHEMA_VERSION}")), "{text}");

    set_version(&server, SERVER_SCHEMA_VERSION + 1);
    let (text, failed) = genie::doctor::print(&genie::doctor::run(data.path(), &cfg, &agents, None));
    assert!(failed > 0, "{text}");
    assert!(text.contains(&format!("records schema {}", SERVER_SCHEMA_VERSION + 1)), "{text}");
    assert!(text.contains("restore a backup made before the update"), "{text}");
}
