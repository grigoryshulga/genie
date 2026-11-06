use crate::common;

use axum::http::StatusCode;
use common::{Harness, call};
use genie_core::Role;
use genie_core::changesets::{Changeset, ChangesetOrigin, NewChangeset, ReviewDecision};
use genie_core::server_db::{ProjectRole, ServerDb};
use genie_core::vault::AuthorKind;
use genie_core::vault::{Policy, Publish, RequiredPage, Section, Space};
use serde_json::json;

/// Pages written straight into the vault: the fixtures of tests that are not about the write route.
fn seed(h: &Harness, pages: &[(&str, &str)]) {
    h.app
        .with_vault(|v| {
            for (path, content) in pages {
                v.write(path, content, AuthorKind::System, ("test", "test"), None, "fixture")?;
            }
            Ok(())
        })
        .unwrap();
}

fn agent_token(h: &Harness, project: &str, role: Role, name: &str) -> String {
    h.app.with_server(|db| db.create_agent_token(project, role, name, None, None, chrono::Duration::hours(1))).unwrap()
}

fn person_token(h: &Harness, project: &str, login: &str, role: ProjectRole) -> String {
    let user = h.app.with_server(|db| db.create_user(login, login, None, Some("password-1"), false)).unwrap();
    h.app.with_server(|db| db.set_membership(project, user.id, role)).unwrap();
    h.app.with_server(|db| db.create_user_token(user.id, "test")).unwrap()
}

/// A changeset opened straight in the database, touching one page.
fn db_changeset(db: &ServerDb, author: &str, task: Option<&str>, project: &str, path: &str) -> Changeset {
    let cs = db
        .create_changeset(&NewChangeset {
            branch: "",
            project: Some(project),
            task,
            author,
            author_name: author,
            author_kind: "human",
            title: "Docs",
            base: "abc123",
            origin: ChangesetOrigin::Genie,
            reverts: None,
        })
        .unwrap();
    db.touch_changeset(cs.id, &[path.to_string()]).unwrap()
}

const PAGE: &str = "---\ntitle: Экспорт\ntype: guide\nstatus: current\n---\n# Экспорт\n\nЗаказы выгружаются в CSV.\n";

