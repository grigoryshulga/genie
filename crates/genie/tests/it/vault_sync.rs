//! The vault in sync with a git remote: people edit it in Obsidian (a clone of
//! the remote), the server and its agents write it too. The remote's commits are
//! imported as changesets and published by the system; branches under `changes/`
//! become open changesets. An overlap is a pending merge for the merge agent, not
//! a `.conflict` copy.

use crate::common;

use std::path::{Path, PathBuf};

use common::Harness;
use genie::vault_sync::SyncState;
use genie_core::changesets::{ChangesetFilter, ChangesetOrigin, ChangesetStatus};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// An Obsidian user: commit everything and push the branch.
fn push(clone: &Path, message: &str) {
    git(clone, &["add", "-A"]);
    git(clone, &["-c", "user.name=Анна", "-c", "user.email=anna@example.com", "commit", "-q", "-m", message]);
    git(clone, &["push", "-q", "origin", "main"]);
}

fn bare_remote(t: &tempfile::TempDir) -> String {
    let remote = t.path().join("vault.git");
    std::process::Command::new("git").args(["init", "-q", "--bare", "-b", "main"]).arg(&remote).status().unwrap();
    remote.to_string_lossy().into_owned()
}

fn harness(remote: &str) -> Harness {
    let h = Harness::with_config(|c| {
        c.vault.commit = Some(true);
        c.vault.remote = Some(remote.to_string());
        c.vault.branch = Some("main".into());
    });
    h.project("shop");
    h
}

fn sync(h: &Harness) -> SyncState {
    genie::vault_sync::sync(&h.app).unwrap().expect("the remote is configured")
}

fn vault_root(h: &Harness) -> PathBuf {
    h.app.with_vault(|v| Ok(v.root().to_path_buf())).unwrap()
}

fn clone_remote(remote: &str, into: &Path) {
    std::process::Command::new("git").args(["clone", "-q", "-b", "main"]).arg(remote).arg(into).status().unwrap();
}

/// The changesets the sync opened, newest first.
fn remote_changesets(h: &Harness) -> Vec<genie_core::changesets::Changeset> {
    h.app
        .with_server(|db| db.changesets(&ChangesetFilter { limit: 500, ..Default::default() }))
        .unwrap()
        .into_iter()
        .filter(|c| c.origin == ChangesetOrigin::Remote)
        .collect()
}

#[tokio::test]
async fn the_remote_commits_land_on_main_through_a_sync_changeset() {
    let t = tempfile::tempdir().unwrap();
    let remote = bare_remote(&t);
    let h = harness(&remote);
    let vault = vault_root(&h);

    // The empty remote takes the server's vault first.
    let st = sync(&h);
    assert!(st.ok && st.pushed > 0, "the server's vault goes to the empty remote: {st:?}");
    let clone = t.path().join("obsidian");
    clone_remote(&remote, &clone);
    assert!(clone.join("shop/changelog.md").exists());
    assert!(std::fs::read_to_string(clone.join(".gitignore")).unwrap().contains(".obsidian/workspace.json"));

    // Obsidian writes a page and pushes it: the server imports it as a changeset.
    write(&clone.join("shop/notes.md"), "# Заметки\n\nСклад работает до 22:00.\n");
    push(&clone, "notes from Obsidian");
    let theirs = git(&clone, &["rev-parse", "HEAD"]);
    let main_before = git(&vault, &["rev-parse", "HEAD"]);

    let st = sync(&h);

    // The import is a changeset whose branch stands on the remote's tip, owned by its author.
    let sync_changeset = remote_changesets(&h)
        .into_iter()
        .find(|c| c.title == format!("Sync from {remote}"))
        .unwrap_or_else(|| panic!("the sync opened a changeset for {remote}"));
    assert_eq!(sync_changeset.base, main_before);
    assert_eq!(sync_changeset.author, "anna", "the newest remote commit's author");
    assert_eq!(sync_changeset.author_kind, "human");
    assert!(sync_changeset.paths.contains(&"shop/notes.md".to_string()), "{sync_changeset:?}");
    assert_eq!(git(&vault, &["rev-parse", &format!("refs/heads/{}", sync_changeset.branch)]), theirs);

    assert!(st.ok, "{st:?}");
    assert_eq!(st.pulled, 1, "{st:?}");
    assert!(st.both.is_empty(), "{st:?}");
    assert!(st.merging.is_empty(), "{st:?}");
    assert!(vault.join("shop/notes.md").exists(), "Obsidian's page is on the server");
    git(&vault, &["merge-base", "--is-ancestor", &theirs, "HEAD"]);
    let published =
        remote_changesets(&h).into_iter().find(|c| c.id == sync_changeset.id).unwrap_or_else(|| panic!("the sync changeset still exists"));
    assert_eq!(published.status, ChangesetStatus::Published, "the system published it");
    assert_eq!(published.closed_by.as_deref(), Some("genie"));

    // And the server's own work reaches Obsidian.
    h.app
        .with_vault(|v| {
            v.write(
                "shop/server.md",
                "---\ntitle: С сервера\n---\n\nСтраница из веба.\n",
                genie_core::vault::AuthorKind::System,
                ("genie", "genie"),
                None,
                "server page",
            )
        })
        .unwrap();
    let st = sync(&h);
    assert!(st.ok, "{st:?}");
    git(&clone, &["pull", "-q", "origin", "main"]);
    assert!(clone.join("shop/server.md").exists(), "the server's page is in Obsidian");
}

