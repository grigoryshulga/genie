//! The git proxy with real `git` clients: what an agent may push is decided by the
//! effective policy, the host's own refusals reach the agent, and nothing of the
//! host's credentials is in the agent's reach.

use crate::common;

use genie_core::DeliveryState;
use std::path::{Path, PathBuf};

use common::githost::{sh, try_sh, upstream, write_git_json};
use common::*;
use genie::runtime::{SpawnRequest, spawn_team};
use genie_core::{Actor, CreateInput, Role, Status, StatusOptions};
use serde_json::json;

struct Rig {
    h: Harness,
    hosts: PathBuf,
    up: PathBuf,
    port: u16,
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// A server on a real port, a `files` host with `acme/api`, and the repository added to project `shop` at the root.
async fn rig(policy: serde_json::Value) -> Rig {
    let port = free_port();
    let h = Harness::with_config(|c| c.port = port);
    let hosts = h.dir.path().join("hosts");
    std::fs::create_dir_all(&hosts).unwrap();
    write_git_json(h.dir.path(), &hosts);
    let up = upstream(&hosts, "acme/api", &[("README.md", "api\n")]);
    h.project("shop");
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    tokio::spawn(genie::serve_on(h.app.clone(), listener, std::future::pending()));
    let (s, b, _) = call(&h.router, "POST", "/api/repos")
        .json(json!({ "name": "api", "host": "files", "remote": "acme/api", "mount": ".", "policy": policy }))
        .header("x-genie-project", "shop")
        .header("host", &format!("127.0.0.1:{port}"))
        .send()
        .await;
    assert_eq!(s, 201, "{b}");
    Rig { h, hosts, up, port }
}

fn ready_task(h: &Harness, title: &str) -> String {
    h.app
        .with_tracker("shop", |t| {
            let orch = Actor::new("orchestrator", Role::Orchestrator);
            let task = t.create(
                &orch,
                CreateInput {
                    title: title.into(),
                    description: Some("do it".into()),
                    acceptance: vec!["it works".into()],
                    ..Default::default()
                },
            )?;
            t.set_status(&orch, &task.id, Status::Ready, StatusOptions::default())?;
            Ok(task.id)
        })
        .unwrap()
}

/// A team for a fresh task; returns (team id, workspace, executor token, reviewer token).
async fn team(r: &Rig) -> (String, PathBuf, String, String) {
    let task = ready_task(&r.h, "Add a feature");
    let app = r.h.app.clone();
    let t = task.clone();
    let team = tokio::task::spawn_blocking(move || {
        spawn_team(
            &app,
            "shop",
            SpawnRequest {
                task: t,
                template: None,
                members: vec![
                    genie::config::MemberSpec { role: "executor".into(), name: Some("bender".into()), ..Default::default() },
                    genie::config::MemberSpec { role: "reviewer".into(), name: Some("yoda".into()), ..Default::default() },
                ],
                models: Default::default(),
                note: None,
                by: Actor::new("orchestrator", Role::Orchestrator),
                initiator: None,
            },
        )
        .unwrap()
    })
    .await
    .unwrap();
    let token = |class: Role, role: &str, name: &str| {
        r.h.app
            .with_server(|db| db.create_role_token("shop", class, Some(role), name, Some(&team.id), None, chrono::Duration::hours(1)))
            .unwrap()
    };
    let (ex, rv) = (token(Role::Executor, "executor", "bender"), token(Role::Reviewer, "reviewer", "yoda"));
    (team.id.clone(), PathBuf::from(&team.cwd), ex, rv)
}

fn git(dir: &Path, token: &str, args: &[&str]) -> (bool, String) {
    try_sh(dir, &[("GENIE_TOKEN", token)], args)
}

fn commit(dir: &Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).unwrap();
    sh(dir, &["add", "."]);
    sh(dir, &["commit", "-q", "-m", &format!("edit {file}")]);
}

fn upstream_has(up: &Path, branch: &str) -> bool {
    try_sh(up, &[], &["rev-parse", "--verify", "-q", &format!("refs/heads/{branch}")]).0
}

