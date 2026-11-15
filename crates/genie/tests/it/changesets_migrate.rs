//! One-time migration of open proposals and `.conflict` copies onto changesets.

use crate::common;

use std::path::{Path, PathBuf};

use common::Harness;
use genie_core::changesets::{ChangesetFilter, ChangesetOrigin, ChangesetStatus};
use genie_core::vault::AuthorKind;
use sha2::{Digest, Sha256};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["-c", "user.name=Анна", "-c", "user.email=anna@example.com", "commit", "-q", "-m", message]);
}

fn sha_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn harness() -> Harness {
    let h = Harness::with_config(|cfg| cfg.vault.commit = Some(true));
    h.project("shop");
    h
}

fn vault_root(h: &Harness) -> PathBuf {
    h.app.with_vault(|v| Ok(v.root().to_path_buf())).unwrap()
}

fn changesets(h: &Harness) -> Vec<genie_core::changesets::Changeset> {
    h.app.with_server(|db| db.changesets(&ChangesetFilter { limit: 500, ..Default::default() })).unwrap()
}

fn page(version: &str) -> String {
    format!("---\ntitle: Заметки\ntype: note\nstatus: current\n---\n\n# Заметки\n\n{version}\n")
}

// A pre-changeset database fixture, intentionally independent of the current schema.
const LEGACY_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS proposals (
    id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT NOT NULL, base_hash TEXT, content TEXT NOT NULL,
    author TEXT NOT NULL, author_kind TEXT NOT NULL, project TEXT, task TEXT, note TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL, created TEXT NOT NULL, decided_by TEXT, decided_at TEXT, decision_note TEXT
);";

#[allow(clippy::too_many_arguments)]
fn create_proposal(
    db: &genie_core::server_db::ServerDb,
    path: &str,
    base_hash: Option<&str>,
    content: &str,
    author: &str,
    author_kind: &str,
    project: Option<&str>,
    task: Option<&str>,
    note: &str,
) -> genie_core::Result<genie_core::work::Proposal> {
    db.conn().execute_batch(LEGACY_SCHEMA)?;
    db.conn().execute(
        "INSERT INTO proposals(path, base_hash, content, author, author_kind, project, task, note, status, created)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'open', ?9)",
        rusqlite::params![path, base_hash, content, author, author_kind, project, task, note, genie_core::db::now()],
    )?;
    Ok(db.legacy_proposals()?.pop().unwrap())
}

#[test]
fn an_open_proposal_forks_from_the_commit_its_base_hash_names() {
    let h = harness();
    let vault = vault_root(&h);
    let v1 = page("Версия один.");
    let v2 = page("Версия два.");
    for text in [&v1, &v2] {
        h.app.with_vault(|v| v.write("shop/notes.md", text, AuthorKind::Human, ("Анна", "anna"), None, "write the notes")).unwrap();
    }
    let base = git(&vault, &["rev-parse", "HEAD^"]);
    let proposed = page("Версия из предложения.");
    let proposal = h
        .app
        .with_server(|db| {
            create_proposal(
                db,
                "shop/notes.md",
                Some(&sha_hex(v1.as_bytes())),
                &proposed,
                "anna",
                "human",
                Some("shop"),
                Some("G-1"),
                "предложенная версия",
            )
        })
        .unwrap();

    let result = genie::changesets_migrate::migrate_legacy(&h.app);

    let migrated = changesets(&h).into_iter().find(|c| c.origin == ChangesetOrigin::Migration).expect("a migration changeset");
    assert_eq!(migrated.status, ChangesetStatus::Open, "the author publishes it, not the migration");
    assert_eq!(migrated.base, base, "forked from the commit whose page still had the proposal's base hash");
    assert_eq!(migrated.author, "anna");
    assert_eq!(migrated.author_kind, "human");
    assert_eq!(migrated.task.as_deref(), Some("G-1"));
    assert_eq!(migrated.project.as_deref(), Some("shop"));
    assert_eq!(migrated.paths, vec!["shop/notes.md"]);

    let committed = git(&vault, &["show", &format!("{}:shop/notes.md", migrated.branch)]);
    assert_eq!(committed, proposed.trim_end(), "the proposal's text is the branch's version");
    let message = git(&vault, &["log", "-1", "--format=%B", &migrated.branch]);
    assert!(message.contains(&format!("Genie-Changeset: {}", migrated.id)), "{message}");
    assert!(message.contains("Genie-Task: G-1"), "{message}");
    assert!(message.contains("Genie-Author-Kind: human"), "{message}");
    assert!(message.contains("предложенная версия"), "{message}");

    let decided = h.app.with_server(|db| db.archived_proposal(proposal.id)).unwrap();
    assert_eq!(decided.status, "migrated");
    assert_eq!(decided.decided_by.as_deref(), Some("genie"));
    assert_eq!(decided.decision_note, Some(format!("changeset #{}", migrated.id)));

    assert!(result.is_ok(), "{:?}", result.err().map(|e| e.to_string()));
}

