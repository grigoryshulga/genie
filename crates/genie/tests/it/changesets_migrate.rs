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
            db.create_proposal(
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

    let decided = h.app.with_server(|db| db.proposal(proposal.id)).unwrap();
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
            db.create_proposal(
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
            db.create_proposal("shop/notes.md", None, &page("Версия из предложения."), "anna", "human", Some("shop"), None, "")
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