/// An agent edits a section in its changeset, reads its own version, publishes, and the
/// owners review the published changeset; another person reads main throughout.
#[tokio::test]
async fn agents_edit_in_a_changeset_publish_it_and_the_owners_review_it() {
    let h = Harness::with_config(|cfg| cfg.vault.commit = Some(true));
    h.project("shop");
    seed(&h, &[("shop/export.md", PAGE)]);
    let documenter = agent_token(&h, "shop", Role::Documenter, "tolkien");

    let (s, edited, _) = call(&h.remote, "POST", "/api/docs/edit")
        .bearer(&documenter)
        .no_csrf()
        .json(json!({
            "path": "shop/export.md",
            "task": "G-1",
            "note": "the export gained XLSX",
            "ops": [{ "op": "replace-section", "heading": "Экспорт", "body": "# Экспорт\n\nЗаказы выгружаются в CSV и XLSX." }],
        }))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{edited}");
    let id = edited["changeset"]["id"].as_i64().expect("a changeset");
    assert_eq!(edited["changeset"]["author"], "tolkien");
    assert_eq!(edited["changeset"]["task"], "G-1");
    assert_eq!(edited["page"]["changeset"].as_i64(), Some(id), "the response reads the new version: {edited}");

    // Read-your-writes: the author reads the branch version; another person reads main and
    // sees that the page has an open changeset.
    let (s, own, _) = call(&h.remote, "GET", "/api/docs/page?path=shop/export.md").bearer(&documenter).no_csrf().send().await;
    assert_eq!(s, StatusCode::OK, "{own}");
    assert!(own["content"].as_str().unwrap().contains("CSV и XLSX"), "{own}");
    assert_eq!(own["changeset"].as_i64(), Some(id));
    let vic = person_token(&h, "shop", "vic", ProjectRole::Member);
    let (_, theirs, _) = call(&h.remote, "GET", "/api/docs/page?path=shop/export.md").bearer(&vic).no_csrf().send().await;
    assert!(theirs["content"].as_str().unwrap().contains("CSV."), "main still has the published text: {theirs}");
    assert!(!theirs["content"].as_str().unwrap().contains("XLSX"), "{theirs}");
    assert_eq!(theirs["openChangesets"][0]["id"], json!(id), "{theirs}");
    assert_eq!(theirs["openChangesets"][0]["author"], "tolkien");

    // The feed shows the open changeset; publishing closes it and flags it for review.
    let (_, feed, _) = call(&h.remote, "GET", "/api/docs/changesets?status=open&mine=1").bearer(&documenter).no_csrf().send().await;
    assert_eq!(feed[0]["id"], json!(id), "{feed}");
    let (s, published, _) =
        call(&h.remote, "POST", &format!("/api/docs/changesets/{id}/publish")).bearer(&documenter).no_csrf().send().await;
    assert_eq!(s, StatusCode::OK, "{published}");
    assert_eq!(published["status"], "published", "{published}");
    let commit = published["commit"].as_str().expect("the published commit").to_string();
    let (_, read, _) = call(&h.remote, "GET", "/api/docs/page?path=shop/export.md").bearer(&vic).no_csrf().send().await;
    assert!(read["content"].as_str().unwrap().contains("XLSX"), "{read}");
    assert_eq!(read["changeset"], json!(null), "the caller has no changeset: {read}");
    assert_eq!(read["unreviewed"][0]["id"], json!(id), "an agent's publication awaits the owner: {read}");

    let (_, feed, _) = call(&h.remote, "GET", "/api/docs/changesets?unreviewed=1").bearer(&vic).no_csrf().send().await;
    assert!(feed.as_array().unwrap().iter().any(|c| c["id"] == json!(id)), "{feed}");
    let (s, _, _) = call(&h.remote, "POST", &format!("/api/docs/changesets/{id}/review"))
        .bearer(&vic)
        .no_csrf()
        .json(json!({ "decision": "reviewed" }))
        .send()
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "only the touched section's owners review");
    let anna = person_token(&h, "shop", "anna", ProjectRole::Admin);
    let (s, reviewed, _) = call(&h.remote, "POST", &format!("/api/docs/changesets/{id}/review"))
        .bearer(&anna)
        .no_csrf()
        .json(json!({ "decision": "reviewed" }))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{reviewed}");
    assert_eq!(reviewed["changeset"]["reviewedBy"], "anna");
    let (_, feed, _) = call(&h.remote, "GET", "/api/docs/changesets?unreviewed=1").bearer(&anna).no_csrf().send().await;
    assert_eq!(feed, json!([]), "a reviewed publication leaves the queue");

    // History and diff read git on main.
    let (s, history, _) = call(&h.remote, "GET", "/api/docs/history?path=shop/export.md").bearer(&vic).no_csrf().send().await;
    assert_eq!(s, StatusCode::OK, "{history}");
    assert!(history.as_array().unwrap().iter().any(|c| c["sha"] == json!(commit)), "{history}");
    let base = edited["changeset"]["base"].as_str().unwrap();
    let (s, diff, _) =
        call(&h.remote, "GET", &format!("/api/docs/diff?path=shop/export.md&from={base}&to={commit}")).bearer(&vic).no_csrf().send().await;
    assert_eq!(s, StatusCode::OK, "{diff}");
    assert!(diff["diff"].as_str().unwrap().contains("XLSX"), "{diff}");
}

