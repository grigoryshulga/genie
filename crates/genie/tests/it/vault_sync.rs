//! The vault in sync with a git remote: people edit it in Obsidian (a clone of
//! the remote), the server and its agents write it too. Changes go both ways;
//! pages edited in both places are merged and the admins are told; a conflict
//! git cannot settle leaves the vault as it was.

use crate::common;

use std::path::Path;

use axum::http::StatusCode;
use common::{Harness, call};
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

/// An Obsidian user: commit everything and push.
fn push(clone: &Path, message: &str) {
    git(clone, &["add", "-A"]);
    git(clone, &["-c", "user.name=Анна", "-c", "user.email=anna@example.com", "commit", "-q", "-m", message]);
    git(clone, &["push", "-q", "origin", "main"]);
}

#[tokio::test]
async fn the_vault_syncs_with_a_remote_both_ways() {
    let t = tempfile::tempdir().unwrap();
    let remote = t.path().join("vault.git");
    std::process::Command::new("git").args(["init", "-q", "--bare", "-b", "main"]).arg(&remote).status().unwrap();
    let url = remote.to_string_lossy().into_owned();
    let h = Harness::with_config(|c| {
        c.vault.remote = Some(url.clone());
        c.vault.branch = Some("main".into());
    });
    h.project("shop");
    let admin = h.app.with_server(|db| db.create_user("anna", "Анна", None, Some("password-1"), true)).unwrap();
    let notes = || h.app.with_server(|db| db.notifications(admin.id, false, 20)).unwrap();
    let vault = h.app.with_vault(|v| Ok(v.root().to_path_buf())).unwrap();

    let st = genie::vault_sync::sync(&h.app).unwrap().unwrap();
    assert!(st.ok && st.pushed > 0, "the server's vault goes to the empty remote: {st:?}");
    let clone = t.path().join("obsidian");
    std::process::Command::new("git").args(["clone", "-q", "-b", "main"]).arg(&remote).arg(&clone).status().unwrap();
    assert!(clone.join("shop/changelog.md").exists());
    assert!(std::fs::read_to_string(clone.join(".gitignore")).unwrap().contains(".obsidian/workspace.json"));

    // Both sides write.
    write(&clone.join(".obsidian/workspace.json"), "{\"open\": \"notes\"}");
    write(&clone.join(".obsidian/app.json"), "{}");
    write(&clone.join("shop/notes.md"), "# Заметки\n\nСклад работает до 22:00.\n\nВолны — каждые 15 минут.\n");
    push(&clone, "notes from Obsidian");
    let r = &h.router;
    let (_, _, cookies) = call(r, "POST", "/api/auth/login").json(json!({ "login": "anna", "password": "password-1" })).send().await;
    let session = cookies[0].clone();
    let (s, e, _) = call(r, "POST", "/api/docs/page")
        .cookie(&session)
        .json(json!({ "path": "shop/server.md", "content": "# С сервера\n\nСтраница из веба.\n" }))
        .send()
        .await;
    assert!(s.is_success(), "{s}: {e}");
    let st = genie::vault_sync::sync(&h.app).unwrap().unwrap();
    assert!(st.ok, "{st:?}");
    assert_eq!((st.pulled, st.both.len()), (1, 0));
    assert!(st.pushed >= 1);
    assert!(vault.join("shop/notes.md").exists(), "Obsidian's page is on the server");
    h.app.with_vault(|v| v.refresh()).unwrap();
    let (s, page, _) = call(r, "GET", "/api/docs/page?path=shop/notes.md").cookie(&session).send().await;
    assert_eq!(s, StatusCode::OK, "{page}");
    git(&clone, &["pull", "-q", "origin", "main"]);
    assert!(clone.join("shop/server.md").exists(), "the server's page is in Obsidian");
    let tracked = git(&clone, &["ls-files"]);
    assert!(
        tracked.contains(".obsidian/app.json") && !tracked.contains("workspace.json"),
        "shared settings travel, open tabs do not:\n{tracked}"
    );

    // The same line edited in both places: the page keeps the server's version, the other goes next to it.
    write(&clone.join("shop/notes.md"), "# Заметки\n\nСклад работает до 23:00.\n\nВолны — каждые 15 минут.\n");
    push(&clone, "later closing");
    let (_, current, _) = call(r, "GET", "/api/docs/page?path=shop/notes.md").cookie(&session).send().await;
    let (s, e, _) = call(r, "POST", "/api/docs/page")
        .cookie(&session)
        .json(json!({ "path": "shop/notes.md", "content": "# Заметки\n\nСклад работает до 21:00.\n\nВолны — каждые 15 минут.\n", "baseHash": current["page"]["contentHash"] }))
        .send()
        .await;
    assert!(s.is_success(), "{s}: {e}");
    let st = genie::vault_sync::sync(&h.app).unwrap().unwrap();
    assert!(st.ok, "{st:?}");
    assert_eq!(st.both, ["shop/notes.md"]);
    assert!(std::fs::read_to_string(vault.join("shop/notes.md")).unwrap().contains("до 21:00"), "the server's version stays");
    let copy = std::fs::read_dir(vault.join("shop"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.starts_with("notes.conflict-") && n.ends_with(".md"))
        .expect("the remote's version is kept next to the page");
    assert!(std::fs::read_to_string(vault.join("shop").join(&copy)).unwrap().contains("до 23:00"));
    assert!(notes().iter().any(|n| n.kind == "vault" && n.body.contains(&copy)), "the admins hear of it: {:#?}", notes());
    git(&clone, &["pull", "-q", "origin", "main"]);
    assert!(clone.join("shop").join(&copy).exists(), "both versions reach Obsidian too");

    // Deleted there, edited here: the edited page stays and goes back to the remote.
    git(&clone, &["rm", "-q", "shop/server.md"]);
    push(&clone, "remove the server page");
    let (_, current, _) = call(r, "GET", "/api/docs/page?path=shop/server.md").cookie(&session).send().await;
    let (s, e, _) = call(r, "POST", "/api/docs/page")
        .cookie(&session)
        .json(json!({ "path": "shop/server.md", "content": "# С сервера\n\nСтраница из веба, дополнена.\n", "baseHash": current["page"]["contentHash"] }))
        .send()
        .await;
    assert!(s.is_success(), "{s}: {e}");
    let st = genie::vault_sync::sync(&h.app).unwrap().unwrap();
    assert!(st.ok, "{st:?}");
    assert!(st.conflicts.iter().any(|c| c.starts_with("shop/server.md: удалена в репозитории")), "{st:?}");
    assert!(std::fs::read_to_string(vault.join("shop/server.md")).unwrap().contains("дополнена"));
    assert!(!vault.join(".git/MERGE_HEAD").exists(), "no merge is left half-done");
    git(&clone, &["pull", "-q", "origin", "main"]);
    assert!(clone.join("shop/server.md").exists());

    // The remote is gone: the server keeps its copy and tells the admins once.
    std::fs::rename(&remote, t.path().join("moved.git")).unwrap();
    for _ in 0..3 {
        let st = genie::vault_sync::sync(&h.app).unwrap().unwrap();
        assert!(!st.ok && st.error.is_some(), "{st:?}");
    }
    let failures = notes().into_iter().filter(|n| n.title.contains("не удалась")).count();
    assert_eq!(failures, 1, "the same failure is told once");
    std::fs::rename(t.path().join("moved.git"), &remote).unwrap();
    assert!(genie::vault_sync::sync(&h.app).unwrap().unwrap().ok);
}
