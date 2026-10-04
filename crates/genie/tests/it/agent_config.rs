//! Roles and team templates as server configuration: the admin API, agents
//! acting with their role's permissions, kickoffs and handoffs from relations.

use crate::common;

use axum::http::StatusCode;
use common::{Harness, call};
use genie_core::server_db::ProjectRole;
use genie_core::{Actor, CreateInput, Role, Status, StatusOptions};
use serde_json::{Value, json};

const QA: &str = "---\ntitle: QA\ndescription: Tests the change and ticks the criteria it verified.\nbase: tester\nallow: [task.check]\n---\nYou are the QA engineer of a focus team.\n";
const QUIET: &str =
    "---\ndescription: An executor that talks only to its team.\nbase: executor\ndeny: [mail.orchestrator]\n---\nYou implement the task.\n";

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

fn token(h: &Harness, class: Role, role_id: &str, name: &str, team: &str) -> String {
    h.app.with_server(|db| db.create_role_token("shop", class, Some(role_id), name, Some(team), None, chrono::Duration::hours(1))).unwrap()
}

fn kickoff_of(h: &Harness, team: &str, member: &str) -> String {
    h.app
        .with_tracker("shop", |t| t.bus().history(team, 100))
        .unwrap()
        .into_iter()
        .find(|m| m.kind == "kickoff" && m.to == member)
        .map(|m| m.text)
        .unwrap_or_default()
}