/// The compatibility write opens a changeset; a publication can be reverted with a new
/// changeset, and pages can be moved and removed inside one.
#[tokio::test]
async fn a_write_moves_and_removals_become_changesets_that_can_be_reverted() {
    let h = Harness::with_config(|cfg| cfg.vault.commit = Some(true));
    h.project("shop");
    let anna = person_token(&h, "shop", "anna", ProjectRole::Admin);

    let (s, created, _) = call(&h.remote, "POST", "/api/docs/page")
        .bearer(&anna)
        .no_csrf()
        .json(json!({ "path": "shop/export.md", "content": PAGE, "mode": "create" }))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{created}");
    let id = created["changeset"]["id"].as_i64().unwrap();
    assert_eq!(created["diagnostics"], json!([]));
    let (_, before, _) = call(&h.remote, "GET", "/api/docs/page?path=shop/export.md").bearer(&anna).no_csrf().send().await;
    let base = before["version"].as_str().unwrap().to_string();
    call(&h.remote, "POST", &format!("/api/docs/changesets/{id}/publish")).bearer(&anna).no_csrf().send().await;
    let (_, read, _) = call(&h.remote, "GET", "/api/docs/page?path=shop/export.md").bearer(&anna).no_csrf().send().await;
    assert!(read["content"].as_str().unwrap().contains("CSV."), "{read}");

    // Edit, publish and revert: the page goes back to the text before the changeset.
    let (s, edited, _) = call(&h.remote, "POST", "/api/docs/edit")
        .bearer(&anna)
        .no_csrf()
        .json(json!({
            "path": "shop/export.md",
            "note": "XLSX",
            "ops": [{ "op": "replace-section", "heading": "Экспорт", "body": "# Экспорт\n\nЗаказы выгружаются в CSV и XLSX." }],
        }))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{edited}");
    let id = edited["changeset"]["id"].as_i64().unwrap();
    let (s, body, _) = call(&h.remote, "POST", &format!("/api/docs/changesets/{id}/publish")).bearer(&anna).no_csrf().send().await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, reverted, _) = call(&h.remote, "POST", &format!("/api/docs/changesets/{id}/revert")).bearer(&anna).no_csrf().send().await;
    assert_eq!(s, StatusCode::OK, "{reverted}");
    assert_eq!(reverted["status"], "published", "{reverted}");
    let (_, read, _) = call(&h.remote, "GET", "/api/docs/page?path=shop/export.md").bearer(&anna).no_csrf().send().await;
    assert!(!read["content"].as_str().unwrap().contains("XLSX"), "the revert undoes the changeset: {read}");

    // Restore the page to its first version.
    let (s, restored, _) = call(&h.remote, "POST", "/api/docs/restore")
        .bearer(&anna)
        .no_csrf()
        .json(json!({ "path": "shop/export.md", "at": base }))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{restored}");
    assert_eq!(restored["status"], "published", "{restored}");

    // Move a page; its wiki-links follow in the same changeset.
    let (s, moved, _) = call(&h.remote, "POST", "/api/docs/pages/move")
        .bearer(&anna)
        .no_csrf()
        .json(json!({ "from": "shop/export.md", "to": "shop/archive/export.md" }))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{moved}");
    let moving = moved["changeset"]["id"].as_i64().unwrap();
    let (s, body, _) = call(&h.remote, "POST", &format!("/api/docs/changesets/{moving}/publish")).bearer(&anna).no_csrf().send().await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, _, _) = call(&h.remote, "GET", "/api/docs/page?path=shop/export.md").bearer(&anna).no_csrf().send().await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _, _) = call(&h.remote, "GET", "/api/docs/page?path=shop/archive/export.md").bearer(&anna).no_csrf().send().await;
    assert_eq!(s, StatusCode::OK);

    // Remove a page inside a changeset, then publish the removal.
    let (s, removed, _) = call(&h.remote, "DELETE", "/api/docs/page?path=shop/archive/export.md").bearer(&anna).no_csrf().send().await;
    assert_eq!(s, StatusCode::OK, "{removed}");
    let removal = removed["changeset"]["id"].as_i64().unwrap();
    let (s, body, _) = call(&h.remote, "POST", &format!("/api/docs/changesets/{removal}/publish")).bearer(&anna).no_csrf().send().await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, _, _) = call(&h.remote, "GET", "/api/docs/page?path=shop/archive/export.md").bearer(&anna).no_csrf().send().await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

