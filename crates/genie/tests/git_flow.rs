//! The whole thing with processes: an orchestrator and a team of scripted agents that use
//! plain `git` and `genie pr` in their workspaces, a real proxy, a fake host. What
//! they may not do is refused; a person merges on the host; the watcher tells the
//! orchestrator, which closes the task.

mod common;

use genie_core::{DeliveryState, RequestState};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::fakehost;
use common::githost::{sh, try_sh, upstream};
use genie::config::Config;
use genie::state::App;
use genie_core::{Actor, CreateInput, Role, Status};
use serde_json::json;

fn status(app: &App) -> Status {
    app.with_tracker("shop", |t| Ok(t.get("G-1")?.status)).unwrap()
}

async fn wait_for(app: &App, what: &str, timeout: Duration, mut done: impl FnMut(&App) -> bool) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if done(app) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let turns = app.with_server(|db| db.turns("shop", None, 50)).unwrap();
    let log: Vec<String> =
        turns.iter().map(|t| format!("{} {} {:?}\n{}", t.agent, t.status, t.error, t.log.clone().unwrap_or_default())).collect();
    panic!("timed out waiting for {what}\n{}", log.join("\n---\n"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agents_deliver_through_the_proxy_and_a_person_merges() {
    // Secrets the server has and agents must not: the host's token and the tools' usual ones.
    unsafe {
        std::env::set_var("GITHUB_TOKEN", "ghp_should_not_leak");
    }
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut cfg = Config::load(dir.path()).unwrap();
    // In the sandbox when this machine has one: the workspaces and the proxy must work inside it too.
    cfg.runtime.sandbox.mode = if genie::sandbox::works() { "bwrap".into() } else { "off".into() };
    cfg.port = port;
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/git-agent.sh");
    cfg.runtime.command = vec![vec!["bash".into(), script.to_string_lossy().into_owned()], vec!["{message}".into()]];
    cfg.runtime.turn_timeout_secs = 60;
    cfg.runtime.env.insert("GENIE_BIN".into(), env!("CARGO_BIN_EXE_genie").into());

    let hosts = dir.path().join("hosts");
    std::fs::create_dir_all(&hosts).unwrap();
    let up = upstream(&hosts, "acme/api", &[("README.md", "api\n")]);
    let fake = fakehost::spawn("gitlab", "secret").await;
    let git_json = json!({ "hosts": { "h": {
        "kind": "gitlab", "url": fake.url,
        "clone_urls": { "https": format!("file://{}/{{remote}}.git", hosts.display()) }
    } } });
    std::fs::write(dir.path().join("git.json"), git_json.to_string()).unwrap();

    let app = App::open(dir.path(), cfg, PathBuf::from("/nonexistent")).unwrap();
    app.create_project("shop", "Shop", None, None, None).unwrap();
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
    let a2 = app.clone();
    let (repo, warning) = tokio::task::spawn_blocking(move || {
        genie::git::service::add_repo(
            &a2,
            "shop",
            genie_core::repos::NewRepo {
                name: "api".into(),
                host: "h".into(),
                remote: "acme/api".into(),
                mount: Some(".".into()),
                token: Some(genie_core::secrets::Secret("secret".into())),
                ..Default::default()
            },
        )
        .unwrap()
    })
    .await
    .unwrap();
    assert!(warning.is_none(), "{warning:?}");
    assert_eq!(repo.default_branch, "main");

    // A person files a task; the orchestrator refines it and hands it to a team.
    app.with_tracker("shop", |t| {
        t.create(
            &Actor::new("anna", Role::Human),
            CreateInput { title: "CSV export".into(), status: Some(Status::Inbox), ..Default::default() },
        )
    })
    .unwrap();
    app.wake_runtime.notify_one();

    // The team delivers: pushed, refused where it must be, a request opened, reviewed and approved.
    wait_for(&app, "the task approved", Duration::from_secs(90), |app| status(app) == Status::Approved).await;
    let row = app.with_server(|db| db.task_repo("shop", "G-1", "api")).unwrap().unwrap();
    assert_eq!((row.state, row.cr_state, row.cr_number), (DeliveryState::Published, Some(RequestState::Open), Some(1)), "{row:?}");
    assert_eq!(row.branch, "genie/G-1");
    // What the executor reported seeing, as artifacts of the task.
    let show = |name: &str| -> String {
        app.with_tracker("shop", |t| {
            let task = t.get("G-1")?;
            let n = task.artifacts.iter().find(|a| a.name == name).map(|a| a.id);
            Ok(match n {
                Some(n) => String::from_utf8_lossy(&t.read_artifact("G-1", n)?.content).to_string(),
                None => String::new(),
            })
        })
        .unwrap()
    };
    assert_eq!(show("env.txt").trim(), "no host tokens", "the host's token never reached the agent: {}", show("env.txt"));
    assert!(
        show("repos.txt").contains("genie/G-1") && show("repos.txt").contains("person merges"),
        "the agent is told its rules: {}",
        show("repos.txt")
    );
    assert!(show("refused.txt").contains("protected"), "the direct push to main was refused with a reason: {}", show("refused.txt"));
    assert!(show("pr.txt").contains("#1") && show("pr.txt").contains("merge_requests/1"), "{}", show("pr.txt"));
    assert_eq!(sh(&up, &["show", "genie/G-1:feature.txt"]), "csv export");
    assert!(!try_sh(&up, &[], &["show", "main:feature.txt"]).0, "main is untouched");
    {
        let f = fake.lock();
        assert_eq!(f.prs.len(), 1);
        assert!(f.prs[0].title.starts_with("[G-1]") && f.prs[0].body.contains("Adds the export."), "{:?}", f.prs[0]);
    }
    let journal = |kind: &str| app.with_tracker("shop", |t| genie_core::events::latest_of(t.conn(), kind, 20)).unwrap();
    assert!(!journal("git.pushed").is_empty() && !journal("git.denied").is_empty() && !journal("cr.opened").is_empty());

    // The orchestrator tried to close it and was told the request is open: the task waits.
    wait_for(&app, "the orchestrator's verdict turn", Duration::from_secs(30), |app| {
        app.with_server(|db| db.turns("shop", None, 100)).unwrap().iter().all(|t| t.status != "running")
    })
    .await;
    assert_eq!(status(&app), Status::Approved);

    // A person merges on the host; the watcher records it; the orchestrator is told and closes the task.
    fake.lock().prs[0].state = "merged".into();
    let row = app.with_server(|db| db.task_repo("shop", "G-1", "api")).unwrap().unwrap();
    genie::git::delivery::watch_one(&app, &row).await.unwrap();
    wait_for(&app, "the task done", Duration::from_secs(60), |app| status(app) == Status::Done).await;
    assert_eq!(app.with_server(|db| db.task_repo("shop", "G-1", "api")).unwrap().unwrap().state, DeliveryState::Merged);
}