fn member_named(team: &Value, role: &str) -> String {
    team["members"].as_array().unwrap().iter().find(|m| m["role"] == role).unwrap()["name"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn admins_change_roles_and_templates_everyone_reads_them() {
    let h = Harness::new();
    h.project("shop");
    let (admin, member) = h
        .app
        .with_server(|db| {
            let a = db.create_user("root", "Root", None, Some("password1"), true)?;
            let m = db.create_user("pm", "PM", None, Some("password1"), false)?;
            db.set_membership("shop", m.id, ProjectRole::Member)?;
            Ok((db.create_user_token(a.id, "t")?, db.create_user_token(m.id, "t")?))
        })
        .unwrap();
    let r = &h.router;

    let (s, cat, _) = call(r, "GET", "/api/agent-config").bearer(&member).send().await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(cat["admin"], false);
    assert!(cat["teams"].as_array().unwrap().iter().any(|t| t["id"] == "standard"));
    assert!(cat["roles"].as_array().unwrap().iter().all(|r| r.get("prompt").is_none()), "the list leaves prompts out");
    assert_eq!(
        cat["classes"]["reviewer"],
        json!([
            "status.approve",
            "status.return",
            "task.check",
            "task.block",
            "docs.read",
            "docs.write",
            "mail.team",
            "mail.orchestrator",
            "team.peek"
        ]),
        "what each class starts from, for the web's checkboxes"
    );
    assert!(cat["mcpAdapter"].is_boolean());

    let (s, _, _) = call(r, "PUT", "/api/roles/qa").bearer(&member).json(json!({ "content": QA })).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN, "only server admins change roles");

    let (s, v, _) = call(r, "PUT", "/api/roles/qa").bearer(&admin).json(json!({ "content": QA, "baseHash": "" })).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(h.dir.path().join("agents/qa.md").exists(), "the role is a file in the data directory");
    let (_, v, _) = call(r, "GET", "/api/roles/qa").bearer(&member).send().await;
    assert_eq!(v["role"]["class"], "tester");
    assert!(v["role"]["capabilities"].as_array().unwrap().contains(&json!("task.check")));
    let hash = v["file"]["hash"].as_str().unwrap().to_string();

    let (s, _, _) = call(r, "PUT", "/api/roles/qa").bearer(&admin).json(json!({ "content": QA, "baseHash": "stale" })).send().await;
    assert_eq!(s, StatusCode::CONFLICT, "an edit started from an old version is refused");
    let (s, e, _) = call(r, "PUT", "/api/roles/qa")
        .bearer(&admin)
        .json(json!({ "content": "---\nbase: tester\nallow: [fly]\n---\nx\n", "baseHash": hash }))
        .send()
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(e["error"].as_str().unwrap().contains("unknown permission `fly`"), "{e}");
    assert!(std::fs::read_to_string(h.dir.path().join("agents/qa.md")).unwrap().contains("QA engineer"), "a refused edit is not written");

    let template = json!({
        "title": "QA pair",
        "description": "An executor and a QA engineer",
        "members": [{ "role": "executor" }, { "role": "qa" }],
        "relations": [
            { "from": "executor", "to": "qa", "type": "handoff", "on": "review", "note": "ready to test" },
            { "from": "qa", "to": "executor", "type": "returns", "on": "changes_requested" },
            { "from": "qa", "to": "orchestrator", "type": "reports", "note": "the test verdict" }
        ]
    });
    let (s, v, _) = call(r, "PUT", "/api/templates/qa-pair").bearer(&admin).json(json!({ "template": template })).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["problems"].as_array().unwrap().iter().any(|p| p["message"].as_str().unwrap().contains("status.approve")), "{v}");

    // The lists say how many automations use a template or a role (once per automation).
    let spec = json!({ "name": "QA on review", "on": { "event": "task.status_changed" }, "steps": [
        { "id": "t", "team": { "template": "qa-pair" } },
        { "id": "a", "agent": { "role": "analyst", "goal": "check" } },
        { "id": "b", "agent": { "role": "analyst", "goal": "check again" } }
    ]});
    h.app.with_server(|db| db.create_automation("shop", &spec, "root")).unwrap();
    let (_, cat, _) = call(r, "GET", "/api/agent-config").bearer(&member).send().await;
    assert_eq!(cat["automations"], json!({ "templates": { "qa-pair": 1 }, "roles": { "analyst": 1 } }));

    let (s, p, _) = call(r, "POST", "/api/templates/qa-pair/preview").bearer(&member).json(json!({})).send().await;
    assert_eq!(s, StatusCode::OK, "{p}");
    let kickoff =
        |role: &str| p["members"].as_array().unwrap().iter().find(|m| m["role"] == role).unwrap()["kickoff"].as_str().unwrap().to_string();
    assert!(kickoff("executor").contains("move the task to review with a note"), "{}", kickoff("executor"));
    assert!(kickoff("qa").contains("genie tells you when the task moves to review"), "{}", kickoff("qa"));

    let (s, e, _) = call(r, "DELETE", "/api/roles/qa").bearer(&admin).send().await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "the template still uses the role");
    assert!(e["error"].as_str().unwrap().contains("team:qa-pair"), "{e}");

    let (s, hist, _) = call(r, "GET", "/api/agent-config/history?item=role:qa").bearer(&admin).send().await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(hist.as_array().unwrap().len(), 1, "one saved edit: {hist}");
    assert_eq!(hist[0]["user"], "root");
    let (s, _, _) = call(r, "GET", "/api/agent-config/history").bearer(&member).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn agents_act_with_their_roles_permissions_and_status_handoffs_reach_teammates() {
    let h = Harness::new();
    h.project("shop");
    std::fs::create_dir_all(h.dir.path().join("agents")).unwrap();
    std::fs::write(h.dir.path().join("agents/qa.md"), QA).unwrap();
    std::fs::write(h.dir.path().join("agents/quiet.md"), QUIET).unwrap();
    h.app.reload_agents();
    let id = ready_task(&h, "CSV export");
    let r = &h.router;
    let (s, team, _) = call(r, "POST", "/api/teams")
        .json(json!({ "task": id, "members": [{ "role": "quiet" }, { "role": "qa" }], "note": "small change" }))
        .send()
        .await;
    assert_eq!(s, StatusCode::CREATED, "{team}");
    let (exec, qa) = (member_named(&team, "quiet"), member_named(&team, "qa"));
    let spec = &team["spec"];
    assert!(spec["relations"].as_array().unwrap().iter().any(|r| r["type"] == "handoff" && r["on"] == "review"), "{spec}");
    let k = kickoff_of(&h, &id, &qa);
    assert!(k.contains("You start after") && k.contains("genie tells you when the task moves to review"), "{k}");

    let (exec_tok, qa_tok) = (token(&h, Role::Executor, "quiet", &exec, &id), token(&h, Role::Tester, "qa", &qa, &id));
    let remote = &h.remote;
    let (s, e, _) = call(remote, "POST", &format!("/api/teams/{id}/mail"))
        .bearer(&exec_tok)
        .json(json!({ "to": "orchestrator", "text": "hi" }))
        .send()
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the role denies writing to the orchestrator");
    assert!(e["error"].as_str().unwrap().contains("mail.orchestrator"), "{e}");
    let (s, _, _) =
        call(remote, "POST", &format!("/api/teams/{id}/mail")).bearer(&exec_tok).json(json!({ "to": qa, "text": "heads up" })).send().await;
    assert_eq!(s, StatusCode::CREATED);

    for status in ["in_progress", "review"] {
        let (s, t, _) = call(remote, "POST", &format!("/api/tasks/{id}/status"))
            .bearer(&exec_tok)
            .json(json!({ "status": status, "note": "export works" }))
            .send()
            .await;
        assert_eq!(s, StatusCode::OK, "{t}");
    }
    genie::engine::tick(&h.app).unwrap();
    let mail = h.app.with_tracker("shop", |t| t.bus().history(&id, 100)).unwrap();
    let handoff = mail.iter().find(|m| m.to == qa && m.kind == "system").expect("genie hands the work to QA");
    assert!(
        handoff.text.contains("moved") && handoff.text.contains("to review") && handoff.text.contains("export works"),
        "{}",
        handoff.text
    );
    genie::engine::tick(&h.app).unwrap();
    let again = h.app.with_tracker("shop", |t| t.bus().history(&id, 100)).unwrap();
    assert_eq!(again.iter().filter(|m| m.to == qa && m.kind == "system").count(), 1, "each status change hands over once");

    let (s, t, _) =
        call(remote, "POST", &format!("/api/tasks/{id}/acceptance/1")).bearer(&qa_tok).json(json!({ "done": true })).send().await;
    assert_eq!(s, StatusCode::OK, "QA may tick criteria (allow: task.check): {t}");
    let plain = token(&h, Role::Tester, "tester", &qa, &id);
    let (s, _, _) =
        call(remote, "POST", &format!("/api/tasks/{id}/acceptance/1")).bearer(&plain).json(json!({ "done": false })).send().await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "a plain tester may not");
}