/// A second publication of overlapping edits becomes a pending merge; the merge agent's
/// resolution over HTTP publishes it.
#[tokio::test]
async fn a_conflict_becomes_a_pending_merge_resolved_over_http() {
    let h = Harness::with_config(|cfg| cfg.vault.commit = Some(true));
    h.project("shop");
    seed(&h, &[("shop/export.md", PAGE)]);
    let anna = person_token(&h, "shop", "anna", ProjectRole::Admin);
    let vic = person_token(&h, "shop", "vic", ProjectRole::Member);

    let (s, out, _) = call(&h.remote, "POST", "/api/docs/edit")
        .bearer(&anna)
        .no_csrf()
        .json(json!({ "path": "shop/export.md", "note": "the export", "ops": [{ "op": "replace-section", "heading": "Экспорт", "body": "# Экспорт\n\nЗаказы выгружаются в CSV, XLSX и API." }] }))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{out}");
    let first = out["changeset"]["id"].as_i64().unwrap();
    let (s, out, _) = call(&h.remote, "POST", "/api/docs/edit")
        .bearer(&vic)
        .no_csrf()
        .json(json!({ "path": "shop/export.md", "note": "the export", "ops": [{ "op": "replace-section", "heading": "Экспорт", "body": "# Экспорт\n\nЗаказы выгружаются в CSV и почтой." }] }))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{out}");
    let second = out["changeset"]["id"].as_i64().unwrap();
    let (s, body, _) = call(&h.remote, "POST", &format!("/api/docs/changesets/{first}/publish")).bearer(&anna).no_csrf().send().await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let (s, published, _) = call(&h.remote, "POST", &format!("/api/docs/changesets/{second}/publish")).bearer(&vic).no_csrf().send().await;
    assert_eq!(s, StatusCode::OK, "{published}");
    assert_eq!(published["status"], "merging", "overlapping edits go to the merge agent: {published}");
    let merge = published["merge"]["id"].as_i64().expect("a pending merge");

    let (s, shown, _) = call(&h.remote, "GET", &format!("/api/docs/merges/{merge}")).bearer(&vic).no_csrf().send().await;
    assert_eq!(s, StatusCode::OK, "{shown}");
    assert!(shown["merge"]["conflicts"].as_array().is_some_and(|c| c.iter().any(|p| p == "shop/export.md")), "{shown}");
    let page = shown["pages"][0]["marked"].as_str().unwrap_or_default().to_string();
    assert!(page.contains("<<<<<<<") && page.contains(">>>>>>>"), "the conflicted page comes marked: {shown}");

    let merged = "---\ntitle: Экспорт\ntype: guide\nstatus: current\n---\n# Экспорт\n\nЗаказы выгружаются в CSV, XLSX, API и почтой.\n";
    let (s, resolved, _) = call(&h.remote, "POST", &format!("/api/docs/merges/{merge}/resolve"))
        .bearer(&vic)
        .no_csrf()
        .json(json!({ "pages": [{ "path": "shop/export.md", "content": merged }], "done": true, "note": "kept both sides" }))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{resolved}");
    let (_, read, _) = call(&h.remote, "GET", "/api/docs/page?path=shop/export.md").bearer(&anna).no_csrf().send().await;
    assert!(read["content"].as_str().unwrap().contains("API и почтой"), "{read}");
}

/// The Changes feed filters by status, author, task, page and review state, and stays inside
/// the project's roots.
#[tokio::test]
async fn the_changes_feed_filters_and_stays_inside_the_projects_roots() {
    let h = Harness::new();
    h.project("shop");
    h.project("other");
    let anna = person_token(&h, "shop", "anna", ProjectRole::Member);
    let open = h
        .app
        .with_server(|db| {
            let open = db_changeset(db, "anna", Some("G-1"), "shop", "shop/export.md");
            let published = db_changeset(db, "tolkien", Some("G-2"), "shop", "shop/import.md");
            let published = db.mark_published(published.id, "sha-import", true, "tolkien")?;
            let reviewed = db_changeset(db, "tolkien", Some("G-3"), "shop", "shop/billing.md");
            let reviewed = db.mark_published(reviewed.id, "sha-billing", true, "tolkien")?;
            let reviewed = db.mark_reviewed(reviewed.id, "anna", ReviewDecision::Reviewed)?;
            let foreign = db_changeset(db, "someone", None, "other", "other/private.md");
            Ok::<_, genie_core::GenieError>((open.id, published.id, reviewed.id, foreign.id))
        })
        .unwrap();
    let (open, published, reviewed, foreign) = open;
    let get = |query: &'static str| {
        let anna = anna.clone();
        let router = h.remote.clone();
        async move {
            let (s, body, _) = call(&router, "GET", &format!("/api/docs/changesets{query}")).bearer(&anna).no_csrf().send().await;
            assert_eq!(s, StatusCode::OK, "{body}");
            body.as_array().cloned().unwrap_or_default().iter().filter_map(|c| c["id"].as_i64()).collect::<Vec<_>>()
        }
    };
    let all = get("").await;
    assert!(all.contains(&open) && all.contains(&published) && all.contains(&reviewed), "{all:?}");
    assert!(!all.contains(&foreign), "another project's changesets stay out: {all:?}");
    assert_eq!(get("?status=open").await, [open], "open");
    assert_eq!(get("?status=published").await, [reviewed, published], "published, newest first");
    assert_eq!(get("?author=anna").await, [open], "by author");
    assert_eq!(get("?task=G-2").await, [published], "by task");
    assert_eq!(get("?path=shop/billing.md").await, [reviewed], "by page");
    assert_eq!(get("?unreviewed=1").await, [published], "published and not yet reviewed");
    assert_eq!(get("?mine=1&status=open").await, [open], "the caller's own open changesets");
    let (s, _, _) = call(&h.remote, "GET", "/api/docs/changesets?status=nonsense").bearer(&anna).no_csrf().send().await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "an unknown status is refused");
}

