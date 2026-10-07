//! Closed tasks free their worktrees (G-134): work merged into the main branch
//! goes — the directory with its `target/` gigabytes and the branch — while
//! unmerged work stays, and the sweep the server runs at start catches tasks
//! closed before their worktree was cleaned.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use genie::config::Config;
use genie::state::App;
use genie::tasks::{self, Caller, CreateBody, StatusBody};
use genie_core::model::Worktree;
use genie_core::{Actor, Role, Status};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr));
}

fn project_with_repo() -> (tempfile::TempDir, Arc<App>, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init"]);
    let app = App::open(dir.path(), Config::load(dir.path()).unwrap(), PathBuf::from("/nonexistent")).unwrap();
    app.create_project("shop", "Shop", Some(repo.to_str().unwrap()), None, None).unwrap();
    (dir, app, repo)
}

/// A task with a worktree on `branch`, closed (`done`); `commit` puts work on the branch.
fn closed_task_with_worktree(app: &App, repo: &Path, title: &str, branch: &str, commit: Option<&str>) -> (String, PathBuf) {
    let anna = Caller::person("shop", "anna");
    let task = tasks::create(app, &anna, CreateBody { title: title.into(), ..Default::default() }).unwrap();
    let wt = repo.parent().unwrap().join("worktrees").join(branch);
    git(repo, &["worktree", "add", "-b", branch, wt.to_string_lossy().as_ref()]);
    if let Some(msg) = commit {
        std::fs::write(wt.join("work.txt"), "work").unwrap();
        git(&wt, &["add", "."]);
        git(&wt, &["commit", "-m", msg]);
    }
    app.with_tracker("shop", |t| {
        t.assign_team(
            &Actor::new("genie", Role::Orchestrator),
            &task.id,
            Some("t1"),
            Some(&Worktree { path: wt.to_string_lossy().into_owned(), branch: Some(branch.to_string()) }),
            None,
        )
    })
    .unwrap();
    tasks::set_status(app, &anna, &task.id, StatusBody { force: Some(true), ..StatusBody::to(Status::InProgress, None) }).unwrap();
    tasks::set_status(app, &anna, &task.id, StatusBody { force: Some(true), ..StatusBody::to(Status::Done, Some("finished".into())) })
        .unwrap();
    (task.id, wt)
}

#[test]
fn a_closed_task_frees_its_merged_worktree_and_branch() {
    let (_d, app, repo) = project_with_repo();
    let (id, wt) = closed_task_with_worktree(&app, &repo, "merged work", "genie/T-1", None);
    assert!(!wt.exists(), "the worktree directory is gone");
    let branch = git_ok(&repo, &["rev-parse", "--verify", "refs/heads/genie/T-1"]);
    assert!(!branch, "the merged branch is deleted");
    let cleared = app.with_tracker("shop", |t| t.get(&id)).unwrap().worktree;
    assert!(cleared.is_none(), "the task no longer names a worktree: {cleared:?}");
}

#[test]
fn unmerged_work_stays_until_it_is_in_the_main_branch() {
    let (_d, app, repo) = project_with_repo();
    let (id, wt) = closed_task_with_worktree(&app, &repo, "own commit", "genie/T-2", Some("work"));
    assert!(wt.exists(), "the worktree with unmerged work stays");
    assert!(git_ok(&repo, &["rev-parse", "--verify", "refs/heads/genie/T-2"]), "the branch stays");
    // Once the work lands in the main branch, the next sweep frees it.
    git(&repo, &["merge", "--no-ff", "-m", "merge T-2", "genie/T-2"]);
    let report = genie::runtime::sweep_worktrees(&app, "shop");
    assert!(report.iter().any(|l| l.contains(&id) && l.contains("removed")), "{report:?}");
    assert!(!wt.exists());
}

#[test]
fn the_start_sweep_catches_tasks_closed_earlier() {
    let (_d, app, repo) = project_with_repo();
    // A task closed long ago, its worktree recorded after the fact (the team is gone).
    let anna = Caller::person("shop", "anna");
    let task = tasks::create(&app, &anna, CreateBody { title: "old".into(), ..Default::default() }).unwrap();
    tasks::set_status(&app, &anna, &task.id, StatusBody { force: Some(true), ..StatusBody::to(Status::InProgress, None) }).unwrap();
    tasks::set_status(&app, &anna, &task.id, StatusBody { force: Some(true), ..StatusBody::to(Status::Done, Some("done".into())) })
        .unwrap();
    let wt = repo.parent().unwrap().join("worktrees").join("genie/T-3");
    git(&repo, &["worktree", "add", "-b", "genie/T-3", wt.to_string_lossy().as_ref()]);
    app.with_tracker("shop", |t| {
        t.assign_team(
            &Actor::new("genie", Role::Orchestrator),
            &task.id,
            Some("t3"),
            Some(&Worktree { path: wt.to_string_lossy().into_owned(), branch: Some("genie/T-3".into()) }),
            None,
        )
    })
    .unwrap();
    let report = genie::runtime::sweep_worktrees(&app, "shop");
    assert!(report.iter().any(|l| l.contains(&task.id) && l.contains("removed")), "{report:?}");
    assert!(!wt.exists(), "freed by the sweep the server runs at start");
}

fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git").arg("-C").arg(dir).args(args).output().is_ok_and(|o| o.status.success())
}