#[tokio::test]
async fn templates_decide_stages_and_the_spike_reaches_review() {
    let h = Harness::new();
    h.project("shop");
    let r = &h.router;
    let draft = h
        .app
        .with_tracker("shop", |t| {
            Ok(t.create(&Actor::new("orchestrator", Role::Orchestrator), CreateInput { title: "vague".into(), ..Default::default() })?.id)
        })
        .unwrap();
    let (s, e, _) = call(r, "POST", "/api/teams").json(json!({ "task": draft, "template": "standard" })).send().await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(e["error"].as_str().unwrap().contains("works only on ready tasks"), "{e}");
    let (s, team, _) = call(r, "POST", "/api/teams").json(json!({ "task": draft, "template": "research" })).send().await;
    assert_eq!(s, StatusCode::CREATED, "{team}");
    let analyst = member_named(&team, "analyst");
    let k = kickoff_of(&h, &draft, &analyst);
    assert!(k.contains("not ready yet") && k.contains("the team's voice to the orchestrator"), "{k}");

    let id = ready_task(&h, "Which CSV library?");
    let (s, team, _) = call(r, "POST", "/api/teams").json(json!({ "task": id, "template": "spike" })).send().await;
    assert_eq!(s, StatusCode::CREATED, "{team}");
    let researcher = member_named(&team, "researcher");
    let reviewer = member_named(&team, "reviewer");
    let k = kickoff_of(&h, &id, &researcher);
    assert!(k.contains("Start now.") && k.contains("move the task to review"), "{k}");
    assert!(!k.contains("executor"), "no instructions about a member the team does not have: {k}");
    let tok = token(&h, Role::Analyst, "researcher", &researcher, &id);
    for status in ["in_progress", "review"] {
        let (s, t, _) =
            call(&h.remote, "POST", &format!("/api/tasks/{id}/status")).bearer(&tok).json(json!({ "status": status })).send().await;
        assert_eq!(s, StatusCode::OK, "the researcher submits its findings: {t}");
    }
    genie::engine::tick(&h.app).unwrap();
    let mail = h.app.with_tracker("shop", |t| t.bus().history(&id, 100)).unwrap();
    assert!(mail.iter().any(|m| m.to == reviewer && m.kind == "system" && m.text.contains("hands over to you")));
}