/// Changesets belong to their project: another project's changeset is not found.
#[tokio::test]
async fn another_projects_changeset_is_not_found() {
    let h = Harness::with_config(|cfg| cfg.vault.commit = Some(true));
    h.project("shop");
    h.project("other");
    let foreign = h.app.with_server(|db| Ok(db_changeset(db, "someone", None, "other", "other/private.md"))).unwrap();
    let (s, body, _) = call(&h.router, "GET", &format!("/api/docs/changesets/{}?project=shop", foreign.id)).send().await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{body}");
    let (s, body, _) = call(&h.router, "POST", &format!("/api/docs/changesets/{}/publish?project=shop", foreign.id)).send().await;
    assert_eq!(s, StatusCode::NOT_FOUND, "publishing another project's work is not found either: {body}");
}

/// Proposals are gone: their routes answer 404 and the catalog no longer names them.
#[tokio::test]
async fn proposal_routes_and_operations_are_gone() {
    let h = Harness::new();
    h.project("shop");
    for (method, uri) in [
        ("GET", "/api/docs/proposals"),
        ("GET", "/api/docs/proposals/1"),
        ("POST", "/api/docs/proposals/1/approve"),
        ("POST", "/api/docs/proposals/1/reject"),
    ] {
        let (s, _, _) = call(&h.router, method, uri).json(json!({ "note": "x" })).send().await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{method} {uri}");
    }
    for name in ["proposals", "proposal", "approve", "reject"] {
        assert!(genie::ops::find("docs", name).is_none(), "{name} stays in the catalog");
    }
}

#[tokio::test]
async fn a_locked_section_refuses_agents() {
    let h = Harness::with_config(|cfg| cfg.vault.commit = Some(true));
    h.project("shop");
    h.app
        .with_vault(|v| {
            v.config.spaces.get_mut("shop").unwrap().sections.insert(
                "processes".into(),
                Section { policy: Some(Policy { humans: Publish::Direct, agents: Publish::Locked }), owners: vec![] },
            );
            v.save_config()
        })
        .unwrap();
    let agent = agent_token(&h, "shop", Role::Documenter, "tolkien");
    let (s, body, _) = call(&h.remote, "POST", "/api/docs/edit")
        .bearer(&agent)
        .no_csrf()
        .json(json!({ "path": "shop/processes/release.md", "ops": [{ "op": "write-page", "content": "# R\n\nx" }] }))
        .send()
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{body}");
}

#[tokio::test]
async fn changelog_release_moves_unreleased_entries_to_a_version() {
    let h = Harness::new();
    h.project("shop");
    genie::knowledge::changelog_add(&h.app, "shop", "added", "Экспорт в CSV", Some("G-1")).unwrap();
    let (_, cl, _) = call(&h.router, "GET", "/api/docs/changelog").send().await;
    assert!(cl["content"].as_str().unwrap().contains("### Добавлено\n\n- Экспорт в CSV (G-1)"), "{cl}");
    let (s, rel, _) = call(&h.router, "POST", "/api/docs/changelog/release").json(json!({ "version": "1.0.0" })).send().await;
    assert_eq!(s, StatusCode::OK, "{rel}");
    assert!(rel["notes"].as_str().unwrap().contains("Экспорт в CSV"));
    let events = h.app.with_tracker("shop", |t| t.events_after(0, 100)).unwrap();
    assert!(events.iter().any(|e| e.kind == "release.published"));
}