#[tokio::test]
async fn overlapping_edits_become_a_pending_merge_and_hold_the_push() {
    let t = tempfile::tempdir().unwrap();
    let remote = bare_remote(&t);
    let h = harness(&remote);
    let vault = vault_root(&h);
    assert!(sync(&h).ok);
    let clone = t.path().join("obsidian");
    clone_remote(&remote, &clone);

    // The same line edited in both places.
    let page = std::fs::read_to_string(clone.join("shop/changelog.md")).unwrap();
    write(&clone.join("shop/changelog.md"), &page.replace("## Unreleased", "## Unreleased\n\n- Из Obsidian."));
    push(&clone, "a changelog line from Obsidian");
    let remote_main = git(&clone, &["rev-parse", "HEAD"]);
    h.app
        .with_vault(|v| {
            let version = std::fs::read_to_string(v.root().join("shop/changelog.md")).unwrap();
            v.write(
                "shop/changelog.md",
                &version.replace("## Unreleased", "## Unreleased\n\n- С сервера."),
                genie_core::vault::AuthorKind::Human,
                ("Анна", "anna"),
                None,
                "a changelog line from the server",
            )
        })
        .unwrap();

    let main_before = git(&vault, &["rev-parse", "HEAD"]);
    let st = sync(&h);

    // The same import: one changeset per sync run, rooted at the common ancestor.
    let sync_changesets: Vec<_> = remote_changesets(&h).into_iter().filter(|c| c.title == format!("Sync from {remote}")).collect();
    assert_eq!(sync_changesets.len(), 1, "one sync changeset for the run");
    assert_eq!(sync_changesets[0].base, git(&vault, &["merge-base", &main_before, &remote_main]));
    assert_eq!(git(&vault, &["rev-parse", &format!("refs/heads/{}", sync_changesets[0].branch)]), remote_main);

    assert!(st.ok, "{st:?}");
    assert_eq!(st.both, ["shop/changelog.md"], "{st:?}");
    assert_eq!(st.merging.len(), 1, "the merge agent has the overlap: {st:?}");
    assert!(
        st.conflicts.iter().any(|c| c.starts_with("shop/changelog.md: overlapping edits; the merge agent is resolving merge")),
        "{st:?}"
    );
    let copies: Vec<String> = std::fs::read_dir(vault.join("shop"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".conflict-"))
        .collect();
    assert!(copies.is_empty(), "no conflict copies are written any more: {copies:?}");
    assert_eq!(git(&vault, &["rev-parse", "HEAD"]), sync_commit_before(&h, &st), "main does not move while merging");
    assert_eq!(
        git(&clone, &["ls-remote", "origin", "refs/heads/main"]).split_whitespace().next(),
        Some(remote_main.as_str()),
        "main is not pushed while the merge is open"
    );

    // A later sync does not open a second sync changeset.
    let before = remote_changesets(&h).len();
    let st = sync(&h);
    assert!(st.ok, "{st:?}");
    assert_eq!(st.merging.len(), 1, "the same merge is still waited for: {st:?}");
    assert_eq!(remote_changesets(&h).len(), before, "no second sync changeset");
}

/// The commit the merge waits on: main's head when the sync reported a merge.
fn sync_commit_before(h: &Harness, st: &SyncState) -> String {
    let merge = h.app.with_server(|db| db.doc_merge(st.merging[0])).unwrap();
    merge.main
}

#[tokio::test]
async fn remote_change_branches_become_open_changesets() {
    let t = tempfile::tempdir().unwrap();
    let remote = bare_remote(&t);
    let h = harness(&remote);
    assert!(sync(&h).ok);
    let vault = vault_root(&h);
    let main = git(&vault, &["rev-parse", "HEAD"]);
    let clone = t.path().join("obsidian");
    clone_remote(&remote, &clone);

    // A person pushes a branch of their own instead of writing main.
    git(&clone, &["checkout", "-q", "-b", "changes/anna-1"]);
    write(&clone.join("shop/plan.md"), "# План\n\nЧерновик.\n");
    git(&clone, &["add", "-A"]);
    git(&clone, &["-c", "user.name=Анна", "-c", "user.email=anna@example.com", "commit", "-q", "-m", "draft the plan"]);
    let tip = git(&clone, &["rev-parse", "HEAD"]);
    git(&clone, &["push", "-q", "origin", "changes/anna-1"]);

    let st = sync(&h);
    assert!(st.ok, "{st:?}");
    let registered = remote_changesets(&h)
        .into_iter()
        .find(|c| c.branch == "changes/anna-1")
        .unwrap_or_else(|| panic!("the remote branch became a changeset"));
    assert_eq!(registered.status, ChangesetStatus::Open);
    assert_eq!(registered.author, "anna", "from the tip commit");
    assert_eq!(registered.base, main);
    assert_eq!(registered.paths, vec!["shop/plan.md"]);
    assert_eq!(git(&vault, &["rev-parse", "refs/heads/changes/anna-1"]), tip, "the local branch copies the remote tip");
    assert!(!vault.join("shop/plan.md").exists(), "an open changeset does not change main");

    // The branch moves on: the local one fast-forwards.
    write(&clone.join("shop/plan.md"), "# План\n\nЧерновик, дополнен.\n");
    git(&clone, &["add", "-A"]);
    git(&clone, &["-c", "user.name=Анна", "-c", "user.email=anna@example.com", "commit", "-q", "-m", "more of the plan"]);
    let moved = git(&clone, &["rev-parse", "HEAD"]);
    git(&clone, &["push", "-q", "origin", "changes/anna-1"]);
    let st = sync(&h);
    assert!(st.ok, "{st:?}");
    assert_eq!(git(&vault, &["rev-parse", "refs/heads/changes/anna-1"]), moved);
    assert_eq!(remote_changesets(&h).iter().filter(|c| c.branch == "changes/anna-1").count(), 1, "no duplicate changeset");
}