#[tokio::test]
async fn templates_and_roles_can_be_limited_to_projects() {
    let h = Harness::new();
    h.project("shop");
    std::fs::create_dir_all(h.dir.path().join("teams")).unwrap();
    std::fs::write(
        h.dir.path().join("teams/hotfix.json"),
        r#"{"title": "Hotfix", "description": "One executor and a reviewer for urgent fixes.", "members": [{"role": "executor"}, {"role": "reviewer"}], "projects": ["other"]}"#,
    )
    .unwrap();
    let cfg = h.app.reload_agents();
    assert!(cfg.teams["hotfix"].relations_derived);
    let (_, cat, _) = call(&h.router, "GET", "/api/agent-config").send().await;
    assert!(!cat["teams"].as_array().unwrap().iter().any(|t| t["id"] == "hotfix"), "the template is for project other only");
    let (s, e, _) = call(&h.router, "POST", "/api/teams").json(json!({ "task": ready_task(&h, "x"), "template": "hotfix" })).send().await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(e["error"].as_str().unwrap().contains("not available in project shop"), "{e}");
    let (_, meta, _) = call(&h.router, "GET", "/api/meta").send().await;
    assert!(meta["roles"].as_array().unwrap().contains(&json!("researcher")));
}

#[tokio::test]
async fn a_member_added_later_gets_its_own_relations_without_duplicates() {
    let h = Harness::new();
    h.project("shop");
    let id = ready_task(&h, "Big change");
    let r = &h.router;
    // A model chosen for one member of the template (the web's «Собрать команду») keeps the template's relations.
    let (s, team, _) = call(r, "POST", "/api/teams")
        .json(json!({ "task": id, "template": "pair", "models": { "reviewer": "fake/careful" } }))
        .send()
        .await;
    assert_eq!(s, StatusCode::CREATED, "{team}");
    let reviewer = team["members"].as_array().unwrap().iter().find(|m| m["role"] == "reviewer").cloned().unwrap();
    assert_eq!(reviewer["model"], "fake/careful", "{team}");
    assert_eq!(team["spec"]["template"], "pair");
    assert!(team["spec"]["relations"].as_array().unwrap().iter().any(|r| r["note"].is_string()), "the template's own relations: {team}");
    let (s, added, _) = call(r, "POST", &format!("/api/teams/{id}/members")).json(json!({ "role": "reviewer" })).send().await;
    assert_eq!(s, StatusCode::CREATED, "{added}");
    assert_eq!(added[0]["key"], "reviewer-2");
    let (_, team, _) = call(r, "GET", &format!("/api/teams/{id}")).send().await;
    let review_handoffs: Vec<&Value> =
        team["spec"]["relations"].as_array().unwrap().iter().filter(|r| r["type"] == "handoff" && r["on"] == "review").collect();
    assert_eq!(review_handoffs.len(), 2, "{review_handoffs:?}");
    assert!(review_handoffs.iter().any(|r| r["to"] == json!(["reviewer-2"])), "the newcomer gets its own edge");
    let (exec, second) = (member_named(&team, "executor"), added[0]["name"].as_str().unwrap().to_string());
    let k = kickoff_of(&h, &id, &second);
    assert!(k.starts_with("You are joining team") && k.contains("genie tells you when the task moves to review"), "{k}");
    let tok = token(&h, Role::Executor, "executor", &exec, &id);
    for status in ["in_progress", "review"] {
        let (s, t, _) =
            call(&h.remote, "POST", &format!("/api/tasks/{id}/status")).bearer(&tok).json(json!({ "status": status })).send().await;
        assert_eq!(s, StatusCode::OK, "{t}");
    }
    genie::engine::tick(&h.app).unwrap();
    let mail = h.app.with_tracker("shop", |t| t.bus().history(&id, 200)).unwrap();
    for m in team["members"].as_array().unwrap().iter().filter(|m| m["role"] == "reviewer") {
        let name = m["name"].as_str().unwrap();
        let n = mail.iter().filter(|x| x.to == name && x.kind == "system" && x.text.contains("hands over to you")).count();
        assert_eq!(n, 1, "{name} is told once");
    }
}