#[tokio::test]
async fn a_task_in_review_names_the_pages_its_changes_may_have_made_stale() {
    let h = common::Harness::new();
    let repo = h.dir.path().join("shop-repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/pricing.abap"), "CLASS zcl_pricing DEFINITION.\nENDCLASS.\n").unwrap();
    let git = |dir: &std::path::Path, args: &[&str]| {
        let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    };
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["add", "."]);
    git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "init"]);
    h.app.create_project("shop", "Магазин", Some(&repo.to_string_lossy()), None, None).unwrap();
    let r = &h.router;
    call(r, "POST", "/api/tasks").json(json!({ "title": "Скидки в возвратах", "description": "d", "acceptance": ["a"] })).send().await;
    call(r, "POST", "/api/tasks/G-1/status").json(json!({ "status": "ready" })).send().await;
    let (s, team, _) = call(r, "POST", "/api/teams").json(json!({ "task": "G-1", "template": "pair" })).send().await;
    assert_eq!(s, StatusCode::CREATED, "{team}");
    let worktree = std::path::PathBuf::from(team["worktree"]["path"].as_str().unwrap());
    std::fs::write(worktree.join("src/pricing.abap"), "CLASS zcl_pricing DEFINITION.\n* returns\nENDCLASS.\n").unwrap();
    seed(
        &h,
        &[
            ("shop/ceny.md", "---\ntitle: Цены\npaths: [src/**]\n---\n# Цены\n\nКак считаются цены.\n"),
            ("shop/vozvraty.md", "---\ntitle: Возвраты\nrelated: [G-1]\n---\n# Возвраты\n\nПравила возвратов.\n"),
            ("shop/sklad.md", "---\ntitle: Склад\npaths: [docs/**]\n---\n# Склад\n\nВолны.\n"),
        ],
    );
    let (s, impact, _) = call(r, "GET", "/api/tasks/G-1/docs-impact").send().await;
    assert_eq!(s, StatusCode::OK, "{impact}");
    assert_eq!(impact["changedPathsAvailable"], true, "{impact}");
    assert_eq!(impact["changedPaths"], json!(["src/pricing.abap"]));
    let pages: Vec<&str> = impact["candidates"].as_array().unwrap().iter().map(|c| c["path"].as_str().unwrap()).collect();
    assert_eq!(pages, ["shop/ceny.md", "shop/vozvraty.md"], "changed paths first, then pages naming the task: {impact}");
    assert_eq!(impact["candidates"][0]["reasons"][0], json!({ "kind": "changed-path", "path": "src/pricing.abap", "pattern": "src/**" }));
    assert_eq!(impact["candidates"][1]["reasons"][0], json!({ "kind": "related", "id": "G-1" }));
    assert_eq!(impact["applicable"], false, "the hint is meant for review and done");
}

/// Agents know the project's knowledge base without asking: its pages by title
/// in their prompt (L0) and the pages chosen for a task in their kickoff (L1).
#[tokio::test]
async fn agents_get_the_index_in_their_prompt_and_the_tasks_pages_in_their_kickoff() {
    let h = Harness::with_config(|cfg| cfg.vault.commit = Some(true));
    h.project("shop");
    let r = &h.router;
    let export = "---\ntitle: Export\ntype: guide\nstatus: current\nsummary: How orders leave the shop\nrelated: [G-1]\n---\n# Export\n\n## Formats\n\nOrders go out as CSV and XLSX.\n";
    let billing =
        "---\ntitle: Billing\ntype: reference\nstatus: current\n---\n# Billing\n\nInvoices are monthly.\n\nBody-only-secret-phrase.\n";
    seed(&h, &[("shop/features/export.md", export), ("shop/features/billing.md", billing)]);
    call(r, "POST", "/api/tasks").json(json!({ "title": "CSV export" })).send().await;

    // L0: every page of the space, metadata only, in the orchestrator's prompt.
    let (s, console, _) = call(r, "POST", "/api/orchestrator/console").json(json!({})).send().await;
    assert_eq!(s, StatusCode::OK, "{console}");
    let prompt = console["prompt"].as_str().unwrap();
    assert!(prompt.contains("## Project knowledge (L0 index)"), "{prompt}");
    assert!(
        prompt.contains("- shop/features/export.md — Export (guide, current, ") && prompt.contains("How orders leave the shop"),
        "{prompt}"
    );
    assert!(prompt.contains("- shop/features/billing.md — Billing (reference, current, "));
    assert!(prompt.contains("Invoices are monthly"), "a page without a summary is summed up by its first paragraph");
    assert!(!prompt.contains("Body-only-secret-phrase"), "L0 carries no page bodies");

    // L1: the task's pages in the kickoff — in the template's preview and in a real team's first mail.
    let (_, preview, _) = call(r, "POST", "/api/templates/pair/preview").json(json!({ "task": "G-1" })).send().await;
    let kickoff = preview["members"][0]["kickoff"].as_str().unwrap();
    assert!(kickoff.contains("## Project knowledge (L1 context)"), "{kickoff}");
    assert!(kickoff.contains("### Export — shop/features/export.md (guide, current) — related G-1"), "{kickoff}");
    assert!(kickoff.contains("Orders go out as CSV and XLSX.") && !kickoff.contains("Body-only-secret-phrase"), "{kickoff}");
    let (s, team, _) = call(r, "POST", "/api/teams").json(json!({ "task": "G-1", "template": "research" })).send().await;
    assert_eq!(s, StatusCode::CREATED, "{team}");
    let member = team["members"][0]["name"].as_str().unwrap().to_string();
    let mail = h.app.with_tracker("shop", |t| t.bus().pending(Some("G-1"), &member)).unwrap();
    assert!(mail.iter().any(|m| m.text.contains("### Export — shop/features/export.md")), "the kickoff carries the page");
}

