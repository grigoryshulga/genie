//! The environment controller over the host runner: `up` through its stages to ready, commands,
//! stop and delete, garbage collection, reconciliation after a restart, the failure of a step, and
//! single-flight preparation. Each scenario runs without bubblewrap and, when it works here, with it.

use std::path::PathBuf;

use genie::config::Config;
use genie::env::controller::{self, CommandRef, UpRequest};
use genie::env::runner::ExecResult;
use genie::state::{App, AppError, AppResult};
use genie_core::env::db::{EnvWorkspace, StageStatus, WsState};
use genie_core::env::spec::Profile;
use serde_json::{Value, json};

use crate::common::githost::{sh, upstream, write_git_json};
use crate::common::{Harness, call};

/// Without bubblewrap, and with it when this machine can run it.
fn modes() -> Vec<bool> {
    if genie::sandbox::works() {
        vec![false, true]
    } else {
        eprintln!("skipped: bubblewrap does not work on this machine");
        vec![false]
    }
}

/// A server with the host runner, a project `shop` whose repository `api` sits at the root, and its
/// upstream commit: a `.gitignore` for the outputs, and `deps.txt`.
struct Project {
    h: Harness,
}

impl Project {
    async fn with(sandbox: bool, tune: impl FnOnce(&mut Config)) -> Project {
        let h = Harness::with_config(|cfg| {
            cfg.environments.runner = "host".into();
            // The host runner enforces no limits: a profile that asks for them needs this switched off.
            cfg.environments.policy.require_limits = false;
            cfg.runtime.sandbox.mode = if sandbox { "auto" } else { "off" }.into();
            tune(cfg);
        });
        let hosts = h.dir.path().join("hosts");
        std::fs::create_dir_all(&hosts).unwrap();
        write_git_json(h.dir.path(), &hosts);
        h.project("shop");
        upstream(&hosts, "acme/api", &[(".gitignore", "out/\nnode_modules/\n"), ("deps.txt", "left-pad 1.0\n")]);
        let (s, b, _) = call(&h.router, "POST", "/api/repos")
            .json(json!({ "name": "api", "host": "files", "remote": "acme/api", "mount": "." }))
            .header("x-genie-project", "shop")
            .send()
            .await;
        assert_eq!(s, 201, "{b}");
        Project { h }
    }

    fn data(&self) -> PathBuf {
        self.h.dir.path().to_path_buf()
    }

    /// Write a profile of the project.
    fn profile(&self, name: &str, value: Value) {
        let profile: Profile = serde_json::from_value(value).expect("a valid profile");
        genie::env::catalog::save_profile(&self.data(), "shop", name, &profile).unwrap();
    }

    /// The workspace directory of the project's source clones.
    fn root(&self, name: &str) -> PathBuf {
        self.data().join("workspaces/shop").join(name)
    }

    /// Run `f` on a blocking thread of the server, as the HTTP layer does.
    async fn on<T: Send + 'static>(&self, f: impl FnOnce(&App) -> AppResult<T> + Send + 'static) -> AppResult<T> {
        self.h.app.blocking(f).await
    }

    async fn up(&self, name: &str, profile: &str) -> AppResult<EnvWorkspace> {
        let req = UpRequest { project: "shop".into(), name: name.into(), task: None, profile: profile.into(), for_agent: false };
        self.on(move |app| controller::up(app, &req)).await
    }

    async fn run(&self, name: &str, command: CommandRef) -> AppResult<ExecResult> {
        let name = name.to_string();
        self.on(move |app| controller::run(app, "shop", &name, command, "")).await
    }

    /// The workspace as the database has it (deleted ones included).
    fn workspace(&self, name: &str) -> Option<EnvWorkspace> {
        let id = format!("shop--{name}");
        self.h.app.with_server(|db| db.env_workspace(&id)).unwrap()
    }

    /// The key of a published snapshot, by its qualified step.
    fn key_of(ws: &EnvWorkspace, step: &str) -> String {
        ws.lock.as_ref().expect("a locked workspace").snapshots[step].clone()
    }

    /// Commit a new content of `deps.txt` in a workspace's clone.
    fn commit_deps(&self, name: &str, content: &str) {
        let clone = self.root(name);
        std::fs::write(clone.join("deps.txt"), content).unwrap();
        sh(&clone, &["add", "deps.txt"]);
        sh(&clone, &["commit", "-q", "-m", "bump deps"]);
    }

    /// How many times the snapshot `key` was prepared: its attempts.
    fn attempts(&self, key: &str) -> i64 {
        self.h.app.with_server(|db| db.env_snapshot(key)).unwrap().expect("the snapshot exists").attempt
    }
}

