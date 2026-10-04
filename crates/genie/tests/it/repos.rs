//! Repositories of a project: hosts from `git.json`, the project's list, mirrors and
//! workspaces with several repositories at their mounts.

use crate::common;

use common::githost::{sh, upstream, write_git_json};
use common::*;
use genie::git::service;
use serde_json::json;

async fn add(h: &Harness, name: &str, remote: &str, mount: &str) -> serde_json::Value {
    let (s, b, _) = call(&h.router, "POST", "/api/repos")
        .json(json!({ "name": name, "host": "files", "remote": remote, "mount": mount }))
        .header("x-genie-project", "shop")
        .send()
        .await;
    assert_eq!(s, 201, "{b}");
    b
}

fn setup() -> (Harness, std::path::PathBuf) {
    let h = Harness::new();
    let hosts = h.dir.path().join("hosts");
    std::fs::create_dir_all(&hosts).unwrap();
    write_git_json(h.dir.path(), &hosts);
    h.project("shop");
    (h, hosts)
}

#[tokio::test]
async fn an_administrator_adds_repositories_and_the_server_mirrors_them() {
    let (h, hosts) = setup();
    upstream(&hosts, "acme/api", &[("README.md", "api\n")]);

    let (s, b, _) = call(&h.router, "GET", "/api/git/hosts").send().await;
    assert_eq!(s, 200);
    assert_eq!(b["hosts"][0]["id"], "files");

    let repo = add(&h, "api", "acme/api", "services/api").await;
    assert_eq!(repo["defaultBranch"], "main", "found out from the host: {repo}");
    assert_eq!(repo["mount"], "services/api");
    assert!(repo["warning"].is_null(), "{repo}");
    assert!(h.dir.path().join("repos/files/acme/api.git/HEAD").exists(), "the mirror is made");

    // Wrong host, wrong path, colliding mount, broken policy: refused with a reason.
    let bad = |body: serde_json::Value| async {
        let (s, b, _) = call(&h.router, "POST", "/api/repos").json(body).header("x-genie-project", "shop").send().await;
        (s, b)
    };
    let (s, b) = bad(json!({ "name": "x", "host": "nope", "remote": "a/b" })).await;
    assert_eq!(s, 422);
    assert!(b["error"].as_str().unwrap().contains("not configured"), "{b}");
    let (s, _) = bad(json!({ "name": "x", "host": "files", "remote": "../etc", "mount": "x" })).await;
    assert_eq!(s, 422);
    let (s, _) = bad(json!({ "name": "y", "host": "files", "remote": "acme/api", "mount": "services/api/inner" })).await;
    assert_eq!(s, 422);
    let (s, b) = bad(json!({ "name": "z", "host": "files", "remote": "acme/z", "mount": "z", "policy": { "pushh": "direct" } })).await;
    assert_eq!(s, 400, "{b}");

    let (_, list, _) = call(&h.router, "GET", "/api/repos").header("x-genie-project", "shop").send().await;
    assert_eq!(list.as_array().unwrap().len(), 1, "{list}");

    let (s, b, _) = call(&h.router, "PATCH", "/api/repos/api")
        .json(json!({ "policy": { "push": "branches", "branches": ["genie/{task}"] } }))
        .header("x-genie-project", "shop")
        .send()
        .await;
    assert_eq!(s, 200, "{b}");
    assert_eq!(b["policy"]["push"], "branches");
}

