use crate::common;

use axum::http::StatusCode;
use common::{Harness, call};
use genie_core::Role;
use genie_core::vault::{Policy, Publish, RequiredPage, Section};
use serde_json::json;

#[tokio::test]
async fn people_write_agents_propose_owners_approve() {
    let h = Harness::with_config(|cfg| cfg.vault.commit = Some(true));
    h.project("shop");
    let r = &h.router;
    let page = "---\ntitle: Экспорт\ntype: guide\nstatus: current\n---\n# Экспорт\n\nЗаказы выгружаются в CSV.\n";
    let (s, saved, _) =
        call(r, "POST", "/api/docs/page").json(json!({ "path": "shop/export.md", "content": page, "mode": "create" })).send().await;
    assert_eq!(s, StatusCode::CREATED, "{saved}");
    let hash = saved["page"]["contentHash"].as_str().unwrap().to_string();
    let (s, _, _) =
        call(r, "POST", "/api/docs/page").json(json!({ "path": "shop/export.md", "content": page, "mode": "create" })).send().await;
    assert_eq!(s, StatusCode::CONFLICT);

    let (_, tree, _) = call(r, "GET", "/api/docs/tree").send().await;
    assert!(tree["pages"].as_array().unwrap().iter().any(|p| p["path"] == "shop/changelog.md"), "every project space has a changelog");
    let (_, found, _) = call(r, "GET", "/api/docs/search?q=выгружаются").send().await;
    assert_eq!(found["results"][0]["path"], "shop/export.md");

    let agent =
        h.app.with_server(|db| db.create_agent_token("shop", Role::Documenter, "tolkien", None, None, chrono::Duration::hours(1))).unwrap();
    let edited = page.replace("CSV.", "CSV и XLSX.");
    let (s, prop, _) = call(&h.remote, "POST", "/api/docs/page")
        .bearer(&agent)
        .no_csrf()
        .json(json!({ "path": "shop/export.md", "content": edited, "note": "XLSX", "task": "G-1" }))
        .send()
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "agents propose by default: {prop}");
    let id = prop["proposal"].as_i64().unwrap();
    let (_, listed, _) = call(r, "GET", "/api/docs/proposals").send().await;
    assert_eq!(listed[0]["id"], id);
    let (_, one, _) = call(r, "GET", &format!("/api/docs/proposals/{id}")).send().await;
    assert!(one["current"].as_str().unwrap().contains("CSV."));
    let (s, _, _) = call(&h.remote, "POST", &format!("/api/docs/proposals/{id}/approve")).bearer(&agent).no_csrf().send().await;
    assert_eq!(s, StatusCode::FORBIDDEN, "agents do not approve");
    let (s, done, _) = call(r, "POST", &format!("/api/docs/proposals/{id}/approve")).send().await;
    assert_eq!(s, StatusCode::OK, "{done}");
    let (_, read, _) = call(r, "GET", "/api/docs/page?path=shop/export.md").send().await;
    assert!(read["content"].as_str().unwrap().contains("XLSX"));
    let vault = h.app.with_vault(|v| Ok(v.root().to_path_buf())).unwrap();
    let committed = crate::common::githost::sh(&vault, &["show", "HEAD:shop/export.md"]);
    assert!(committed.contains("CSV и XLSX."), "the approved document is committed to Git: {committed}");

    // A stale edit is a conflict, not a silent overwrite.
    let (s, e, _) =
        call(r, "POST", "/api/docs/page").json(json!({ "path": "shop/export.md", "content": page, "baseHash": hash })).send().await;
    assert_eq!(s, StatusCode::CONFLICT, "{e}");

    let events = h.app.with_tracker("shop", |t| t.events_after(0, 100)).unwrap();
    let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
    assert!(kinds.contains(&"doc.changed") && kinds.contains(&"doc.proposal"), "{kinds:?}");
}

