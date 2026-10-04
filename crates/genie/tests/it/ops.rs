//! Operations: a backup of a live server holds everything needed to restore it
//! (databases, the vault, the configuration) and old backups are pruned; the
//! preflight is there for the server's admins only.

use crate::common;

use axum::http::StatusCode;
use common::{Harness, call};
use genie_core::server_db::ServerDb;
use serde_json::json;

#[tokio::test]
async fn a_backup_holds_the_databases_the_vault_and_the_configuration() {
    let h = Harness::new();
    h.project("shop");
    let data = h.app.data.clone();
    std::fs::write(data.join("config.json"), json!({ "telegram": { "token": "tg-s3cret" } }).to_string()).unwrap();
    std::fs::create_dir_all(data.join("agents")).unwrap();
    std::fs::write(data.join("agents/security-reviewer.md"), "---\nextends: reviewer\n---\n").unwrap();
    let (s, _, _) = call(&h.router, "POST", "/api/tasks").json(json!({ "title": "Экспорт" })).send().await;
    assert_eq!(s, StatusCode::CREATED);

    let backups = tempfile::tempdir().unwrap();
    let report = genie::cli::backup(&data, &h.app.cfg, backups.path()).unwrap();
    let out = std::fs::read_dir(backups.path()).unwrap().flatten().map(|e| e.path()).next().unwrap();
    assert!(report.contains("config/: config.json, agents"), "{report}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&out).unwrap().permissions().mode() & 0o777, 0o700, "the backup holds secrets");
    }
    let server = ServerDb::open(&out.join("server.db")).unwrap();
    assert_eq!(server.projects().unwrap()[0].slug, "shop");
    let tasks: i64 = rusqlite::Connection::open(out.join("projects/shop.db"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tasks, 1);
    assert!(std::fs::read_to_string(out.join("config/config.json")).unwrap().contains("tg-s3cret"));
    assert!(out.join("config/agents/security-reviewer.md").exists());
    if data.join("vault/.git").exists() {
        let restored = tempfile::tempdir().unwrap();
        let st = std::process::Command::new("git")
            .args(["clone", "-q"])
            .arg(out.join("vault.bundle"))
            .arg(restored.path().join("vault"))
            .status()
            .unwrap();
        assert!(st.success(), "the vault restores from its bundle");
    }

    // Retention: the most recent ones stay, anything else in the directory is left alone.
    for stamp in ["20260101-000000", "20260102-000000", "20260103-000000"] {
        std::fs::create_dir_all(backups.path().join(format!("genie-{stamp}"))).unwrap();
    }
    std::fs::create_dir_all(backups.path().join("genie-notes")).unwrap();
    let removed = genie::cli::prune_backups(backups.path(), 2).unwrap();
    let names =
        |paths: Vec<std::path::PathBuf>| paths.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
    assert_eq!(names(removed), ["genie-20260101-000000", "genie-20260102-000000"]);
    let mut left = names(std::fs::read_dir(backups.path()).unwrap().flatten().map(|e| e.path()).collect());
    left.sort();
    assert_eq!(left, ["genie-20260103-000000".to_string(), out.file_name().unwrap().to_string_lossy().into_owned(), "genie-notes".into()]);
}

#[tokio::test]
async fn the_preflight_is_for_server_admins() {
    let h = Harness::new();
    h.project("shop");
    let (s, d, _) = call(&h.router, "GET", "/api/doctor").send().await;
    assert_eq!(s, StatusCode::OK, "the local owner is an admin: {d}");
    let checks = d["checks"].as_array().unwrap();
    assert!(checks.iter().any(|c| c["area"] == "projects" && c["level"] == "ok"), "{d}");
    // `warn` where the web UI is built into the binary (it is served instead), `fail` where it is not.
    assert!(checks.iter().any(|c| c["area"] == "web" && c["level"] != "ok"), "the harness's --web directory is missing: {d}");

    let vic = h.app.with_server(|db| db.create_user("vic", "", None, Some("password-2"), false)).unwrap();
    h.app.with_server(|db| db.set_membership("shop", vic.id, genie_core::server_db::ProjectRole::Admin)).unwrap();
    let (_, _, cookies) = call(&h.remote, "POST", "/api/auth/login").json(json!({ "login": "vic", "password": "password-2" })).send().await;
    let (s, _, _) = call(&h.remote, "GET", "/api/doctor").cookie(&cookies[0]).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN, "a project admin is not a server admin");
}