#[test]
fn a_proposal_without_a_matching_base_hash_forks_from_main() {
    let h = harness();
    let vault = vault_root(&h);
    h.app
        .with_vault(|v| v.write("shop/notes.md", &page("Версия один."), AuthorKind::Human, ("Анна", "anna"), None, "write the notes"))
        .unwrap();
    let main = git(&vault, &["rev-parse", "HEAD"]);
    h.app
        .with_server(|db| {
            create_proposal(
                db,
                "shop/notes.md",
                Some("0000000000000000000000000000000000000000000000000000000000000000"),
                &page("Версия из предложения."),
                "anna",
                "human",
                Some("shop"),
                None,
                "",
            )
        })
        .unwrap();

    let result = genie::changesets_migrate::migrate_legacy(&h.app);

    let migrated = changesets(&h).into_iter().find(|c| c.origin == ChangesetOrigin::Migration).expect("a migration changeset");
    assert_eq!(migrated.base, main, "no commit has that hash: the changeset forks from main");
    assert!(result.is_ok(), "{:?}", result.err().map(|e| e.to_string()));
}

#[test]
fn a_second_run_migrates_nothing_new() {
    let h = harness();
    h.app
        .with_vault(|v| v.write("shop/notes.md", &page("Версия один."), AuthorKind::Human, ("Анна", "anna"), None, "write the notes"))
        .unwrap();
    h.app
        .with_server(|db| {
            create_proposal(db, "shop/notes.md", None, &page("Версия из предложения."), "anna", "human", Some("shop"), None, "")
        })
        .unwrap();

    let first = genie::changesets_migrate::migrate_legacy(&h.app);
    let _ = first;
    let count = changesets(&h).len();
    let second = genie::changesets_migrate::migrate_legacy(&h.app);
    assert!(second.is_ok(), "the second run has nothing to do: {:?}", second.err().map(|e| e.to_string()));
    assert_eq!(changesets(&h).len(), count, "no changeset is opened twice");
    assert!(changesets(&h).iter().all(|c| c.origin == ChangesetOrigin::Migration), "only the migrated one");
}

#[test]
fn a_conflict_copy_becomes_a_changeset_for_its_page() {
    let h = harness();
    let vault = vault_root(&h);
    h.app
        .with_vault(|v| v.write("shop/notes.md", &page("Версия один."), AuthorKind::Human, ("Анна", "anna"), None, "write the notes"))
        .unwrap();
    h.app
        .with_vault(|v| v.write("shop/notes.md", &page("Версия два."), AuthorKind::Human, ("Анна", "anna"), None, "write the notes again"))
        .unwrap();
    let parent = git(&vault, &["rev-parse", "HEAD"]);
    let copy = "shop/notes.conflict-20260101-1200.md";
    let incoming = page("Версия из репозитория.");
    std::fs::write(vault.join(copy), &incoming).unwrap();
    commit(&vault, "the other version, next to the page");
    assert!(git(&vault, &["ls-files"]).contains(copy));

    let result = genie::changesets_migrate::migrate_legacy(&h.app);

    let migrated = changesets(&h)
        .into_iter()
        .find(|c| c.origin == ChangesetOrigin::Migration && c.paths.contains(&"shop/notes.md".to_string()))
        .expect("the copy became a changeset for its page");
    assert_eq!(migrated.base, parent, "forked from the commit before the copy was added");
    assert_eq!(git(&vault, &["show", &format!("{}:shop/notes.md", migrated.branch)]), incoming.trim_end());
    assert!(result.is_ok(), "{:?}", result.err().map(|e| e.to_string()));
}