#[tokio::test]
async fn documentation_is_confined_to_the_selected_projects_roots() {
    let h = Harness::with_config(|cfg| cfg.vault.commit = Some(true));
    h.project("shop");
    h.project("other");
    h.project("empty");
    h.app
        .with_vault(|v| {
            v.config.spaces.insert("handbook".into(), Space { project: Some("shop".into()), ..Default::default() });
            v.save_config()?;
            for (path, content) in [
                ("shop/guide.md", "---\ntitle: Orders\n---\n# Orders\n\nneedle [[guide]] [[other/private]]"),
                ("handbook/intro.md", "# Handbook\n\nneedle"),
                ("other/guide.md", "# Other\n\nneedle [[shop/guide]]"),
                ("other/private.md", "# Private\n\nneedle"),
                ("shared.md", "# Shared\n\nneedle [[shop/guide]]"),
            ] {
                v.write(path, content, AuthorKind::System, ("test", "test"), None, "fixture")?;
            }
            Ok(())
        })
        .unwrap();
    let (_, tree, _) = call(&h.router, "GET", "/api/docs/tree?project=shop").send().await;
    assert_eq!(tree["roots"], json!(["handbook", "shop"]));
    assert!(
        tree["pages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["path"].as_str().unwrap().starts_with("shop/") || p["path"].as_str().unwrap().starts_with("handbook/"))
    );
    for suffix in ["", "&mine=0", "&mine=1"] {
        let (_, found, _) = call(&h.router, "GET", &format!("/api/docs/search?project=shop&q=needle{suffix}")).send().await;
        let paths: Vec<_> = found["results"].as_array().unwrap().iter().map(|p| p["path"].as_str().unwrap()).collect();
        assert_eq!(paths.len(), 2, "{found}");
        assert!(paths.contains(&"shop/guide.md") && paths.contains(&"handbook/intro.md"));
    }
    let (s, read, _) = call(&h.router, "GET", "/api/docs/page?project=shop&path=shop/guide.md").send().await;
    assert_eq!(s, StatusCode::OK, "{read}");
    assert_eq!(read["links"][0]["targetPath"], "shop/guide.md", "a foreign basename cannot make the local link ambiguous");
    assert_eq!(read["links"][1]["resolution"], "unresolved");
    assert_eq!(read["links"][1]["matches"], json!([]));
    assert_eq!(read["backlinks"], json!([]), "foreign backlinks do not disclose page paths");
    for path in ["other/private.md", "/other/private", "other\\private.md", "shared.md", "other/new.md"] {
        let (s, _, _) = call(&h.router, "GET", &format!("/api/docs/page?project=shop&path={}", path.replace('\\', "%5C"))).send().await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{path}");
        let (s, _, _) =
            call(&h.router, "POST", "/api/docs/page?project=shop").json(json!({ "path": path, "content": "# Replacement" })).send().await;
        assert_eq!(s, StatusCode::NOT_FOUND, "cannot write outside the roots: {path}");
    }
    let (_, spaces, _) = call(&h.router, "GET", "/api/docs/spaces?project=shop").send().await;
    assert_eq!(spaces["spaces"].as_object().unwrap().len(), 2);
    let (_, before, _) = call(&h.router, "GET", "/api/docs/version?project=shop").send().await;
    h.app
        .with_vault(|v| v.write("other/private.md", "# Changed foreign page", AuthorKind::System, ("test", "test"), None, "fixture"))
        .unwrap();
    let (_, after, _) = call(&h.router, "GET", "/api/docs/version?project=shop").send().await;
    assert_eq!(before, after, "other projects do not invalidate this project's documentation");
    h.app
        .with_vault(|v| {
            v.config.spaces.remove("empty");
            v.save_config()
        })
        .unwrap();
    let (_, found, _) = call(&h.router, "GET", "/api/docs/search?project=empty&q=needle").send().await;
    assert_eq!(found["results"], json!([]), "a project without roots cannot fall back to the whole vault");
    let (s, _, _) = call(&h.router, "POST", "/api/docs/note?project=empty").json(json!({ "title": "Note" })).send().await;
    assert_eq!(s, StatusCode::NOT_FOUND, "notes cannot fall back to a shared inbox");
}

#[tokio::test]
async fn foreign_search_candidates_cannot_crowd_out_project_results() {
    let h = Harness::new();
    h.project("shop");
    h.project("other");
    h.app
        .with_vault(|v| {
            for n in 0..405 {
                let (_, path) = v.resolve(&format!("other/noise-{n}.md"))?;
                std::fs::write(path, "# Uniqueterm\n\nUniqueterm")?;
            }
            let (_, path) = v.resolve("shop/match.md")?;
            std::fs::write(path, "# Local\n\nUniqueterm appears here among other words.")?;
            v.refresh()?;
            Ok(())
        })
        .unwrap();
    let (s, found, _) = call(&h.router, "GET", "/api/docs/search?project=shop&q=Uniqueterm&limit=1").send().await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(found["results"].as_array().unwrap().len(), 1);
    assert_eq!(found["results"][0]["path"], "shop/match.md");
}

#[tokio::test]
async fn project_admins_remove_sections_with_their_pages() {
    let h = Harness::with_config(|cfg| cfg.vault.commit = Some(true));
    h.project("shop");
    let r = &h.router;
    seed(&h, &[("shop/guides/setup.md", PAGE)]);

    let (s, _, _) = call(r, "DELETE", "/api/docs/section?path=shop").send().await;
    assert!(!s.is_success(), "a space root is not a section");
    let (s, removed, _) = call(r, "DELETE", "/api/docs/section?path=shop/guides").send().await;
    assert_eq!(s, StatusCode::OK, "{removed}");
    assert_eq!(removed["removed"], 1);
    let (_, tree, _) = call(r, "GET", "/api/docs/tree").send().await;
    assert!(
        !tree["pages"].as_array().unwrap().iter().any(|p| p["path"] == "shop/guides/setup.md"),
        "the removed section takes its pages: {tree}"
    );
}

#[tokio::test]
async fn required_reading_lists_a_roles_pages_whole_and_names_missing_ones() {
    let h = Harness::with_config(|cfg| cfg.vault.commit = Some(true));
    h.project("shop");
    let guide = "---\ntitle: Agent guide\ntype: guide\nstatus: current\n---\n# Agent guide\n\nRead me first, before the rest.\n";
    seed(&h, &[("shop/agent-guide.md", guide)]);
    h.app
        .with_vault(|v| {
            v.config.spaces.get_mut("shop").unwrap().required = vec![
                RequiredPage { page: "shop/agent-guide.md".into(), roles: vec!["all".into()] },
                RequiredPage { page: "shop/executor-only.md".into(), roles: vec!["executor".into()] },
            ];
            v.save_config()
        })
        .unwrap();

    let (s, body, _) = call(&h.router, "GET", "/api/docs/required?role=executor").send().await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["role"], "executor");
    let pages = body["pages"].as_array().unwrap();
    assert_eq!(pages.len(), 1, "{body}");
    assert_eq!(pages[0]["path"], "shop/agent-guide.md");
    assert_eq!(pages[0]["title"], "Agent guide");
    let content = pages[0]["content"].as_str().unwrap();
    assert!(content.contains("Read me first") && !content.starts_with("---") && !content.contains("type: guide"), "{content}");
    assert_eq!(pages[0]["version"].as_str().unwrap().chars().count(), 12, "the version is the short content hash");
    assert_eq!(pages[0]["oversize"], false);
    assert_eq!(body["missing"], json!(["shop/executor-only.md"]));

    // A person must name the role; an agent's own role is used when it is omitted.
    let (s, _, _) = call(&h.router, "GET", "/api/docs/required").send().await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let agent = h
        .app
        .with_server(|db| {
            db.create_role_token("shop", Role::Documenter, Some("documenter"), "tolkien", None, None, chrono::Duration::hours(1))
        })
        .unwrap();
    let (s, own, _) = call(&h.remote, "GET", "/api/docs/required").bearer(&agent).no_csrf().send().await;
    assert_eq!(s, StatusCode::OK, "{own}");
    assert_eq!(own["role"], "documenter");
    assert_eq!(own["pages"].as_array().unwrap().len(), 1, "the executor-only page stays out: {own}");
    assert_eq!(own["missing"], json!([]));
}