#[tokio::test]
async fn stats_count_tasks_decisions_and_answers() {
    let h = Harness::new();
    h.project("shop");
    let r = &h.router;
    for title in ["first", "second", "third"] {
        call(r, "POST", "/api/tasks").json(json!({ "title": title })).send().await;
    }
    let orch = h
        .app
        .with_server(|db| {
            db.create_agent_token("shop", genie_core::Role::Orchestrator, "orchestrator", None, None, chrono::Duration::hours(1))
        })
        .unwrap();
    let (s, e, _) = call(&h.remote, "POST", "/api/tasks/G-1/status")
        .bearer(&orch)
        .no_csrf()
        .json(json!({ "status": "needs_owner", "note": "CSV или XLSX?" }))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{e}");
    call(r, "POST", "/api/tasks/G-1/comments").json(json!({ "text": "CSV" })).send().await;
    let (s, e, _) = call(r, "POST", "/api/tasks/G-2/status").json(json!({ "status": "done", "force": true })).send().await;
    assert_eq!(s, StatusCode::OK, "{e}");
    call(r, "POST", "/api/tasks/G-3/status").json(json!({ "status": "cancelled" })).send().await;

    let stats = genie::stats::collect(&h.app.data, 7, None, &h.app.cfg.model_prices).unwrap();
    let p = &stats.projects[0];
    assert_eq!((p.created, p.created_by_people, p.done, p.cancelled, p.open), (3, 3, 1, 1, 1), "{p:?}");
    assert_eq!(p.decisions, 1);
    assert!(p.answer_hours_median.is_some(), "the answer to the agent's question is timed: {p:?}");
    assert_eq!((p.comments_by_people, p.comments_by_agents), (1, 0));
    assert!(p.cycle_hours_median.is_some());
    assert_eq!(p.daily.len(), 8, "a week and today: {:?}", p.daily);
    let today = p.daily.last().unwrap();
    assert_eq!((today.created, today.done), (3, 1), "{today:?}");
    assert!(genie::stats::render(&stats).contains("agents asked people 1 time(s)"));
    let (s, j, _) = call(r, "GET", "/api/stats?days=30").send().await;
    assert_eq!(s, StatusCode::OK, "{j}");
    assert_eq!(j["projects"][0]["decisions"], 1);
    assert!(genie::stats::collect(&h.app.data, 7, Some("nope"), &Default::default()).is_err());
}