const RELAY: &str = r#"{
  "title": "Relay",
  "description": "Members write only along the route.",
  "stage": "delivery",
  "mail": "flow",
  "members": [{ "role": "executor" }, { "role": "reviewer" }, { "role": "tester" }],
  "relations": [
    { "from": "executor", "to": ["reviewer"], "type": "handoff", "on": "review" },
    { "from": "reviewer", "to": ["executor"], "type": "returns", "on": "changes_requested" },
    { "from": "tester", "to": ["executor"], "type": "consults" },
    { "from": "reviewer", "to": ["orchestrator"], "type": "reports", "note": "the verdict" }
  ]
}"#;

async fn mail(h: &Harness, team: &str, token: &str, to: &str, intent: Option<&str>) -> (StatusCode, Value) {
    let (s, v, _) = call(&h.remote, "POST", &format!("/api/teams/{team}/mail"))
        .bearer(token)
        .json(json!({ "to": to, "text": "hello", "intent": intent }))
        .send()
        .await;
    (s, v)
}

#[tokio::test]
async fn in_a_flow_team_members_write_only_along_the_route() {
    let h = Harness::new();
    h.project("shop");
    std::fs::create_dir_all(h.dir.path().join("teams")).unwrap();
    std::fs::write(h.dir.path().join("teams/relay.json"), RELAY).unwrap();
    h.app.reload_agents();
    let id = ready_task(&h, "Strict route");
    let (s, team, _) = call(&h.router, "POST", "/api/teams").json(json!({ "task": id, "template": "relay" })).send().await;
    assert_eq!(s, StatusCode::CREATED, "{team}");
    let (exec, rev, tester) = (member_named(&team, "executor"), member_named(&team, "reviewer"), member_named(&team, "tester"));
    let k = kickoff_of(&h, &id, &exec);
    assert!(k.contains(&format!("you may write to {rev}, and to the orchestrator only questions and blockers")), "{k}");
    assert!(kickoff_of(&h, &id, &rev).contains(&format!("you may write to {exec} and the orchestrator")), "the voice may report");
    let e = token(&h, Role::Executor, "executor", &exec, &id);
    let r = token(&h, Role::Reviewer, "reviewer", &rev, &id);
    let t = token(&h, Role::Tester, "tester", &tester, &id);

    // Along the route.
    assert_eq!(mail(&h, &id, &e, &rev, None).await.0, StatusCode::CREATED, "handoff: executor → reviewer");
    assert_eq!(mail(&h, &id, &r, &exec, None).await.0, StatusCode::CREATED, "returns: reviewer → executor");
    let (s, asked) = mail(&h, &id, &t, &exec, Some("question")).await;
    assert_eq!(s, StatusCode::CREATED, "consults: tester → executor");
    assert_eq!(mail(&h, &id, &r, "orchestrator", Some("verdict")).await.0, StatusCode::CREATED, "the voice reports");
    assert_eq!(mail(&h, &id, &e, "orchestrator", Some("blocker")).await.0, StatusCode::CREATED, "a blocker goes to the orchestrator");

    // Off the route: refused, with the route in the answer.
    let (s, err) = mail(&h, &id, &e, &tester, None).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let msg = err["error"].as_str().unwrap();
    assert!(msg.contains(&format!("{tester} is not on your route")) && msg.contains(&format!("You may write to {rev}")), "{msg}");
    let (s, err) = mail(&h, &id, &e, "orchestrator", Some("done")).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(err["error"].as_str().unwrap().contains(&format!("{rev} report to the orchestrator")), "{err}");
    let (s, err) = mail(&h, &id, &t, "all", None).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(err["error"].as_str().unwrap().contains("no mail to everyone"), "{err}");
    let (s, err, _) = call(&h.remote, "POST", "/api/agent/ask")
        .bearer(&e)
        .json(json!({ "to": tester, "text": "which data?", "timeout": 1 }))
        .send()
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{err}");

    // Answers always go back; the orchestrator and people write to anyone.
    let (s, v, _) =
        call(&h.remote, "POST", "/api/agent/reply").bearer(&e).json(json!({ "id": asked[0]["id"], "text": "CSV" })).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v, _) =
        call(&h.router, "POST", &format!("/api/teams/{id}/mail")).json(json!({ "to": tester, "text": "look at the export" })).send().await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
}

