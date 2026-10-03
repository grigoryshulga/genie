//! The agent sandbox over real processes: a one-shot job runs in bubblewrap with
//! a probing script in place of pi. From inside, it tries what an agent led
//! astray would — read the server's configuration and databases, another
//! project's repository, the home's keys and the server's own process, write
//! outside its worktree — and does its real work: writes and commits in its
//! worktree, uses its `/tmp` and a tool cache, reports through `genie agent`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use genie::config::{Config, RuntimeConfig};
use genie::state::App;
use genie_core::work::NewJob;
use serde_json::json;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn repo(path: &Path, file: &str) {
    write(&path.join(file), "x\n");
    git(path, &["init", "-q", "-b", "main"]);
    git(path, &["config", "user.email", "genie@example.com"]);
    git(path, &["config", "user.name", "genie"]);
    git(path, &["add", "."]);
    git(path, &["commit", "-q", "-m", "init"]);
}

const PROBE: &str = r#"
{
  test -e "$DATA/config.json" && echo config:visible || echo config:hidden
  test -e "$DATA/server.db" && echo db:visible || echo db:hidden
  test -e "$ROOT/other/secret.txt" && echo other:visible || echo other:hidden
  test -e "$ROOT/other.worktrees/x/file" && echo other-worktrees:visible || echo other-worktrees:hidden
  test -e "$HOME/.ssh/id_ed25519" && echo ssh:visible || echo ssh:hidden
  test -e "/proc/$SERVER_PID/environ" && echo server:visible || echo server:hidden
  (echo x > "$ROOT/outside.txt") 2>/dev/null && echo outside:writable || echo outside:read-only
  (echo x > "$HOME/profile-x") 2>/dev/null && echo home:writable || echo home:read-only
  (echo x > "$HOME/.cache/x") 2>/dev/null && echo cache:writable || echo cache:read-only
  (echo x > /tmp/x && test -s /tmp/x) 2>/dev/null && echo tmp:writable || echo tmp:read-only
  (echo change > change.txt && git add change.txt && git commit -qm probe) >/dev/null 2>&1 && echo commit:ok || echo commit:failed
  # Ways round the mounts: a link inside the worktree, `..`, the server's key, the container and desktop sockets.
  ln -s "$DATA/server.db" link-db; test -s link-db && echo link-to-db:visible || echo link-to-db:hidden
  ln -s "$ROOT/outside.txt" link-out; (echo x > link-out) 2>/dev/null && echo link-write:writable || echo link-write:read-only
  test -e ../../data/config.json && echo dotdot:visible || echo dotdot:hidden
  test -e "$DATA/secrets.key" && echo key:visible || echo key:hidden
  test -S /var/run/docker.sock && echo docker-sock:visible || echo docker-sock:hidden
  test -S /run/docker.sock && echo run-docker-sock:visible || echo run-docker-sock:hidden
  # The repository's own git directory is writable (commits go there), but what the server runs outside the sandbox is not:
  # a hook or a config key written here would run with the server's rights the next time it uses git there.
  GITDIR=$(git rev-parse --path-format=absolute --git-common-dir)
  (echo '#!/bin/sh' > "$GITDIR/hooks/post-checkout") 2>/dev/null && echo git-hook:writable || echo git-hook:read-only
  (git config core.fsmonitor 'touch pwned') 2>/dev/null && echo git-config:writable || echo git-config:read-only
  (git config --worktree core.hooksPath /tmp) 2>/dev/null && echo git-hookspath:writable || echo git-hookspath:read-only
  # Without a token the API does not take the agent for the local owner once the server has people.
  echo "api-anonymous:$(curl -s -o /dev/null -w '%{http_code}' -m 5 "$GENIE_URL/api/users" || echo none)"
} > probe.txt 2>&1
"$GENIE_BIN" agent output '{"summary":"probed"}'
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sandboxed_agent_sees_only_its_own_work() {
    if !genie::sandbox::works() {
        assert!(std::env::var("GENIE_REQUIRE_SANDBOX").is_err(), "bubblewrap does not work here, and GENIE_REQUIRE_SANDBOX is set");
        eprintln!("skipped: bubblewrap does not work on this machine");
        return;
    }
    // Outside /tmp: the sandbox gives the agent its own /tmp, which would hide these by accident.
    let t = tempfile::Builder::new().tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let root = t.path().canonicalize().unwrap();
    let home = root.join("home");
    write(&home.join(".ssh/id_ed25519"), "PRIVATE KEY\n");
    std::fs::create_dir_all(home.join(".cache")).unwrap();
    // SAFETY: the only test of this binary; the server reads HOME for the sandbox's defaults.
    unsafe { std::env::set_var("HOME", &home) };

    let data = root.join("data");
    write(&data.join("config.json"), &json!({ "telegram": { "token": "tg-s3cret" } }).to_string());
    write(&data.join("secrets.key"), "KEY\n");
    let shop = root.join("shop");
    repo(&shop, "README.md");
    let other = root.join("other");
    repo(&other, "secret.txt");
    write(&root.join("other.worktrees/x/file"), "theirs\n");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = Config::load(&data).unwrap();
    cfg.port = listener.local_addr().unwrap().port();
    cfg.runtime.sandbox.mode = "bwrap".into();
    let mut command = RuntimeConfig::default().command;
    command[0] = vec!["bash".into(), "-c".into(), PROBE.into(), "harness".into()];
    cfg.runtime.command = command;
    cfg.runtime.mode = "turns".into();
    for (k, v) in [
        ("GENIE_BIN", env!("CARGO_BIN_EXE_genie").to_string()),
        ("DATA", data.to_string_lossy().into_owned()),
        ("ROOT", root.to_string_lossy().into_owned()),
        ("SERVER_PID", std::process::id().to_string()),
    ] {
        cfg.runtime.env.insert(k.into(), v);
    }
    let app = App::open(&data, cfg, PathBuf::from("/nonexistent")).unwrap();
    for (slug, repo, prefix) in [("shop", &shop, "SHOP"), ("other", &other, "OTH")] {
        app.create_project(slug, slug, Some(&repo.to_string_lossy()), None, Some(prefix)).unwrap();
        app.with_server(|db| db.set_autonomy(slug, "manual")).unwrap();
    }
    // The server has people: an address on this machine is no longer taken for its owner.
    app.with_server(|db| db.create_user("anna", "Anna", None, Some("password-1"), true)).unwrap();
    let (_stop, rx) = tokio::sync::oneshot::channel::<()>();
    let a = app.clone();
    tokio::spawn(async move {
        genie::serve_on(a, listener, async {
            let _ = rx.await;
        })
        .await
        .unwrap();
    });
    genie::runtime::start(&app);

    let job = NewJob {
        project: "shop".into(),
        role: "executor".into(),
        goal: "Probe".into(),
        inputs: json!({}),
        workspace: "worktree".into(),
        ..Default::default()
    };
    let id = app.with_server(|db| db.create_job(job)).unwrap().id;
    app.wake_runtime.notify_one();
    let start = Instant::now();
    let j = loop {
        let j = app.with_server(|db| db.job(id)).unwrap();
        if j.status == "succeeded" || j.status == "failed" {
            break j;
        }
        if start.elapsed() > Duration::from_secs(60) {
            let turns = app.with_server(|db| db.turns("shop", None, 5)).unwrap();
            panic!("the job did not finish: {:?}", turns.iter().map(|t| (&t.status, &t.error, &t.log)).collect::<Vec<_>>());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(j.status, "succeeded", "{:?}", j.error);

    let worktree = root.join(format!("shop.worktrees/job-{id}"));
    let probe = std::fs::read_to_string(worktree.join("probe.txt")).unwrap();
    let seen: Vec<&str> = probe.lines().collect();
    for expected in [
        "config:hidden",
        "db:hidden",
        "other:hidden",
        "other-worktrees:hidden",
        "ssh:hidden",
        "server:hidden",
        "outside:read-only",
        "home:read-only",
        "cache:writable",
        "tmp:writable",
        "commit:ok",
        "link-to-db:hidden",
        "link-write:read-only",
        "dotdot:hidden",
        "key:hidden",
        "docker-sock:hidden",
        "run-docker-sock:hidden",
        "git-hook:read-only",
        "git-config:read-only",
        "git-hookspath:read-only",
        "api-anonymous:401",
    ] {
        assert!(seen.contains(&expected), "expected {expected}, the agent saw:\n{probe}");
    }
    assert!(!root.join("outside.txt").exists());
    assert!(git(&shop, &["log", "--all", "--format=%s"]).lines().any(|s| s == "probe"), "the commit is in the repository");
}