#[tokio::test]
async fn changeset_branches_are_pushed_and_closed_ones_deleted() {
    let t = tempfile::tempdir().unwrap();
    let remote = bare_remote(&t);
    let h = harness(&remote);
    assert!(sync(&h).ok);
    let vault = vault_root(&h);
    let main = git(&vault, &["rev-parse", "HEAD"]);
    let vgit = genie_core::vault_git::Git::open(&vault).unwrap();

    // An open changeset this server owns: its branch must reach the remote.
    let open = h
        .app
        .with_server(|db| {
            db.create_changeset(&genie_core::changesets::NewChangeset {
                branch: "",
                project: Some("shop"),
                task: None,
                author: "anna",
                author_name: "Анна",
                author_kind: "human",
                title: "Draft a page",
                base: &main,
                origin: ChangesetOrigin::Genie,
                reverts: None,
            })
        })
        .unwrap();
    let meta = genie_core::vault_git::CommitMeta {
        message: "draft a page".into(),
        author_name: "Анна".into(),
        author_email: "anna@example.com".into(),
        trailers: vec![("Genie-Changeset".into(), open.id.to_string())],
    };
    let commit = vgit.commit_tree(&main, &[&main], &[("shop/draft.md".to_string(), Some(b"# Draft\n".to_vec()))], &meta).unwrap();
    vgit.update_ref(&format!("refs/heads/{}", open.branch), &commit, None).unwrap();
    h.app.with_server(|db| db.touch_changeset(open.id, &["shop/draft.md".to_string()])).unwrap();

    // A changeset this server already closed, whose branch still sits on the remote.
    let closed = h
        .app
        .with_server(|db| {
            db.create_changeset(&genie_core::changesets::NewChangeset {
                branch: "",
                project: Some("shop"),
                task: None,
                author: "anna",
                author_name: "Анна",
                author_kind: "human",
                title: "Published already",
                base: &main,
                origin: ChangesetOrigin::Genie,
                reverts: None,
            })
        })
        .unwrap();
    let commit = vgit.commit_tree(&main, &[&main], &[("shop/done.md".to_string(), Some(b"# Done\n".to_vec()))], &meta).unwrap();
    vgit.update_ref(&format!("refs/heads/{}", closed.branch), &commit, None).unwrap();
    h.app.with_server(|db| db.mark_published(closed.id, &commit, false, "anna")).unwrap();
    git(&vault, &["push", "-q", "origin", &format!("refs/heads/{0}:refs/heads/{0}", closed.branch)]);

    let st = sync(&h);
    assert!(st.ok, "{st:?}");
    let heads = git(&vault, &["ls-remote", "--heads", "origin"]);
    assert!(heads.contains(&format!("refs/heads/{}", open.branch)), "the open branch is pushed:\n{heads}");
    assert!(!heads.contains(&format!("refs/heads/{}", closed.branch)), "the closed branch is deleted:\n{heads}");
}

#[tokio::test]
async fn a_sync_failure_is_told_to_the_admins_once() {
    let t = tempfile::tempdir().unwrap();
    let remote = bare_remote(&t);
    let h = harness(&remote);
    let admin = h.app.with_server(|db| db.create_user("anna", "Анна", None, Some("password-1"), true)).unwrap();
    let notes = || h.app.with_server(|db| db.notifications(admin.id, false, 20)).unwrap();
    assert!(sync(&h).ok);

    // The remote is gone: the server keeps its copy and tells the admins once.
    std::fs::rename(&remote, t.path().join("moved.git")).unwrap();
    for _ in 0..3 {
        let st = sync(&h);
        assert!(!st.ok && st.error.is_some(), "{st:?}");
    }
    let failures = notes().into_iter().filter(|n| n.title.contains("не удалась")).count();
    assert_eq!(failures, 1, "the same failure is told once");
    std::fs::rename(t.path().join("moved.git"), &remote).unwrap();
    assert!(sync(&h).ok);
}