#[tokio::test]
async fn a_team_notices_its_template_changed_after_it_started() {
    let h = Harness::new();
    h.project("shop");
    let id = ready_task(&h, "Snapshot");
    let r = &h.router;
    let (s, team, _) = call(r, "POST", "/api/teams").json(json!({ "task": id, "template": "pair" })).send().await;
    assert_eq!(s, StatusCode::CREATED, "{team}");
    let (_, v, _) = call(r, "GET", &format!("/api/teams/{id}")).send().await;
    assert_eq!(v["templateChanged"], false, "{v}");
    // A member added later changes the team, not its template.
    let (s, _, _) = call(r, "POST", &format!("/api/teams/{id}/members")).json(json!({ "role": "tester" })).send().await;
    assert_eq!(s, StatusCode::CREATED);
    let (_, v, _) = call(r, "GET", &format!("/api/teams/{id}")).send().await;
    assert_eq!(v["templateChanged"], false);
    // The template changes: the team keeps its snapshot and shows the change.
    let (_, tpl, _) = call(r, "GET", "/api/templates/pair").send().await;
    let mut edited: Value = serde_json::from_str(tpl["builtin"].as_str().unwrap()).unwrap();
    edited["charter"] = json!("Small commits.");
    let (s, v, _) = call(r, "PUT", "/api/templates/pair").json(json!({ "template": edited, "baseHash": "" })).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, v, _) = call(r, "GET", &format!("/api/teams/{id}")).send().await;
    assert_eq!(v["templateChanged"], true);
    assert!(v["spec"]["charter"].is_null(), "the running team keeps its snapshot");
}

