//! Closed tasks free their worktrees (G-134) — never their work: what the main
//! branch has and nothing unsaved goes (the directory with its `target/` gigabytes
//! and the branch), while unmerged, uncommitted or unpushed work stays. A team
//! still working keeps its worktree, and the sweep the server runs at start
//! catches tasks closed before their worktree was cleaned.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use genie::config::Config;
use genie::state::App;
use genie::tasks::{self, Caller, CreateBody, StatusBody};
use genie_core::model::Worktree;
use genie_core::team::{NewMember, NewTeam};
use genie_core::{Actor, Role, Status, TeamState};

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

/// A task in progress with a worktree at `wt` on `branch`; `team` (when given)
/// exists in the tracker's bus and is the task's team. `commit` puts work on the branch.
fn prepared_task_with_worktree_at(
    app: &App,
    repo: &Path,
    wt: &Path,
    branch: Option<&str>,
    team: Option<&str>,
    commit: Option<&str>,
) -> (String, Caller) {
    let anna = Caller::person("shop", "anna");
    let task = tasks::create(app, &anna, CreateBody { title: "sweep".into(), ..Default::default() }).unwrap();
    if let Some(branch) = branch {
        git(repo, &["worktree", "add", "-b", branch, wt.to_string_lossy().as_ref()]);
        if let Some(msg) = commit {
            std::fs::write(wt.join("work.txt"), "work").unwrap();
            git(wt, &["add", "."]);
            git(wt, &["commit", "-m", msg]);
        }
    }
    if let Some(team) = team {
        app.with_tracker("shop", |t| {
            t.bus().create(
                "anna",
                "human",
                NewTeam {
                    id: team.into(),
                    task: task.id.clone(),
                    cwd: "/tmp".into(),
                    members: vec![] as Vec<NewMember>,
                    ..Default::default()
                },
            )
        })
        .unwrap();
    }
    app.with_tracker("shop", |t| {
        t.assign_team(
            &Actor::new("genie", Role::Orchestrator),
            &task.id,
            Some(team.unwrap_or("t1")),
            Some(&Worktree { path: wt.to_string_lossy().into_owned(), branch: branch.map(str::to_string) }),
            None,
        )
    })
    .unwrap();
    tasks::set_status(app, &anna, &task.id, StatusBody { force: Some(true), ..StatusBody::to(Status::InProgress, None) }).unwrap();
    (task.id, anna)
}

fn close_task(app: &App, anna: &Caller, id: &str) {
    tasks::set_status(app, anna, id, StatusBody { force: Some(true), ..StatusBody::to(Status::Done, Some("finished".into())) }).unwrap();
}

/// The common case: a closed task with a worktree of the project's repository.
fn closed_task_with_worktree(app: &App, repo: &Path, branch: &str, commit: Option<&str>) -> (String, Caller, PathBuf) {
    let wt = repo.parent().unwrap().join("worktrees").join(branch);
    let (id, anna) = prepared_task_with_worktree_at(app, repo, &wt, Some(branch), None, commit);
    close_task(app, &anna, &id);
    (id, anna, wt)
}

fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git").arg("-C").arg(dir).args(args).output().is_ok_and(|o| o.status.success())
}

#[test]
fn a_closed_task_frees_its_merged_worktree_and_branch() {
    let (_d, app, repo) = project_with_repo();
    let (id, _anna, wt) = closed_task_with_worktree(&app, &repo, "genie/T-1", None);
    assert!(!wt.exists(), "the worktree directory is gone");
    assert!(!git_ok(&repo, &["rev-parse", "--verify", "refs/heads/genie/T-1"]), "the merged branch is deleted");
    let cleared = app.with_tracker("shop", |t| t.get(&id)).unwrap().worktree;
    assert!(cleared.is_none(), "the task no longer names a worktree: {cleared:?}");
}

#[test]
fn unmerged_work_stays_until_it_is_in_the_main_branch() {
    let (_d, app, repo) = project_with_repo();
    let (id, _anna, wt) = closed_task_with_worktree(&app, &repo, "genie/T-2", Some("work"));
    assert!(wt.exists(), "the worktree with unmerged work stays");
    assert!(git_ok(&repo, &["rev-parse", "--verify", "refs/heads/genie/T-2"]), "the branch stays");
    // Once the work lands in the main branch, the next sweep frees it.
    git(&repo, &["merge", "--no-ff", "-m", "merge T-2", "genie/T-2"]);
    let report = genie::runtime::sweep_worktrees(&app, "shop");
    assert!(report.iter().any(|l| l.contains(&id) && l.contains("removed")), "{report:?}");
    assert!(!wt.exists());
}