/// A profile with one component (the repository `api`, recipe `custom`) whose one step writes
/// `out/deps.txt` from `deps.txt`, with a `test` and a failing `fail` command and a `lint` check.
fn standard() -> Value {
    json!({
        "components": [{
            "repo": "api",
            "recipe": "custom",
            "prepare": [{
                "id": "deps",
                "run": "mkdir -p out && cp deps.txt out/deps.txt && echo built",
                "outputs": ["out"],
                "inputs": ["deps.txt"]
            }],
            "commands": {
                "test": "test \"$(cat out/deps.txt)\" = 'left-pad 1.0' && echo tests-pass",
                "fail": "echo failing; exit 3"
            }
        }],
        "checks": { "lint": "true" }
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_environment_comes_up_ready_with_its_output_and_runs_its_commands() {
    for sandbox in modes() {
        let p = Project::with(sandbox, |_| {}).await;
        p.profile("default", standard());
        let ws = p.up("dev", "default").await.unwrap_or_else(|e| panic!("sandbox {sandbox}: {e}"));
        assert_eq!(ws.state, WsState::Ready, "sandbox {sandbox}");
        assert_eq!(
            std::fs::read_to_string(p.root("dev").join("out/deps.txt")).unwrap(),
            "left-pad 1.0\n",
            "the output is in the workspace"
        );
        let lock = ws.lock.clone().expect("locked");
        assert_eq!(lock.snapshots.len(), 1);
        assert_eq!(lock.repos[0].sha.len(), 40, "the commit is locked");

        let check = p.on(|app| controller::check(app, "shop", "default", false)).await.unwrap();
        assert!(check.gaps.is_empty(), "{:?}", check.gaps);
        assert_eq!(check.components, vec!["api (custom r1)".to_string()]);
        assert_eq!(controller::profile_for(&p.h.app, "shop").as_deref(), Some("default"));

        let test = p.run("dev", CommandRef::Named("test".into())).await.unwrap();
        assert_eq!((test.code, test.output.contains("tests-pass")), (0, true), "{}", test.output);
        let fail = p.run("dev", CommandRef::Named("fail".into())).await.unwrap();
        assert_eq!(fail.code, 3, "{}", fail.output);
        let lint = p.run("dev", CommandRef::Named("lint".into())).await.unwrap();
        assert_eq!(lint.code, 0);
        let raw = p.run("dev", CommandRef::Raw("cat out/deps.txt".into())).await.unwrap();
        assert!(raw.output.contains("left-pad"), "{}", raw.output);

        let status = p.on(|app| controller::status(app, "shop", "dev")).await.unwrap();
        let stages: Vec<(String, &str)> = status.stages.iter().map(|s| (s.stage.clone(), s.status.as_str())).collect();
        assert!(stages.contains(&("environment".to_string(), StageStatus::Done.as_str())), "{stages:?}");
        assert!(stages.contains(&("services".to_string(), StageStatus::Done.as_str())), "{stages:?}");
        assert!(status.workspace.lock.is_some());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_workspace_reuses_the_snapshot_instead_of_running_the_step_again() {
    for sandbox in modes() {
        let p = Project::with(sandbox, |_| {}).await;
        p.profile("default", standard());
        let dev = p.up("dev", "default").await.unwrap();
        let key = Project::key_of(&dev, "api/deps");
        let other = p.up("other", "default").await.unwrap();
        assert_eq!(Project::key_of(&other, "api/deps"), key, "the same commit, profile and runner give the same key");
        assert_eq!(p.attempts(&key), 1, "the step ran once, for the first workspace (sandbox {sandbox})");
        assert_eq!(std::fs::read_to_string(p.root("other").join("out/deps.txt")).unwrap(), "left-pad 1.0\n");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_commit_with_a_changed_input_produces_a_new_key_and_its_output() {
    for sandbox in modes() {
        let p = Project::with(sandbox, |_| {}).await;
        p.profile("default", standard());
        let first = p.up("dev", "default").await.unwrap();
        let first_key = Project::key_of(&first, "api/deps");

        p.commit_deps("dev", "left-pad 2.0\n");
        let second = p.up("dev", "default").await.unwrap();
        let second_key = Project::key_of(&second, "api/deps");
        assert_ne!(first_key, second_key, "sandbox {sandbox}");
        assert_eq!(p.attempts(&second_key), 1);
        assert_eq!(second.state, WsState::Ready);
        assert_eq!(std::fs::read_to_string(p.root("dev").join("out/deps.txt")).unwrap(), "left-pad 2.0\n", "the output follows the commit");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failing_step_names_itself_its_exit_code_and_its_log_and_fails_the_workspace() {
    for sandbox in modes() {
        let p = Project::with(sandbox, |_| {}).await;
        p.profile(
            "broken",
            json!({ "components": [{ "repo": "api", "recipe": "custom", "prepare": [{
                "id": "deps",
                "run": "echo fetching; echo 'no such package' >&2; exit 4",
                "outputs": ["out"],
                "inputs": ["deps.txt"]
            }] }] }),
        );
        let err = p.up("bad", "broken").await.unwrap_err();
        let text = err.to_string();
        assert!(text.contains("step api/deps failed with exit code 4"), "sandbox {sandbox}: {text}");
        assert!(text.contains("no such package"), "the log tail is quoted: {text}");
        let ws = p.workspace("bad").unwrap();
        assert_eq!(ws.state, WsState::Failed);
        assert_eq!(ws.failed_stage.as_deref(), Some("environment"));
        assert!(ws.error.unwrap_or_default().contains("exit code 4"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_keeps_the_data_and_up_again_makes_the_workspace_ready_and_runnable() {
    for sandbox in modes() {
        let p = Project::with(sandbox, |_| {}).await;
        p.profile("default", standard());
        p.up("dev", "default").await.unwrap();
        let stopped = p.on(|app| controller::stop(app, "shop", "dev")).await.unwrap();
        assert_eq!(stopped.state, WsState::Stopped);
        assert!(p.root("dev").join("out/deps.txt").exists(), "the data stays");
        let refused = p.run("dev", CommandRef::Named("test".into())).await.unwrap_err();
        assert!(matches!(refused, AppError::Conflict(_)), "{refused}");

        let again = p.up("dev", "default").await.unwrap();
        assert_eq!(again.state, WsState::Ready);
        assert_eq!(p.run("dev", CommandRef::Named("test".into())).await.unwrap().code, 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_refuses_unsaved_work_without_force_and_succeeds_with_force() {
    for sandbox in modes() {
        let p = Project::with(sandbox, |_| {}).await;
        p.profile("default", standard());
        p.up("dev", "default").await.unwrap();
        p.commit_deps("dev", "left-pad 3.0\n");
        let refused = p.on(|app| controller::delete(app, "shop", "dev", false)).await.unwrap_err();
        assert!(matches!(refused, AppError::Conflict(_)) && refused.to_string().contains("not saved"), "{refused}");
        assert!(p.root("dev").exists(), "nothing was removed");

        let gone = p.on(|app| controller::delete(app, "shop", "dev", true)).await.unwrap();
        assert_eq!(gone.state, WsState::Deleted);
        assert!(!p.root("dev").exists(), "the clone is removed");
        assert!(p.on(|app| controller::status(app, "shop", "dev")).await.is_err(), "a deleted workspace is not found");

        // A clean workspace is deleted without force; its ignored outputs are not unsaved work.
        p.up("clean", "default").await.unwrap();
        let clean = p.on(|app| controller::delete(app, "shop", "clean", false)).await.unwrap();
        assert_eq!(clean.state, WsState::Deleted);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn garbage_collection_dry_run_lists_only_unpinned_snapshots_and_gc_removes_them() {
    for sandbox in modes() {
        // A store that holds one byte of snapshots: everything not pinned goes.
        let p = Project::with(sandbox, |cfg| cfg.environments.storage.snapshots_max = "1".into()).await;
        p.profile("default", standard());
        let first = p.up("dev", "default").await.unwrap();
        let old_key = Project::key_of(&first, "api/deps");
        p.commit_deps("dev", "left-pad 4.0\n");
        let second = p.up("dev", "default").await.unwrap();
        let new_key = Project::key_of(&second, "api/deps");
        let snapshots = p.data().join("env-store/snapshots");

        let dry = p.on(|app| controller::gc(app, true)).await.unwrap();
        assert!(dry.dry_run);
        assert_eq!(dry.snapshots, vec![old_key.clone()], "only the snapshot no workspace pins is listed");
        assert!(snapshots.join(&old_key).exists(), "a dry run removes nothing");
        assert!(p.h.app.with_server(|db| db.env_snapshot(&old_key)).unwrap().is_some());

        let real = p.on(|app| controller::gc(app, false)).await.unwrap();
        assert_eq!(real.snapshots, vec![old_key.clone()]);
        assert!(!snapshots.join(&old_key).exists(), "the directory is removed");
        assert!(p.h.app.with_server(|db| db.env_snapshot(&old_key)).unwrap().is_none(), "and its row");
        assert!(snapshots.join(&new_key).exists(), "the pinned snapshot stays (sandbox {sandbox})");
        assert_eq!(std::fs::read_to_string(p.root("dev").join("out/deps.txt")).unwrap(), "left-pad 4.0\n");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reconcile_stops_a_workspace_whose_runtime_is_gone_and_interrupts_a_stage_of_a_dead_process() {
    for sandbox in modes() {
        let p = Project::with(sandbox, |_| {}).await;
        p.profile("default", standard());
        p.up("dev", "default").await.unwrap();
        // The runtime's state of the workspace disappears, as after a machine restart.
        std::fs::remove_dir_all(p.data().join("env-store/workspaces/shop--dev/run")).unwrap();
        // A stage another process was running when it died.
        p.on(|app| app.with_server(|db| db.env_workspace_ensure("shop", "other", None, "default", "host")).map(|_| ())).await.unwrap();
        p.h.app.with_server(|db| db.env_stage_begin("shop--other", "environment", "h", "dead-process-owner")).unwrap();

        let report = p.on(controller::reconcile).await.unwrap();
        assert!(report.workspaces_stopped.contains(&"shop--dev".to_string()), "{report:?}");
        assert!(report.stages_interrupted >= 1, "{report:?}");
        let dev = p.workspace("dev").unwrap();
        assert_eq!(dev.state, WsState::Stopped);
        assert!(dev.error.unwrap_or_default().contains("lost after a restart"));
        let stages = p.h.app.with_server(|db| db.env_stages("shop--other")).unwrap();
        assert_eq!(stages[0].status.as_str(), StageStatus::Failed.as_str());
        assert_eq!(stages[0].detail, "interrupted by a restart");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_workspaces_prepared_at_once_run_a_shared_step_once() {
    for sandbox in modes() {
        let p = Project::with(sandbox, |_| {}).await;
        p.profile("default", standard());
        let (a, b) = tokio::join!(p.up("a", "default"), p.up("b", "default"));
        let (a, b) = (a.unwrap(), b.unwrap());
        let key = Project::key_of(&a, "api/deps");
        assert_eq!(Project::key_of(&b, "api/deps"), key);
        assert_eq!(p.attempts(&key), 1, "one owner prepared it, the other waited (sandbox {sandbox})");
        assert_eq!(a.state, WsState::Ready);
        assert_eq!(b.state, WsState::Ready);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_environment_that_cannot_run_here_fails_before_any_step_with_every_gap() {
    for sandbox in modes() {
        let p = Project::with(sandbox, |_| {}).await;
        let mut value = standard();
        value["requires"] = json!({ "os": "plan9", "capabilities": ["gpu"] });
        p.profile("gapped", value);
        let report = p.on(|app| controller::check(app, "shop", "gapped", false)).await.unwrap();
        let gaps: Vec<String> = report.gaps.iter().map(|g| g.to_string()).collect();
        assert!(gaps.iter().any(|g| g.contains("plan9")) && gaps.iter().any(|g| g.contains("gpu")), "{gaps:?}");

        let err = p.up("gap", "gapped").await.unwrap_err();
        assert!(err.to_string().contains("plan9") && err.to_string().contains("gpu"), "{err}");
        let ws = p.workspace("gap").unwrap();
        assert_eq!((ws.state, ws.failed_stage.as_deref()), (WsState::Failed, Some("environment")));
        assert!(p.h.app.with_server(|db| db.env_snapshots()).unwrap().is_empty(), "no step ran");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tool_is_probed_in_the_base_and_its_version_must_match() {
    for sandbox in modes() {
        let p = Project::with(sandbox, |_| {}).await;
        // A project recipe that needs `sh` at version 9; the probe reports 1.2.3.
        let recipes = p.data().join("environments/shop/recipes");
        std::fs::create_dir_all(&recipes).unwrap();
        let recipe = json!({ "id": "toolbox", "revision": 1, "tools": { "sh": { "version": "9", "probe": "echo sh 1.2.3" } } });
        std::fs::write(recipes.join("toolbox.json"), recipe.to_string()).unwrap();
        p.profile("needs-9", json!({ "components": [{ "repo": "api", "recipe": "toolbox" }] }));
        let err = p.up("tools", "needs-9").await.unwrap_err();
        assert!(err.to_string().contains("tool api/sh is 1.2.3, the profile needs 9"), "sandbox {sandbox}: {err}");
        assert_eq!(p.workspace("tools").unwrap().failed_stage.as_deref(), Some("environment"));

        let mut recipe = recipe;
        recipe["tools"]["sh"]["version"] = json!("1");
        std::fs::write(recipes.join("toolbox.json"), recipe.to_string()).unwrap();
        let ws = p.up("tools", "needs-9").await.unwrap();
        assert_eq!(ws.state, WsState::Ready);
        assert_eq!(ws.lock.unwrap().tools.get("api/sh").map(String::as_str), Some("1.2.3"), "the probed version is locked");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_process_is_started_once_and_an_up_of_a_live_workspace_does_not_restart_it() {
    for sandbox in modes() {
        let p = Project::with(sandbox, |_| {}).await;
        let mut value = standard();
        value["processes"] = json!({ "worker": { "command": "exec sleep 600", "ready": { "command": "test -f out/deps.txt" } } });
        p.profile("served", value);
        let ws = p.up("serve", "served").await.unwrap();
        assert_eq!(ws.state, WsState::Ready);
        let pid_file = p.data().join("env-store/workspaces/shop--serve/run/worker.pid");
        let first = std::fs::read_to_string(&pid_file).expect("the process has a pid file");

        let again = p.up("serve", "served").await.unwrap();
        assert_eq!(again.state, WsState::Ready);
        assert_eq!(std::fs::read_to_string(&pid_file).unwrap(), first, "the running process is kept (sandbox {sandbox})");
        p.on(|app| controller::stop(app, "shop", "serve")).await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_process_that_never_becomes_ready_fails_the_services_stage_within_the_timeout() {
    for sandbox in modes() {
        let p = Project::with(sandbox, |cfg| cfg.environments.ready_timeout_secs = 1).await;
        let mut value = standard();
        value["processes"] = json!({ "worker": { "command": "exec sleep 600", "ready": { "command": "false" } } });
        p.profile("unready", value);
        let err = p.up("slow", "unready").await.unwrap_err();
        assert!(err.to_string().contains("process worker is not ready after 1s"), "sandbox {sandbox}: {err}");
        let ws = p.workspace("slow").unwrap();
        assert_eq!((ws.state, ws.failed_stage.as_deref()), (WsState::Failed, Some("services")));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_ready_snapshot_whose_directory_is_gone_is_prepared_again() {
    for sandbox in modes() {
        let p = Project::with(sandbox, |_| {}).await;
        p.profile("default", standard());
        let dev = p.up("dev", "default").await.unwrap();
        let key = Project::key_of(&dev, "api/deps");
        // Published trees are read-only: removing one takes the store's own removal, as gc does.
        genie::env::store::remove_tree(&p.data().join("env-store/snapshots").join(&key)).unwrap();

        let other = p.up("other", "default").await.unwrap();
        assert_eq!(Project::key_of(&other, "api/deps"), key);
        assert_eq!(p.attempts(&key), 2, "the missing snapshot was prepared again (sandbox {sandbox})");
        assert_eq!(std::fs::read_to_string(p.root("other").join("out/deps.txt")).unwrap(), "left-pad 1.0\n");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_attempt_keeps_its_staging_with_the_log_for_inspection() {
    for sandbox in modes() {
        let p = Project::with(sandbox, |_| {}).await;
        p.profile(
            "broken",
            json!({ "components": [{ "repo": "api", "recipe": "custom", "prepare": [{
                "id": "deps", "run": "echo partial-output; exit 2", "outputs": ["out"], "inputs": ["deps.txt"] }] }] }),
        );
        p.up("bad", "broken").await.unwrap_err();
        let staging = p.data().join("env-store/snapshots/.staging");
        let entries: Vec<PathBuf> = std::fs::read_dir(&staging).unwrap().map(|e| e.unwrap().path()).collect();
        assert_eq!(entries.len(), 1, "one failed attempt is kept (sandbox {sandbox})");
        let log = std::fs::read_to_string(entries[0].join("step.log")).unwrap();
        assert!(log.contains("partial-output"), "{log}");
    }
}