/// G-85: the drill. A backup of a working server is restored into an empty directory and the
/// restored server holds what the first one did: projects, tasks, people, the stored tokens
/// (with `--with-secrets`), the vault and the configuration. A damaged backup is refused before
/// anything is written, and a server is not overwritten without `--force`.
#[tokio::test]
async fn a_backup_restores_into_an_empty_directory_with_everything_in_it() {
    let h = Harness::new();
    h.project("shop");
    let data = h.app.data.clone();
    std::fs::write(data.join("config.json"), json!({ "telegram": { "token": "tg-s3cret" } }).to_string()).unwrap();
    std::fs::write(data.join("git.json"), r#"{"hosts": {}}"#).unwrap();
    std::fs::create_dir_all(data.join("agents")).unwrap();
    std::fs::write(data.join("agents/security-reviewer.md"), "---\nextends: reviewer\n---\n").unwrap();
    for title in ["Экспорт", "Импорт"] {
        let (s, _, _) = call(&h.router, "POST", "/api/tasks").json(json!({ "title": title })).send().await;
        assert_eq!(s, StatusCode::CREATED);
    }
    let anna = h.app.with_server(|db| db.create_user("anna", "Anna", None, Some("password-1"), true)).unwrap();
    h.app.with_server(|db| db.set_user_secret(anna.id, "litellm", "sk-secret")).unwrap();
    assert!(data.join("secrets.key").exists());

    let backups = tempfile::tempdir().unwrap();
    let report = genie::cli::backup_with(&data, &h.app.cfg, backups.path(), true).unwrap();
    assert!(report.contains("secrets.key (the key"), "{report}");
    let out = std::fs::read_dir(backups.path()).unwrap().flatten().map(|e| e.path()).next().unwrap();
    assert!(out.join("MANIFEST.json").exists() && out.join("config/git.json").exists());

    // An empty directory on another "machine".
    let fresh = tempfile::tempdir().unwrap();
    let new_data = fresh.path().join("genie-data");
    let report = genie::cli::restore(&new_data, &out, false).unwrap();
    assert!(report.contains("server.db →") && report.contains("secrets.key → the stored tokens are readable"), "{report}");

    let app = genie::state::App::open(&new_data, genie::config::Config::load(&new_data).unwrap(), std::path::PathBuf::from("/nonexistent"))
        .unwrap();
    let project = app.with_server(|db| db.project("shop")).unwrap();
    assert!(
        project.tracker_dir.starts_with(new_data.to_string_lossy().as_ref()),
        "the tracker moved with the data: {}",
        project.tracker_dir
    );
    let titles: Vec<String> =
        app.with_tracker("shop", |t| Ok(t.list(&Default::default())?.into_iter().map(|x| x.title).collect())).unwrap();
    assert!(titles.contains(&"Экспорт".to_string()) && titles.contains(&"Импорт".to_string()), "{titles:?}");
    let user = app.with_server(|db| db.user_by_login("anna")).unwrap().expect("the person is back");
    assert_eq!(
        app.with_server(|db| db.user_secret(user.id, "litellm")).unwrap().as_deref(),
        Some("sk-secret"),
        "the stored token reads again"
    );
    assert!(std::fs::read_to_string(new_data.join("config.json")).unwrap().contains("tg-s3cret"));
    assert!(new_data.join("git.json").exists() && new_data.join("agents/security-reviewer.md").exists());
    drop(app);

    // A server is not overwritten by accident; a damaged backup is refused before anything is written.
    let err = genie::cli::restore(&new_data, &out, false).unwrap_err();
    assert!(err.contains("--force"), "{err}");
    let broken = tempfile::tempdir().unwrap().keep();
    std::fs::create_dir_all(broken.join("projects")).unwrap();
    std::fs::copy(out.join("server.db"), broken.join("server.db")).unwrap();
    std::fs::write(broken.join("projects/shop.db"), vec![7u8; 8192]).unwrap();
    let empty = fresh.path().join("untouched");
    let err = genie::cli::restore(&empty, &broken, false).unwrap_err();
    assert!(err.contains("shop.db"), "{err}");
    assert!(!empty.exists(), "nothing was written");
    assert!(genie::cli::restore(&empty, backups.path(), false).unwrap_err().contains("not a genie backup"));

    // Without the key in the backup the restored server says the tokens must be entered again.
    let plain = tempfile::tempdir().unwrap();
    let report = genie::cli::backup(&data, &h.app.cfg, plain.path()).unwrap();
    assert!(report.contains("secrets.key is NOT included"), "{report}");
    let out2 = std::fs::read_dir(plain.path()).unwrap().flatten().map(|e| e.path()).next().unwrap();
    let without = fresh.path().join("without-key");
    let report = genie::cli::restore(&without, &out2, false).unwrap();
    assert!(report.contains("must be entered again"), "{report}");
}