fn journal(h: &Harness, kind: &str) -> Vec<genie_core::Event> {
    h.app.with_tracker("shop", |t| genie_core::events::latest_of(t.conn(), kind, 50)).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_pushes_its_task_branch_and_nothing_else() {
    let r = rig(json!({})).await;
    let (_, ws, token, _) = team(&r).await;
    let main_before = sh(&r.up, &["rev-parse", "main"]);
    assert_eq!(sh(&ws, &["branch", "--show-current"]), format!("genie/{}", task_id(&r)));
    let branch = sh(&ws, &["branch", "--show-current"]);

    commit(&ws, "feature.txt", "one\n");
    let (ok, out) = git(&ws, &token, &["push", "origin", "HEAD"]);
    assert!(ok, "the task branch is pushed: {out}");
    assert!(upstream_has(&r.up, &branch), "it reached the host");
    let rows = r.h.app.with_server(|db| db.task_repos("shop", &task_id(&r))).unwrap();
    assert_eq!(rows[0].state, DeliveryState::Published);
    assert!(rows[0].head_sha.is_some());
    assert_eq!(journal(&r.h, "git.pushed").len(), 1);

    // Everything else is refused, with the reason on the agent's screen.
    let (ok, out) = git(&ws, &token, &["push", "origin", "HEAD:main"]);
    assert!(!ok && out.contains("protected"), "{out}");
    let (ok, out) = git(&ws, &token, &["push", "origin", "HEAD:refs/heads/feature/x"]);
    assert!(!ok && out.contains("not allowed"), "{out}");
    let (ok, out) = git(&ws, &token, &["push", "origin", "HEAD:refs/heads/genie/other"]);
    assert!(!ok && out.contains("not allowed"), "{out}");
    let (ok, out) = git(&ws, &token, &["push", "origin", "HEAD:refs/tags/v1"]);
    assert!(!ok && out.contains("only branches"), "{out}");
    let (ok, out) = git(&ws, &token, &["push", "origin", &format!(":{branch}")]);
    assert!(!ok && out.contains("deleting"), "{out}");
    assert!(!upstream_has(&r.up, "feature/x") && !upstream_has(&r.up, "genie/other"));
    assert!(upstream_has(&r.up, &branch), "the branch survived the delete");
    assert_eq!(sh(&r.up, &["rev-parse", "main"]), main_before, "main did not move");
    assert!(journal(&r.h, "git.denied").len() >= 5, "refusals are in the journal");

    // A rewritten history is a force-push: refused.
    sh(&ws, &["commit", "-q", "--amend", "-m", "rewritten"]);
    let (ok, out) = git(&ws, &token, &["push", "--force", "origin", "HEAD"]);
    assert!(!ok, "{out}");
    // A normal follow-up commit is fine.
    sh(&ws, &["reset", "-q", "--hard", &format!("origin/{branch}")]);
    commit(&ws, "feature.txt", "two\n");
    let (ok, out) = git(&ws, &token, &["push", "origin", "HEAD"]);
    assert!(ok, "{out}");
}

fn task_id(r: &Rig) -> String {
    r.h.app.with_tracker("shop", |t| Ok(t.list(&Default::default())?.first().map(|s| s.id.clone()).unwrap_or_default())).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_read_only_role_can_fetch_but_not_push() {
    let r = rig(json!({})).await;
    let (_, ws, _, reviewer) = team(&r).await;
    let (ok, out) = git(&ws, &reviewer, &["fetch", "origin"]);
    assert!(ok, "{out}");
    commit(&ws, "x.txt", "x\n");
    let (ok, out) = git(&ws, &reviewer, &["push", "origin", "HEAD"]);
    assert!(!ok && out.contains("read-only"), "{out}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nobody_gets_in_without_a_token_for_this_project() {
    let r = rig(json!({})).await;
    let (_, _, token, _) = team(&r).await;
    let url = format!("http://127.0.0.1:{}/git/shop/api.git", r.port);
    let scratch = r.h.dir.path().join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let (ok, out) = try_sh(&scratch, &[], &["clone", &url, "none"]);
    assert!(!ok && (out.contains("401") || out.contains("Authentication") || out.contains("Username")), "{out}");
    let (ok, out) = try_sh(&scratch, &[], &["-c", "http.extraHeader=Authorization: Bearer nope", "clone", &url, "bad"]);
    assert!(!ok, "{out}");
    // A token of another project is refused, one of this project works.
    r.h.project("other");
    let foreign =
        r.h.app.with_server(|db| db.create_agent_token("other", Role::Executor, "x", None, None, chrono::Duration::hours(1))).unwrap();
    let (ok, out) = try_sh(&scratch, &[], &["-c", &format!("http.extraHeader=Authorization: Bearer {foreign}"), "clone", &url, "foreign"]);
    assert!(!ok && out.contains("403"), "{out}");
    let (ok, out) = try_sh(&scratch, &[], &["-c", &format!("http.extraHeader=Authorization: Bearer {token}"), "clone", &url, "mine"]);
    assert!(ok, "{out}");
    assert!(scratch.join("mine/README.md").is_file());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_the_host_refuses_reaches_the_agent_and_leaves_no_trace_in_the_mirror() {
    let r = rig(json!({ "push": "branches", "branches": ["genie/{task}", "genie/{task}/*"] })).await;
    let (_, ws, token, _) = team(&r).await;
    let branch = sh(&ws, &["branch", "--show-current"]);
    // The host has a rule of its own (a hook stands for branch protection or a CI gate).
    let hook = r.up.join("hooks/pre-receive");
    std::fs::write(
        &hook,
        "#!/bin/sh\nwhile read old new ref; do case \"$ref\" in *reject*) echo 'branch rules: rejected' >&2; exit 1;; esac; done\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    commit(&ws, "a.txt", "a\n");
    let target = format!("HEAD:refs/heads/{branch}/reject");
    let (ok, out) = git(&ws, &token, &["push", "origin", &target]);
    assert!(!ok && out.contains("the git host refused"), "{out}");
    let mirror = r.h.dir.path().join("repos/files/acme/api.git");
    assert!(!try_sh(&mirror, &[], &["rev-parse", "--verify", "-q", &format!("refs/heads/{branch}/reject")]).0, "the mirror was put back");
    // The same commit goes through under a name the host accepts.
    let (ok, out) = git(&ws, &token, &["push", "origin", &format!("HEAD:refs/heads/{branch}/fine")]);
    assert!(ok, "{out}");
    assert!(upstream_has(&r.up, &format!("{branch}/fine")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn direct_push_is_possible_only_when_the_policy_says_so() {
    let r = rig(json!({ "push": "direct", "protected": [] })).await;
    let (_, ws, token, _) = team(&r).await;
    commit(&ws, "hotfix.txt", "fix\n");
    let (ok, out) = git(&ws, &token, &["push", "origin", "HEAD:main"]);
    assert!(ok, "{out}");
    assert_eq!(sh(&r.up, &["show", "main:hotfix.txt"]), "fix");
    let _ = &r.hosts;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_workspace_without_write_access_to_the_repository_cannot_push() {
    let r = rig(json!({ "push": "none" })).await;
    let (_, ws, token, _) = team(&r).await;
    commit(&ws, "x.txt", "x\n");
    let (ok, out) = git(&ws, &token, &["push", "origin", "HEAD"]);
    assert!(!ok && out.contains("forbids pushing"), "{out}");
}