#[test]
fn distinct_drafts_and_decided_records_survive_retirement_and_statistics() {
    let h = harness();
    let vault = vault_root(&h);
    h.app.with_vault(|v| v.write("shop/notes.md", &page("Original"), AuthorKind::Human, ("Anna", "anna"), None, "original")).unwrap();
    let mut proposals = Vec::new();
    for (content, task) in [("First draft", "G-1"), ("Second draft", "G-2"), ("Approved", "G-3"), ("Rejected", "G-4")] {
        proposals.push(
            h.app
                .with_server(|db| {
                    create_proposal(db, "shop/notes.md", None, &page(content), "anna", "human", Some("shop"), Some(task), "reason")
                })
                .unwrap(),
        );
    }
    h.app.with_server(|db| {
        for (p, status) in [(&proposals[2], "approved"), (&proposals[3], "rejected")] {
            db.conn().execute("UPDATE proposals SET status = ?1, decided_by = 'owner', decided_at = '2026-01-02T03:04:05Z', decision_note = 'decision' WHERE id = ?2", rusqlite::params![status, p.id])?;
        }
        Ok(())
    }).unwrap();
    let before = genie::stats::collect(&h.app.data, 7, Some("shop"), &Default::default()).unwrap();
    assert_eq!((before.projects[0].proposals, before.projects[0].proposals_approved, before.projects[0].proposals_rejected), (4, 1, 1));

    genie::changesets_migrate::migrate_legacy(&h.app).unwrap();
    let migrated = changesets(&h);
    assert_eq!(migrated.len(), 2, "same author and page must retain two distinct drafts");
    for proposal in &proposals[..2] {
        let cs = migrated.iter().find(|cs| cs.task == proposal.task).unwrap();
        assert_eq!(git(&vault, &["show", &format!("{}:{}", cs.branch, proposal.path)]), proposal.content.trim_end());
    }
    h.app
        .with_server(|db| {
            assert!(!db.has_legacy_proposals()?);
            for proposal in &proposals[2..] {
                let archived = db.archived_proposal(proposal.id)?;
                assert_eq!(archived.content, proposal.content);
                assert_eq!(archived.author, proposal.author);
                assert_eq!(archived.task, proposal.task);
                assert_eq!(archived.decided_by.as_deref(), Some("owner"));
                assert_eq!(archived.decided_at.as_deref(), Some("2026-01-02T03:04:05Z"));
                assert_eq!(archived.decision_note.as_deref(), Some("decision"));
            }
            Ok(())
        })
        .unwrap();
    let after = genie::stats::collect(&h.app.data, 7, Some("shop"), &Default::default()).unwrap();
    assert_eq!((after.projects[0].proposals, after.projects[0].proposals_approved, after.projects[0].proposals_rejected), (4, 1, 1));
    genie::changesets_migrate::migrate_legacy(&h.app).unwrap();
    assert_eq!(changesets(&h).len(), 2);
}

#[test]
fn a_failed_database_acknowledgement_resumes_the_committed_draft_after_restart() {
    resume_committed_draft(false);
}

#[test]
fn an_interrupted_pre_receipt_migration_adopts_its_exact_changeset_after_upgrade() {
    resume_committed_draft(true);
}