#[test]
fn uncommitted_work_keeps_the_worktree_even_when_the_branch_is_in_main() {
    let (_d, app, repo) = project_with_repo();
    let (id, _anna, wt) = closed_task_with_worktree(&app, &repo, "genie/T-2", Some("work"));
    git(&repo, &["merge", "--no-ff", "-m", "merge T-2", "genie/T-2"]);
    // The branch is in main now, but the worktree holds a change that is not in it.
    std::fs::write(wt.join("work.txt"), "changed after the task closed").unwrap();
    let report = genie::runtime::sweep_worktrees(&app, "shop");
    assert!(report.iter().any(|l| l.contains(&id) && l.contains("uncommitted")), "{report:?}");
    assert!(wt.exists(), "the worktree with unsaved changes stays");
    assert!(git_ok(&repo, &["rev-parse", "--verify", "refs/heads/genie/T-2"]), "the branch stays too");
}

#[test]
fn untracked_files_keep_the_worktree() {
    let (_d, app, repo) = project_with_repo();
    let wt = repo.parent().unwrap().join("worktrees").join("genie/T-2");
    let (id, anna) = prepared_task_with_worktree_at(&app, &repo, &wt, Some("genie/T-2"), None, None);
    std::fs::write(wt.join("draft.md"), "work in progress").unwrap();
    close_task(&app, &anna, &id);
    let report = genie::runtime::sweep_worktrees(&app, "shop");
    assert!(report.iter().any(|l| l.contains(&id) && l.contains("uncommitted")), "{report:?}");
    assert!(wt.exists(), "the worktree with untracked work stays");
}