#[tokio::test]
async fn locked_sections_and_changelog_release() {
    let h = Harness::new();
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
    let agent =
        h.app.with_server(|db| db.create_agent_token("shop", Role::Documenter, "tolkien", None, None, chrono::Duration::hours(1))).unwrap();
    let (s, _, _) = call(&h.remote, "POST", "/api/docs/page")
        .bearer(&agent)
        .no_csrf()
        .json(json!({ "path": "shop/processes/release.md", "content": "# R\n\nx" }))
        .send()
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

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
    for (path, content) in [
        ("shop/ceny.md", "---\ntitle: Цены\npaths: [src/**]\n---\n# Цены\n\nКак считаются цены.\n"),
        ("shop/vozvraty.md", "---\ntitle: Возвраты\nrelated: [G-1]\n---\n# Возвраты\n\nПравила возвратов.\n"),
        ("shop/sklad.md", "---\ntitle: Склад\npaths: [docs/**]\n---\n# Склад\n\nВолны.\n"),
    ] {
        let (s, e, _) = call(r, "POST", "/api/docs/page").json(json!({ "path": path, "content": content })).send().await;
        assert!(s.is_success(), "{s}: {e}");
    }
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
    let h = Harness::new();
    h.project("shop");
    let r = &h.router;
    let export = "---\ntitle: Export\ntype: guide\nstatus: current\nsummary: How orders leave the shop\nrelated: [G-1]\n---\n# Export\n\n## Formats\n\nOrders go out as CSV and XLSX.\n";
    let billing =
        "---\ntitle: Billing\ntype: reference\nstatus: current\n---\n# Billing\n\nInvoices are monthly.\n\nBody-only-secret-phrase.\n";
    for (path, content) in [("shop/features/export.md", export), ("shop/features/billing.md", billing)] {
        let (s, v, _) = call(r, "POST", "/api/docs/page").json(json!({ "path": path, "content": content, "mode": "create" })).send().await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
    }
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
    use genie_core::vault::{AuthorKind, Space};

    let h = Harness::new();
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
    let (_, read, _) = call(&h.router, "GET", "/api/docs/page?project=shop&path=shop/guide.md").send().await;
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
async fn documentation_proposals_do_not_cross_project_boundaries() {
    let h = Harness::new();
    h.project("shop");
    h.project("other");
    let agent =
        h.app.with_server(|db| db.create_agent_token("other", Role::Documenter, "writer", None, None, chrono::Duration::hours(1))).unwrap();
    let (_, proposed, _) = call(&h.remote, "POST", "/api/docs/page")
        .bearer(&agent)
        .no_csrf()
        .json(json!({ "path": "other/proposal.md", "content": "# Proposal" }))
        .send()
        .await;
    let id = proposed["proposal"].as_i64().unwrap();
    let (_, list, _) = call(&h.router, "GET", "/api/docs/proposals?project=shop").send().await;
    assert_eq!(list, json!([]));
    h.app
        .with_server(|db| {
            db.create_proposal("shop/local.md", None, "# Local", "writer", "agent", Some("shop"), None, "fixture")?;
            for _ in 0..205 {
                db.create_proposal("other/new.md", None, "# Foreign", "writer", "agent", Some("other"), None, "fixture")?;
            }
            Ok(())
        })
        .unwrap();
    let (_, list, _) = call(&h.router, "GET", "/api/docs/proposals?project=shop").send().await;
    assert_eq!(list.as_array().unwrap().len(), 1, "foreign proposals cannot crowd out the project's proposals");
    assert_eq!(list[0]["path"], "shop/local.md");
    for (method, suffix) in [("GET", ""), ("POST", "/approve"), ("POST", "/reject")] {
        let (s, _, _) = call(&h.router, method, &format!("/api/docs/proposals/{id}{suffix}?project=shop")).send().await;
        assert_eq!(s, StatusCode::NOT_FOUND);
    }
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
    let page = "---\ntitle: Гид\ntype: guide\nstatus: current\n---\n# Гид\n\nТекст.\n";
    let (s, saved, _) =
        call(r, "POST", "/api/docs/page").json(json!({ "path": "shop/guides/setup.md", "content": page, "mode": "create" })).send().await;
    assert_eq!(s, StatusCode::CREATED, "{saved}");

    let (s, _, _) = call(r, "DELETE", "/api/docs/section?path=shop").send().await;
    assert!(!s.is_success(), "a space root is not a section");
    let (s, removed, _) = call(r, "DELETE", "/api/docs/section?path=shop/guides").send().await;
    assert_eq!(s, StatusCode::OK, "{removed}");
    assert_eq!(removed["removed"], 1);
    let (s, _, _) = call(r, "GET", "/api/docs/page?path=shop/guides/setup.md").send().await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let (s, _, _) =
        call(r, "POST", "/api/docs/page").json(json!({ "path": "shop/notes.md", "content": page, "mode": "create" })).send().await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, gone, _) = call(r, "DELETE", "/api/docs/page?path=shop/notes.md").send().await;
    assert_eq!(s, StatusCode::OK, "{gone}");
    let (s, _, _) = call(r, "GET", "/api/docs/page?path=shop/notes.md").send().await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn required_reading_lists_a_roles_pages_whole_and_names_missing_ones() {
    let h = Harness::new();
    h.project("shop");
    let guide = "---\ntitle: Agent guide\ntype: guide\nstatus: current\n---\n# Agent guide\n\nRead me first, before the rest.\n";
    let (s, saved, _) = call(&h.router, "POST", "/api/docs/page")
        .json(json!({ "path": "shop/agent-guide.md", "content": guide, "mode": "create" }))
        .send()
        .await;
    assert_eq!(s, StatusCode::CREATED, "{saved}");
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