#[tokio::test]
async fn admins_upload_the_files_of_a_skill_everyone_reads_them() {
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(outside.path().join("shared-skill")).unwrap();
    std::fs::write(outside.path().join("shared-skill/SKILL.md"), "---\nname: shared-skill\ndescription: From a clone.\n---\nx\n").unwrap();
    let path = outside.path().to_path_buf();
    let h = Harness::with_config(move |c| c.skills.paths = vec![path]);
    h.project("shop");
    let (admin, member) = h
        .app
        .with_server(|db| {
            let a = db.create_user("root", "Root", None, Some("password1"), true)?;
            let m = db.create_user("pm", "PM", None, Some("password1"), false)?;
            db.set_membership("shop", m.id, ProjectRole::Member)?;
            Ok((db.create_user_token(a.id, "t")?, db.create_user_token(m.id, "t")?))
        })
        .unwrap();
    let r = &h.router;
    let skill = "---\nname: owasp\ndescription: OWASP checks.\n---\nRun `scripts/scan.sh`.\n";
    let (s, v, _) = call(r, "PUT", "/api/skills/owasp").bearer(&admin).json(json!({ "content": skill, "baseHash": "" })).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // A script in a folder, and a binary file.
    let (s, v, _) = call(r, "PUT", "/api/skills/owasp/files/scripts/scan.sh").bearer(&admin).bytes(b"#!/bin/sh\necho scan\n").send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["path"], "skills/owasp/scripts/scan.sh");
    assert_eq!(std::fs::read_to_string(h.dir.path().join("skills/owasp/scripts/scan.sh")).unwrap(), "#!/bin/sh\necho scan\n");
    let (s, _, _) =
        call(r, "PUT", "/api/skills/owasp/files/logo.png").bearer(&admin).bytes(&[0x89, b'P', b'N', b'G', 0, 0xff]).send().await;
    assert_eq!(s, StatusCode::OK);

    // Everyone with access sees the files; text as text, binary by its size.
    let (_, v, _) = call(r, "GET", "/api/skills/owasp").bearer(&member).send().await;
    assert_eq!(v["files"], json!(["SKILL.md", "logo.png", "scripts/scan.sh"]));
    let (s, v, _) = call(r, "GET", "/api/skills/owasp/files/scripts/scan.sh").bearer(&member).send().await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["text"], "#!/bin/sh\necho scan\n");
    let (_, v, _) = call(r, "GET", "/api/skills/owasp/files/logo.png").bearer(&member).send().await;
    assert_eq!((v["size"].clone(), v["text"].clone()), (json!(6), Value::Null));

    // Only administrators write; paths stay inside the skill; SKILL.md is the skill itself.
    let (s, _, _) = call(r, "PUT", "/api/skills/owasp/files/x.txt").bearer(&member).bytes(b"x").send().await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    for bad in ["../evil.txt", "scripts/../../evil.txt", "a//b.txt", "./x.txt"] {
        let (s, _, _) = call(r, "PUT", &format!("/api/skills/owasp/files/{bad}")).bearer(&admin).bytes(b"x").send().await;
        assert!(s == StatusCode::BAD_REQUEST || s == StatusCode::NOT_FOUND, "{bad}: {s}");
    }
    assert!(!h.dir.path().join("skills/evil.txt").exists() && !h.dir.path().join("evil.txt").exists());
    let (s, e, _) = call(r, "PUT", "/api/skills/owasp/files/SKILL.md").bearer(&admin).bytes(b"x").send().await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{e}");
    let (s, e, _) = call(r, "PUT", "/api/skills/shared-skill/files/x.txt").bearer(&admin).bytes(b"x").send().await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "a skill from skills.paths is changed where it lives: {e}");
    let big = vec![b'x'; 5 * 1024 * 1024 + 1];
    let (s, _, _) = call(r, "PUT", "/api/skills/owasp/files/big.txt").bearer(&admin).bytes(&big).send().await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);

    // A removed file takes its emptied folder along; the history keeps who did what.
    let (s, _, _) = call(r, "DELETE", "/api/skills/owasp/files/scripts/scan.sh").bearer(&admin).send().await;
    assert_eq!(s, StatusCode::OK);
    assert!(!h.dir.path().join("skills/owasp/scripts").exists());
    let (s, _, _) = call(r, "DELETE", "/api/skills/owasp/files/scripts/scan.sh").bearer(&admin).send().await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, hist, _) = call(r, "GET", "/api/agent-config/history?item=skill:owasp").bearer(&admin).send().await;
    let rows: Vec<(String, Value, Value)> = hist
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["path"].as_str().unwrap().to_string(), c["before"].clone(), c["after"].clone()))
        .collect();
    assert_eq!(rows[0], ("skills/owasp/scripts/scan.sh".into(), json!("#!/bin/sh\necho scan\n"), Value::Null));
    assert_eq!(rows[1], ("skills/owasp/logo.png".into(), Value::Null, json!("[6 bytes]")));
    assert_eq!(rows.len(), 4, "SKILL.md, two uploads, one removal: {hist}");
}