#[test]
fn ignored_build_output_does_not_keep_the_worktree() {
    let (_d, app, repo) = project_with_repo();
    std::fs::write(repo.join(".gitignore"), "target/\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "ignore target"]);
    let wt = repo.parent().unwrap().join("worktrees").join("genie/T-2");
    let (id, anna) = prepared_task_with_worktree_at(&app, &repo, &wt, Some("genie/T-2"), None, None);
    std::fs::create_dir_all(wt.join("target")).unwrap();
    std::fs::write(wt.join("target").join("genie"), "17 GB of build output").unwrap();
    close_task(&app, &anna, &id);
    assert!(app.with_tracker("shop", |t| t.get(&id)).unwrap().worktree.is_none());
    assert!(!wt.exists(), "ignored build output goes with the directory");
}

#[test]
fn merged_is_measured_against_the_repositorys_main_branch_not_the_checkout() {
    let (_d, app, repo) = project_with_repo();
    let (id, _anna, wt) = closed_task_with_worktree(&app, &repo, "genie/T-4", Some("work"));
    // The project's checkout sits on a feature branch that has the task's work; the
    // main branch does not. The checkout's HEAD must not vouch for the worktree.
    git(&repo, &["checkout", "-b", "feat"]);
    git(&repo, &["merge", "--no-ff", "-m", "merge T-4 into feat", "genie/T-4"]);
    let report = genie::runtime::sweep_worktrees(&app, "shop");
    assert!(report.iter().any(|l| l.contains(&id) && l.contains("not in main")), "{report:?}");
    assert!(wt.exists(), "work that only a feature branch has stays");
    // And once the main branch takes it, the sweep frees it.
    git(&repo, &["checkout", "main"]);
    git(&repo, &["merge", "--no-ff", "-m", "merge T-4 into main", "genie/T-4"]);
    let report = genie::runtime::sweep_worktrees(&app, "shop");
    assert!(report.iter().any(|l| l.contains(&id) && l.contains("removed")), "{report:?}");
    assert!(!wt.exists());
}

#[test]
fn a_workspace_whose_work_is_on_the_git_host_goes() {
    let (dir, app, repo) = project_with_repo();
    // The team's workspace: a clone of the project's repository, everything pushed.
    let ws = dir.path().join("workspaces").join("shop").join("t1");
    // The "host" is the project's repository: a push into its checked-out branch is allowed.
    git(&repo, &["config", "receive.denyCurrentBranch", "ignore"]);
    git(&repo, &["clone", "--quiet", repo.to_str().unwrap(), ws.to_str().unwrap()]);
    std::fs::write(ws.join("clone.txt"), "committed and pushed").unwrap();
    git(&ws, &["add", "."]);
    git(&ws, &["commit", "-m", "done"]);
    git(&ws, &["push", "--quiet", "origin", "HEAD:main"]);
    let (id, anna) = prepared_task_with_worktree_at(&app, &repo, &ws, None, Some("t1"), None);
    close_task(&app, &anna, &id);
    assert!(!ws.exists(), "a workspace whose work is all on the host goes with the close");
    assert!(app.with_tracker("shop", |t| t.get(&id)).unwrap().worktree.is_none());
}

#[test]
fn a_workspace_with_uncommitted_or_unpushed_work_stays() {
    let (dir, app, repo) = project_with_repo();
    let ws = dir.path().join("workspaces").join("shop").join("t1");
    // The "host" is the project's repository: a push into its checked-out branch is allowed.
    git(&repo, &["config", "receive.denyCurrentBranch", "ignore"]);
    git(&repo, &["clone", "--quiet", repo.to_str().unwrap(), ws.to_str().unwrap()]);
    let (id, anna) = prepared_task_with_worktree_at(&app, &repo, &ws, None, Some("t1"), None);

    std::fs::write(ws.join("notes.md"), "uncommitted").unwrap();
    close_task(&app, &anna, &id);
    assert!(ws.exists(), "uncommitted work keeps the workspace");

    std::fs::remove_file(ws.join("notes.md")).unwrap();
    std::fs::write(ws.join("clone.txt"), "committed but not pushed").unwrap();
    git(&ws, &["add", "."]);
    git(&ws, &["commit", "-m", "work"]);
    let report = genie::runtime::sweep_worktrees(&app, "shop");
    assert!(report.iter().any(|l| l.contains(&id) && l.contains("not on the git host")), "{report:?}");
    assert!(ws.exists(), "work that is on no remote keeps the workspace");

    git(&ws, &["push", "--quiet", "origin", "HEAD:main"]);
    let report = genie::runtime::sweep_worktrees(&app, "shop");
    assert!(report.iter().any(|l| l.contains(&id) && l.contains("removed")), "{report:?}");
    assert!(!ws.exists());
}

#[test]
fn a_team_still_working_keeps_its_worktree() {
    let (_d, app, repo) = project_with_repo();
    let wt = repo.parent().unwrap().join("worktrees").join("genie/T-7");
    let (id, anna) = prepared_task_with_worktree_at(&app, &repo, &wt, Some("genie/T-7"), Some("t7"), Some("work"));
    close_task(&app, &anna, &id);
    // The team is working again after the close (a reopen, a new member): the sweep waits.
    app.with_tracker("shop", |t| t.bus().set_state("t7", TeamState::Active, None, "anna")).unwrap();
    // Even once the work is in the main branch, the active team keeps its worktree.
    git(&repo, &["merge", "--no-ff", "-m", "merge T-7", "genie/T-7"]);
    let report = genie::runtime::sweep_worktrees(&app, "shop");
    assert!(report.iter().any(|l| l.contains(&id) && l.contains("kept for now")), "{report:?}");
    assert!(wt.exists(), "an active team's worktree stays");
    // The team stops: the next sweep frees the worktree.
    app.with_tracker("shop", |t| t.bus().set_state("t7", TeamState::Stopped, Some("done"), "genie")).unwrap();
    let report = genie::runtime::sweep_worktrees(&app, "shop");
    assert!(report.iter().any(|l| l.contains(&id) && l.contains("removed")), "{report:?}");
    assert!(!wt.exists());
}

#[test]
fn closing_a_task_sweeps_only_the_task_it_closes() {
    let (_d, app, repo) = project_with_repo();
    let (t2, _anna2, wt2) = closed_task_with_worktree(&app, &repo, "genie/T-8", Some("work"));
    let wt3 = repo.parent().unwrap().join("worktrees").join("genie/T-9");
    let (t3, anna3) = prepared_task_with_worktree_at(&app, &repo, &wt3, Some("genie/T-9"), Some("t9"), None);
    // Closing the second task stops its team and frees its (merged, clean) worktree,
    // and leaves the first task's unmerged worktree alone.
    close_task(&app, &anna3, &t3);
    let stopped = app.with_tracker("shop", |t| t.bus().get("t9").map(|m| m.state)).unwrap();
    assert_eq!(stopped, TeamState::Stopped, "the closed task's team stops");
    assert!(wt2.exists() && !wt3.exists());
    let report = genie::runtime::sweep_worktrees(&app, "shop");
    assert!(report.iter().any(|l| l.contains(&t2) && l.contains("not in main")), "{report:?}");
    assert!(report.iter().all(|l| !l.contains(&t3)), "{report:?}");
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

#[test]
fn closing_a_task_frees_its_merged_worktree_right_away() {
    let (_d, app, repo) = project_with_repo();
    // Not via sweep_worktrees: the close itself must sweep the task it closes.
    let anna = Caller::person("shop", "anna");
    let task = tasks::create(&app, &anna, CreateBody { title: "now".into(), ..Default::default() }).unwrap();
    let wt = repo.parent().unwrap().join("worktrees").join("genie/T-10");
    git(&repo, &["worktree", "add", "-b", "genie/T-10", wt.to_string_lossy().as_ref()]);
    app.with_tracker("shop", |t| {
        t.assign_team(
            &Actor::new("genie", Role::Orchestrator),
            &task.id,
            Some("t10"),
            Some(&Worktree { path: wt.to_string_lossy().into_owned(), branch: Some("genie/T-10".into()) }),
            None,
        )
    })
    .unwrap();
    tasks::set_status(&app, &anna, &task.id, StatusBody { force: Some(true), ..StatusBody::to(Status::InProgress, None) }).unwrap();
    tasks::set_status(&app, &anna, &task.id, StatusBody { force: Some(true), ..StatusBody::to(Status::Done, Some("done".into())) })
        .unwrap();
    assert!(!wt.exists(), "the worktree goes with the close itself");
    assert!(app.with_tracker("shop", |t| t.get(&task.id)).unwrap().worktree.is_none());
}