fn resume_committed_draft(pre_receipt: bool) {
    let h = harness();
    let vault = vault_root(&h);
    h.app.with_vault(|v| v.write("shop/notes.md", &page("Original"), AuthorKind::Human, ("Anna", "anna"), None, "original")).unwrap();
    let proposal = h
        .app
        .with_server(|db| {
            let p = create_proposal(db, "shop/notes.md", None, &page("Draft"), "anna", "human", Some("shop"), Some("G-1"), "reason")?;
            db.conn().execute_batch(
                "CREATE TRIGGER fail_ack BEFORE UPDATE ON proposals BEGIN SELECT RAISE(ABORT, 'injected acknowledgement failure'); END;",
            )?;
            Ok(p)
        })
        .unwrap();
    assert!(genie::changesets_migrate::migrate_legacy(&h.app).is_err());
    let first = changesets(&h);
    assert_eq!(first.len(), 1);
    assert_eq!(git(&vault, &["show", &format!("{}:shop/notes.md", first[0].branch)]), proposal.content.trim_end());
    h.app
        .with_server(|db| {
            assert!(db.has_legacy_proposals()?);
            assert_eq!(db.legacy_proposals()?.len(), 1);
            assert!(db.archived_proposal(proposal.id).is_err());
            db.conn().execute_batch("DROP TRIGGER fail_ack;")?;
            if pre_receipt {
                // Older binaries left the branch and changeset but had no migration receipt.
                db.conn().execute("DELETE FROM doc_proposal_migrations", [])?;
            }
            Ok(())
        })
        .unwrap();

    let restarted = genie::state::App::open(&h.app.data, h.app.cfg.clone(), PathBuf::from("/nonexistent-web")).unwrap();
    genie::changesets_migrate::migrate_legacy(&restarted).unwrap();
    assert_eq!(changesets(&h).len(), 1, "restart must acknowledge the existing branch, not allocate another");
    restarted
        .with_server(|db| {
            assert!(!db.has_legacy_proposals()?);
            assert_eq!(db.archived_proposal(proposal.id)?.status, "migrated");
            Ok(())
        })
        .unwrap();
}

#[test]
fn a_failed_git_ref_write_keeps_the_draft_and_retries_the_same_changeset() {
    let h = harness();
    let vault = vault_root(&h);
    h.app.with_vault(|v| v.write("shop/notes.md", &page("Original"), AuthorKind::Human, ("Anna", "anna"), None, "original")).unwrap();
    let proposal = h
        .app
        .with_server(|db| create_proposal(db, "shop/notes.md", None, &page("Draft"), "anna", "human", Some("shop"), None, "reason"))
        .unwrap();
    let lock = vault.join(".git/refs/heads/changes/1.lock");
    std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
    std::fs::write(&lock, "held by another writer").unwrap();
    assert!(genie::changesets_migrate::migrate_legacy(&h.app).is_err());
    assert_eq!(changesets(&h).len(), 1);
    h.app
        .with_server(|db| {
            assert_eq!(db.legacy_proposals()?[0].content, proposal.content);
            assert!(db.archived_proposal(proposal.id).is_err());
            Ok(())
        })
        .unwrap();
    std::fs::remove_file(lock).unwrap();
    genie::changesets_migrate::migrate_legacy(&h.app).unwrap();
    assert_eq!(changesets(&h).len(), 1);
    assert!(!h.app.with_server(|db| db.has_legacy_proposals()).unwrap());
}