#[tokio::test]
async fn a_person_gives_one_agent_its_own_model_and_takes_it_back() {
    let h = Harness::new();
    h.project("shop");
    let id = ready_task(&h, "Pick a model");
    let r = &h.router;
    let (s, team, _) = call(r, "POST", "/api/teams").json(json!({ "task": id, "template": "pair" })).send().await;
    assert_eq!(s, StatusCode::CREATED, "{team}");
    let (exec, reviewer) = (member_named(&team, "executor"), member_named(&team, "reviewer"));

    // The menu lists the models the configuration names, and the thinking levels.
    let (s, models, _) = call(r, "GET", "/api/models").send().await;
    assert_eq!(s, StatusCode::OK, "{models}");
    let ids: Vec<&str> = models["models"].as_array().unwrap().iter().filter_map(|m| m["id"].as_str()).collect();
    assert!(ids.contains(&"openai-codex/gpt-6-luna"), "the roleModels of the configuration: {models}");
    assert!(models["thinking"].as_array().unwrap().contains(&json!("xhigh")));

    // One member switches; the rest of the team keeps its role's model.
    let url = format!("/api/teams/{id}/members/{exec}");
    let (s, v, _) = call(r, "PATCH", &url).json(json!({ "model": "fake/strong", "thinking": "xhigh" })).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, team, _) = call(r, "GET", &format!("/api/teams/{id}")).send().await;
    let member = |team: &Value, name: &str| team["members"].as_array().unwrap().iter().find(|m| m["name"] == name).cloned().unwrap();
    assert_eq!((member(&team, &exec)["model"].clone(), member(&team, &exec)["thinking"].clone()), (json!("fake/strong"), json!("xhigh")));
    assert!(member(&team, &reviewer)["model"].is_null(), "{team}");
    assert!(team["log"].as_array().unwrap().iter().any(|e| e["event"] == "member_model" && e["member"] == exec.as_str()), "{team}");

    // Nonsense is refused; a teammate may not switch another agent's model.
    let (s, _, _) = call(r, "PATCH", &url).json(json!({ "thinking": "extreme" })).send().await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = call(r, "PATCH", &url).json(json!({ "model": "fake strong" })).send().await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let tok = token(&h, Role::Reviewer, "reviewer", &reviewer, &id);
    let (s, _, _) = call(&h.remote, "PATCH", &url).bearer(&tok).json(json!({ "model": "fake/cheap" })).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, e, _) = call(r, "PATCH", &format!("/api/teams/{id}/members/nobody")).json(json!({ "model": "fake/cheap" })).send().await;
    assert!(s.is_client_error() && e["error"].as_str().unwrap().contains("has no member nobody"), "{e}");

    // Empty values take the member back to its role.
    let (s, _, _) = call(r, "PATCH", &url).json(json!({ "model": null, "thinking": "" })).send().await;
    assert_eq!(s, StatusCode::OK);
    let (_, team, _) = call(r, "GET", &format!("/api/teams/{id}")).send().await;
    assert!(member(&team, &exec)["model"].is_null() && member(&team, &exec)["thinking"].is_null(), "{team}");
}
