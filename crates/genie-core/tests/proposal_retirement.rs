//! Upgrade and retirement preserve legacy documentation records without reviving their write API.
use genie_core::server_db::{SERVER_SCHEMA_VERSION, ServerDb};
use rusqlite::params;

const LEGACY_SCHEMA: &str = "CREATE TABLE proposals (
    id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT NOT NULL, base_hash TEXT, content TEXT NOT NULL,
    author TEXT NOT NULL, author_kind TEXT NOT NULL, project TEXT, task TEXT, note TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL, created TEXT NOT NULL, decided_by TEXT, decided_at TEXT, decision_note TEXT
);";

#[test]
fn a_fresh_server_never_creates_the_retired_live_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    for _ in 0..2 {
        let db = ServerDb::open(&path).unwrap();
        assert!(!db.has_legacy_proposals().unwrap());
        assert!(db.legacy_proposals().unwrap().is_empty());
        assert!(db.retire_legacy_proposals().unwrap());
    }
}

#[test]
fn schema_15_upgrade_retains_pending_work_and_archives_decisions_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.db");
    {
        let db = ServerDb::open(&path).unwrap();
        db.conn()
            .execute_batch(
                "DROP TABLE doc_proposal_migrations; DROP TABLE doc_proposal_archive;
            UPDATE meta SET value = '15' WHERE key = 'schema';",
            )
            .unwrap();
        db.conn().execute_batch(LEGACY_SCHEMA).unwrap();
        for (id, status) in [(1, "open"), (2, "approved"), (3, "rejected"), (4, "migrated")] {
            db.conn()
                .execute(
                    "INSERT INTO proposals VALUES (?1, 'shop/page.md', 'hash', 'exact content', 'author', 'agent', 'shop', 'G-1',
                 'reason', ?2, '2026-01-01T00:00:00Z', 'owner', '2026-01-02T00:00:00Z', 'decision')",
                    params![id, status],
                )
                .unwrap();
        }
    }
    let check = genie_core::migrate::check_server(&path).unwrap();
    assert_eq!((check.from, check.to), (15, SERVER_SCHEMA_VERSION));
    assert!(check.rehearsed);
    let db = ServerDb::open(&path).unwrap();
    assert_eq!(db.legacy_proposals().unwrap().len(), 1);
    assert!(!db.retire_legacy_proposals().unwrap(), "no retirement while a draft lacks its changeset");
    assert_eq!(db.conn().query_row("SELECT count(*) FROM proposals", [], |r| r.get::<_, i64>(0)).unwrap(), 4);
    // Model the successful git conversion; the server integration tests exercise that boundary.
    db.conn().execute("UPDATE proposals SET status = 'migrated' WHERE id = 1", []).unwrap();
    db.conn()
        .execute_batch(
            "CREATE TRIGGER fail_archive BEFORE INSERT ON doc_proposal_archive WHEN NEW.id = 3
        BEGIN SELECT RAISE(ABORT, 'injected archive failure'); END;",
        )
        .unwrap();
    assert!(db.retire_legacy_proposals().is_err());
    assert!(db.has_legacy_proposals().unwrap());
    assert_eq!(
        db.conn().query_row("SELECT count(*) FROM doc_proposal_archive", [], |r| r.get::<_, i64>(0)).unwrap(),
        0,
        "a partial archive rolls back with the original table retained"
    );
    db.conn().execute_batch("DROP TRIGGER fail_archive;").unwrap();
    assert!(db.retire_legacy_proposals().unwrap());
    assert!(!db.has_legacy_proposals().unwrap());
    for (id, status) in [(1, "migrated"), (2, "approved"), (3, "rejected"), (4, "migrated")] {
        let p = db.archived_proposal(id).unwrap();
        assert_eq!(p.path, "shop/page.md");
        assert_eq!(p.base_hash.as_deref(), Some("hash"));
        assert_eq!(p.content, "exact content");
        assert_eq!(p.author, "author");
        assert_eq!(p.author_kind, "agent");
        assert_eq!(p.project.as_deref(), Some("shop"));
        assert_eq!(p.task.as_deref(), Some("G-1"));
        assert_eq!(p.note, "reason");
        assert_eq!(p.status, status);
        assert_eq!(p.created, "2026-01-01T00:00:00Z");
        assert_eq!(p.decided_by.as_deref(), Some("owner"));
        assert_eq!(p.decided_at.as_deref(), Some("2026-01-02T00:00:00Z"));
        assert_eq!(p.decision_note.as_deref(), Some("decision"));
    }
    assert!(db.conn().execute("UPDATE doc_proposal_archive SET content = 'overwrite' WHERE id = 2", []).is_err());
    assert!(db.conn().execute("DELETE FROM doc_proposal_archive WHERE id = 2", []).is_err());
    drop(db);
    let reopened = ServerDb::open(&path).unwrap();
    assert!(!reopened.has_legacy_proposals().unwrap(), "reopening must not recreate the retired table");
    assert!(reopened.retire_legacy_proposals().unwrap());
    assert_eq!(reopened.archived_proposal(2).unwrap().content, "exact content");
}

#[test]
fn pending_drafts_are_not_limited_to_the_old_feed_page_size() {
    let dir = tempfile::tempdir().unwrap();
    let db = ServerDb::open(&dir.path().join("server.db")).unwrap();
    db.conn().execute_batch(LEGACY_SCHEMA).unwrap();
    db.tx(|| {
        for id in 1..=1001 {
            db.conn().execute(
                "INSERT INTO proposals(id, path, content, author, author_kind, status, created)
                VALUES (?1, 'shop/page.md', 'draft', 'author', 'agent', 'open', '2026-01-01T00:00:00Z')",
                [id],
            )?;
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(db.legacy_proposals().unwrap().len(), 1001);
    assert!(!db.retire_legacy_proposals().unwrap());
}