#[tokio::test]
async fn a_task_gets_a_clone_of_each_repository_at_its_mount_on_its_own_branch() {
    let (h, hosts) = setup();
    upstream(&hosts, "acme/api", &[("main.rs", "fn main() {}\n")]);
    upstream(&hosts, "acme/web", &[("index.html", "<html/>\n")]);
    upstream(&hosts, "acme/docs", &[("guide.md", "# guide\n")]);
    add(&h, "api", "acme/api", "services/api").await;
    add(&h, "web", "acme/web", "services/web").await;
    // A project may allow only reading a repository.
    let (s, _, _) = call(&h.router, "POST", "/api/repos")
        .json(json!({ "name": "docs", "host": "files", "remote": "acme/docs", "mount": "docs", "access": "read" }))
        .header("x-genie-project", "shop")
        .send()
        .await;
    assert_eq!(s, 201);

    // With several writable repositories the task must say which it works in.
    let app = h.app.clone();
    let err = tokio::task::spawn_blocking(move || service::task_workspace(&app, "shop", "team-1", Some("S-1")).map(|_| ())).await.unwrap();
    assert!(err.unwrap_err().to_string().contains("must name the repositories"));

    h.app.with_server(|db| db.set_task_repos("shop", "S-1", &[("api".into(), "write".into()), ("docs".into(), "read".into())])).unwrap();
    let app = h.app.clone();
    let (root, placed) =
        tokio::task::spawn_blocking(move || service::task_workspace(&app, "shop", "team-1", Some("S-1")).unwrap()).await.unwrap();

    assert!(root.join("services/api/main.rs").is_file());
    assert!(root.join("docs/guide.md").is_file());
    assert!(root.join("services/web/index.html").is_file(), "read access is implied for repositories the task does not name");
    let by = |n: &str| placed.iter().find(|p| p.repo.name == n).unwrap();
    assert_eq!(by("api").branch.as_deref(), Some("genie/S-1"));
    assert_eq!(by("docs").branch, None, "read-only repositories stay on the default branch");
    assert_eq!(by("web").branch, None, "a repository the task does not name is not writable");
    assert_eq!(sh(&root.join("services/api"), &["branch", "--show-current"]), "genie/S-1");
    assert_eq!(sh(&root.join("docs"), &["branch", "--show-current"]), "main");

    // `origin` is the server, and the clone holds no host address or token.
    let origin = sh(&root.join("services/api"), &["remote", "get-url", "origin"]);
    assert!(origin.contains("/git/shop/api.git") && origin.starts_with("http://127.0.0.1:"), "{origin}");
    let config = std::fs::read_to_string(root.join("services/api/.git/config")).unwrap();
    assert!(!config.contains("acme/api") && !config.contains(&hosts.to_string_lossy().to_string()), "{config}");

    // Rebuilding keeps the agent's commits and branch.
    std::fs::write(root.join("services/api/new.txt"), "x").unwrap();
    sh(&root.join("services/api"), &["add", "."]);
    sh(&root.join("services/api"), &["commit", "-q", "-m", "work"]);
    let app = h.app.clone();
    tokio::task::spawn_blocking(move || service::task_workspace(&app, "shop", "team-1", Some("S-1")).unwrap()).await.unwrap();
    assert!(root.join("services/api/new.txt").is_file());
    assert_eq!(sh(&root.join("services/api"), &["log", "--format=%s", "-1"]), "work");

    let rows = h.app.with_server(|db| db.task_repos("shop", "S-1")).unwrap();
    assert_eq!(rows.iter().find(|r| r.repo == "api").unwrap().branch, "genie/S-1");
}

#[tokio::test]
async fn a_repository_at_the_root_holds_the_others_in_subdirectories() {
    let (h, hosts) = setup();
    upstream(&hosts, "acme/app", &[("app.txt", "app\n")]);
    upstream(&hosts, "acme/lib", &[("lib.txt", "lib\n")]);
    add(&h, "app", "acme/app", ".").await;
    add(&h, "lib", "acme/lib", "vendor/lib").await;
    h.app.with_server(|db| db.set_task_repos("shop", "S-2", &[("app".into(), "write".into()), ("lib".into(), "read".into())])).unwrap();
    let app = h.app.clone();
    let (root, _) =
        tokio::task::spawn_blocking(move || service::task_workspace(&app, "shop", "team-2", Some("S-2")).unwrap()).await.unwrap();
    assert!(root.join("app.txt").is_file() && root.join("vendor/lib/lib.txt").is_file());
    assert_eq!(sh(&root, &["status", "--porcelain"]), "", "the nested repository is not the root's business");
}

#[tokio::test]
async fn the_view_follows_the_default_branch_and_a_project_without_repositories_has_none() {
    let (h, hosts) = setup();
    let app = h.app.clone();
    assert!(tokio::task::spawn_blocking(move || service::view_workspace(&app, "shop")).await.unwrap().is_none());
    let up = upstream(&hosts, "acme/api", &[("a.txt", "1\n")]);
    add(&h, "api", "acme/api", "api").await;
    let app = h.app.clone();
    let view = tokio::task::spawn_blocking(move || service::view_workspace(&app, "shop").unwrap()).await.unwrap();
    assert_eq!(std::fs::read_to_string(view.join("api/a.txt")).unwrap(), "1\n");
    // The host moves on; a sync brings it into the mirror and the next view has it.
    let work = hosts.join("more");
    sh(&hosts, &["clone", "-q", &up.to_string_lossy(), &work.to_string_lossy()]);
    std::fs::write(work.join("a.txt"), "2\n").unwrap();
    sh(&work, &["commit", "-qam", "second"]);
    sh(&work, &["push", "-q", "origin", "HEAD:refs/heads/main"]);
    let (s, _, _) = call(&h.router, "POST", "/api/repos/api/sync").header("x-genie-project", "shop").send().await;
    assert_eq!(s, 200);
    let app = h.app.clone();
    let view = tokio::task::spawn_blocking(move || service::view_workspace(&app, "shop").unwrap()).await.unwrap();
    assert_eq!(std::fs::read_to_string(view.join("api/a.txt")).unwrap(), "2\n");
}