fn dated_commit(root: &Path, message: &str, at: &str) {
    git(root, &["add", "-A"]);
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "user.name=Anna", "-c", "user.email=anna@example.com", "commit", "-q", "-m", message])
        .env("GIT_AUTHOR_DATE", at)
        .env("GIT_COMMITTER_DATE", at)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn unmatched_hash_uses_main_at_creation_and_missing_history_preserves_the_draft() {
    let h = harness();
    let vault = vault_root(&h);
    std::fs::write(vault.join("shop/notes.md"), page("At creation")).unwrap();
    dated_commit(&vault, "original", "2020-01-01T00:00:00Z");
    let base = git(&vault, &["rev-parse", "HEAD"]);
    std::fs::write(vault.join("shop/notes.md"), page("Written later")).unwrap();
    dated_commit(&vault, "later", "2022-01-01T00:00:00Z");
    let proposal = h
        .app
        .with_server(|db| {
            let p = create_proposal(db, "shop/notes.md", Some("unmatched"), &page("Draft"), "anna", "human", Some("shop"), None, "reason")?;
            db.conn().execute("UPDATE proposals SET created = '1900-01-01T00:00:00Z' WHERE id = ?1", [p.id])?;
            Ok(p)
        })
        .unwrap();
    assert!(genie::changesets_migrate::migrate_legacy(&h.app).is_err());
    assert!(changesets(&h).is_empty());
    h.app
        .with_server(|db| {
            assert_eq!(db.legacy_proposals()?[0].content, proposal.content);
            db.conn().execute("UPDATE proposals SET created = '2021-01-01T00:00:00Z' WHERE id = ?1", [proposal.id])?;
            Ok(())
        })
        .unwrap();
    genie::changesets_migrate::migrate_legacy(&h.app).unwrap();
    assert_eq!(changesets(&h)[0].base, base);
}

#[test]
fn a_matching_base_older_than_two_hundred_page_edits_is_still_used() {
    use std::io::Write;
    use std::process::Stdio;

    let h = harness();
    let vault = vault_root(&h);
    let original = page("Original base");
    h.app.with_vault(|v| v.write("shop/notes.md", &original, AuthorKind::Human, ("Anna", "anna"), None, "original")).unwrap();
    let base = git(&vault, &["rev-parse", "HEAD"]);
    let branch = git(&vault, &["symbolic-ref", "HEAD"]);
    // Fast-import builds realistic long history without hundreds of process launches.
    let mut input = String::new();
    for i in 0..201 {
        let content = page(&format!("Later edit {i}"));
        input.push_str(&format!("commit {branch}\ncommitter Anna <anna@example.com> {} +0000\ndata 4\nedit\n", 1_800_000_000 + i));
        if i == 0 {
            input.push_str(&format!("from {base}\n"));
        }
        input.push_str(&format!("M 100644 inline shop/notes.md\ndata {}\n{content}\n", content.len()));
    }
    let mut child = std::process::Command::new("git")
        .arg("-C")
        .arg(&vault)
        .args(["fast-import", "--quiet"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    h.app
        .with_server(|db| {
            create_proposal(
                db,
                "shop/notes.md",
                Some(&sha_hex(original.as_bytes())),
                &page("Draft"),
                "anna",
                "human",
                Some("shop"),
                None,
                "reason",
            )
        })
        .unwrap();

    genie::changesets_migrate::migrate_legacy(&h.app).unwrap();
    assert_eq!(changesets(&h)[0].base, base);
}

#[test]
fn missing_git_retains_open_drafts_but_does_not_prevent_archiving_decided_records() {
    let h = Harness::new();
    h.project("shop");
    let proposal = h
        .app
        .with_server(|db| create_proposal(db, "shop/notes.md", None, &page("Draft"), "anna", "human", Some("shop"), None, "reason"))
        .unwrap();
    assert!(genie::changesets_migrate::migrate_legacy(&h.app).is_err());
    h.app
        .with_server(|db| {
            assert_eq!(db.legacy_proposals()?[0].content, proposal.content);
            db.conn().execute("UPDATE proposals SET status = 'rejected', decided_by = 'owner' WHERE id = ?1", [proposal.id])?;
            Ok(())
        })
        .unwrap();
    genie::changesets_migrate::migrate_legacy(&h.app).unwrap();
    h.app
        .with_server(|db| {
            assert!(!db.has_legacy_proposals()?);
            assert_eq!(db.archived_proposal(proposal.id)?.status, "rejected");
            Ok(())
        })
        .unwrap();
}
