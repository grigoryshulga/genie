//! Jev's real HTTP/wire boundary is exercised against a local fake, without paid calls.
use crate::common::{Harness, call};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use genie::decisions::{Mode, Policy, Profile};
use genie_core::{Actor, CreateInput, Role, Status};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct FakeState {
    calls: AtomicUsize,
    expected_key: Mutex<Option<String>>,
    requests: Mutex<Vec<Value>>,
    status: Mutex<Option<StatusCode>>,
    flat: Mutex<bool>,
    malformed: Mutex<bool>,
    hook: Mutex<Option<Box<dyn Fn() + Send>>>,
}
async fn fake(State(s): State<Arc<FakeState>>, headers: HeaderMap, Json(request): Json<Value>) -> axum::response::Response {
    let expected = s.expected_key.lock().unwrap();
    assert_eq!(headers["authorization"], format!("Bearer {}", expected.as_deref().unwrap_or("test-typesafe-key")));
    drop(expected);
    s.calls.fetch_add(1, Ordering::SeqCst);
    s.requests.lock().unwrap().push(request.clone());
    if let Some(hook) = s.hook.lock().unwrap().take() {
        hook();
    }
    if let Some(status) = *s.status.lock().unwrap() {
        return (status, Json(json!({"error":"try later"}))).into_response();
    }
    let mut answers = serde_json::Map::new();
    for (id, q) in request["questions"].as_object().unwrap() {
        let answer = match q["type"].as_str().unwrap() {
            "choice" => {
                let options = q["criteria"].as_object().unwrap();
                let selected =
                    options.keys().find(|k| *k != "need_evidence" && *k != "proposed").unwrap_or_else(|| options.keys().next().unwrap());
                let probabilities: serde_json::Map<String, Value> = options
                    .keys()
                    .map(|k| {
                        (
                            k.clone(),
                            json!(if *s.flat.lock().unwrap() {
                                1.0 / options.len() as f64
                            } else if k == selected {
                                1.0
                            } else {
                                0.0
                            }),
                        )
                    })
                    .collect();
                json!({"type":"choice","choice":if *s.malformed.lock().unwrap() {"invented"}else{selected},"confidence":1.0,"probabilities":probabilities})
            }
            "score" => json!({"type":"score","score":1.0,"confidence":0.9,"probabilities":{"0":0.0,"1":1.0,"2":0.0}}),
            "noul" => json!({"type":"noul","noul":0.9}),
            other => panic!("unexpected wire type {other}"),
        };
        answers.insert(id.clone(), answer);
    }
    Json(json!({"model":"jev-1.13.0","answers":answers,"usage":{"input_tokens":100,"output_tokens":20}})).into_response()
}
async fn harness() -> (Harness, Arc<FakeState>) {
    connection_harness(|_| {}).await
}
async fn connection_harness(configure: impl FnOnce(&mut genie::decisions::Config)) -> (Harness, Arc<FakeState>) {
    let state = Arc::new(FakeState::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1/", listener.local_addr().unwrap());
    let router = Router::new()
        .route("/v1/systemone", post(fake))
        .route("/api/v1/systemone", post(fake))
        .route("/custom/classify", post(fake))
        .with_state(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let h = Harness::with_config(|c| {
        c.decisions.base_url = url;
        configure(&mut c.decisions);
    });
    h.project("shop");
    h.app.with_server(|db| db.set_global_secret("typesafe", "test-typesafe-key")).unwrap();
    (h, state)
}
fn ready(h: &Harness) -> String {
    h.app
        .with_tracker("shop", |t| {
            let actor = Actor::new("anna", Role::Human);
            let task = t.create(
                &actor,
                CreateInput {
                    title: "Export orders".into(),
                    description: Some("Add CSV export".into()),
                    acceptance: vec!["CSV downloads".into()],
                    status: Some(Status::Ready),
                    merge_strategy: Some("deliver artifact".into()),
                    ..Default::default()
                },
            )?;
            t.set_status(&actor, &task.id, Status::Ready, genie_core::StatusOptions { force: true, ..Default::default() })?;
            Ok(task.id)
        })
        .unwrap()
}
fn save(h: &Harness, p: Policy) {
    genie::decisions::validate_policy(&h.app, "shop", &p).unwrap();
    h.app.with_server(|db| db.set_decision_policy("shop", "anna", &json!(p))).unwrap();
}
fn enabled() -> Policy {
    Policy { allow_external: true, tool_enabled: true, ..Default::default() }
}
fn request(task: &str) -> Value {
    json!({"task":task,"purpose":"approach","state":{"goal":"test"},"questions":{
        "approach":{"type":"choice","instructions":"Choose next step","criteria":{"a":"Inspect evidence","b":"Experiment","need_evidence":"Need more facts"}}
    }})
}
fn spawn(task: &str) -> Value {
    json!({"task":task,"template":"standard"})
}
fn profile_policy(mode: Mode) -> Policy {
    Policy {
        team: mode,
        model: mode,
        thinking: mode,
        templates: vec!["pair".into()],
        profiles: vec![Profile {
            id: "careful".into(),
            description: "Tool-capable model checked for independent implementation and review".into(),
            model: "fake/careful".into(),
            thinking: Some("high".into()),
            roles: vec!["executor".into(), "reviewer".into()],
        }],
        ..enabled()
    }
}
fn token(h: &Harness, role: Role, role_id: &str, team: Option<&str>) -> String {
    h.app.with_server(|db| db.create_role_token("shop", role, Some(role_id), "agent", team, None, chrono::Duration::hours(1))).unwrap()
}

#[tokio::test]
async fn disabled_by_default_and_only_project_admins_can_enable() {
    let (h, s) = harness().await;
    let task = ready(&h);
    let (status, _, _) = call(&h.router, "POST", "/api/decisions/evaluate").json(request(&task)).send().await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let user = h.app.with_server(|db| db.create_user("member", "Member", None, Some("password-1"), false)).unwrap();
    h.app.with_server(|db| db.set_membership("shop", user.id, genie_core::server_db::ProjectRole::Member)).unwrap();
    let key = h.app.with_server(|db| db.create_user_token(user.id, "test")).unwrap();
    let (status, _, _) = call(&h.router, "PUT", "/api/decisions/policy").bearer(&key).json(json!(enabled())).send().await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn explicit_tool_preserves_typed_answers_usage_and_durable_policy() {
    let (h, s) = harness().await;
    let task = ready(&h);
    let (status, _, _) = call(&h.router, "PUT", "/api/decisions/policy").json(json!(enabled())).send().await;
    assert_eq!(status, StatusCode::OK);
    let mut r = request(&task);
    r["questions"]["enough"] = json!({"type":"bool","instructions":"Enough facts?","criteria":{"true":"Enough","false":"Missing"}});
    r["questions"]["risk"] = json!({"type":"score","instructions":"Rate risk","criteria":["low","medium","high"]});
    let (status, v, _) = call(&h.router, "POST", "/api/decisions/evaluate").json(r).send().await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "ok");
    assert_eq!(v["result"]["answers"]["enough"]["probability"], 0.9);
    assert_eq!(v["result"]["answers"]["risk"]["score"], 1.0);
    assert!((v["costUsd"].as_f64().unwrap() - 0.0000042).abs() < 1e-10);
    assert_eq!(s.requests.lock().unwrap()[0]["questions"]["enough"]["type"], "noul");
    let reopened = genie::state::App::open(&h.app.data, h.app.cfg.clone(), None).unwrap();
    assert!(genie::decisions::policy(&reopened, "shop").unwrap().tool_enabled);
    assert_eq!(reopened.with_server(|db| db.decisions("shop", Some(&task), 100)).unwrap().len(), 1);
    let rows = reopened.with_tracker("shop", |t| t.usage_of_task(&task)).unwrap();
    assert_eq!(rows[0].tokens.input, 100);
    let spend = genie::spend::Spend::of(&reopened.prices().unwrap(), &rows);
    assert_eq!(spend.unpriced_tokens, 0);
    assert_eq!(spend.models[0].price.unwrap().input, 0.042);
}

#[tokio::test]
async fn provider_failures_are_bounded_not_retried_and_keep_their_reservation() {
    let (h, s) = harness().await;
    let task = ready(&h);
    save(&h, Policy { max_calls_per_task: 1, ..enabled() });
    *s.status.lock().unwrap() = Some(StatusCode::TOO_MANY_REQUESTS);
    let (status, v, _) = call(&h.router, "POST", "/api/decisions/evaluate").json(request(&task)).send().await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["status"], "unavailable");
    assert!(v["costUsd"].is_null());
    assert!(v["reservedUsd"].as_f64().unwrap() > 0.0);
    assert_eq!(v["result"]["error"], "Jev HTTP 429");
    let (status, _, _) = call(&h.router, "POST", "/api/decisions/evaluate").json(request(&task)).send().await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn malformed_billed_answers_are_rejected_and_flat_probabilities_cannot_fake_confidence() {
    let (h, s) = harness().await;
    let task = ready(&h);
    save(&h, enabled());
    *s.malformed.lock().unwrap() = true;
    let (_, v, _) = call(&h.router, "POST", "/api/decisions/evaluate").json(request(&task)).send().await;
    assert_eq!(v["status"], "unavailable");
    assert!(v["costUsd"].as_f64().unwrap() > 0.0);
    *s.malformed.lock().unwrap() = false;
    *s.flat.lock().unwrap() = true;
    let (_, v, _) = call(&h.router, "POST", "/api/decisions/evaluate").json(request(&task)).send().await;
    assert_eq!(v["status"], "uncertain");
    assert_eq!(v["result"]["answers"]["approach"]["qualified"], false);
}

#[tokio::test]
async fn tools_enforce_role_grants_and_task_scope_even_for_history() {
    let (h, s) = harness().await;
    let task = ready(&h);
    let other = ready(&h);
    save(&h, enabled());
    h.app
        .with_tracker("shop", |t| {
            t.bus().create(
                "anna",
                "human",
                genie_core::team::NewTeam {
                    id: "scope".into(),
                    task: task.clone(),
                    cwd: "/tmp".into(),
                    members: vec![genie_core::team::NewMember { name: "agent".into(), role: "analyst".into(), ..Default::default() }],
                    ..Default::default()
                },
            )
        })
        .unwrap();
    let analyst = token(&h, Role::Analyst, "analyst", Some("scope"));
    let reviewer = token(&h, Role::Reviewer, "reviewer", Some("scope"));
    let (status, _, _) = call(&h.router, "POST", "/api/decisions/evaluate").bearer(&reviewer).json(request(&task)).send().await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = call(&h.router, "POST", "/api/decisions/evaluate").bearer(&analyst).json(request(&other)).send().await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (_, record, _) = call(&h.router, "POST", "/api/decisions/evaluate").json(request(&other)).send().await;
    let (status, _, _) = call(&h.router, "GET", &format!("/api/decisions/{}", record["id"])).bearer(&analyst).send().await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = call(&h.router, "GET", "/api/decisions").bearer(&analyst).send().await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = call(&h.router, "POST", "/api/decisions/evaluate").bearer(&analyst).json(request(&task)).send().await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(s.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn select_applies_a_curated_team_and_compatible_pairs_but_pins_human_choices() {
    let (h, s) = harness().await;
    let task = ready(&h);
    save(&h, profile_policy(Mode::Select));
    let orchestrator = token(&h, Role::Orchestrator, "orchestrator", None);
    let (status, v, _) = call(&h.router, "POST", "/api/teams").bearer(&orchestrator).json(spawn(&task)).send().await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["template"], "pair");
    assert_eq!(v["members"].as_array().unwrap().len(), 2);
    for member in v["members"].as_array().unwrap() {
        assert_eq!(member["model"], "fake/careful");
        assert_eq!(member["thinking"], "high");
    }
    let task = ready(&h);
    let mut bypass = spawn(&task);
    bypass["decisionOverride"] = json!("I prefer the original team despite automatic selection.");
    let (status, _, _) = call(&h.router, "POST", "/api/teams").bearer(&orchestrator).json(bypass).send().await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "select mode must not be bypassed by an agent");
    let (status, v, _) = call(&h.router, "POST", "/api/teams").json(spawn(&task)).send().await;
    // An explicit person's choice remains pinned.
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["template"], "standard");
    assert_eq!(s.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn joint_requires_reconsideration_and_a_specific_override_reuses_evidence() {
    let (h, s) = harness().await;
    let task = ready(&h);
    save(&h, Policy { team: Mode::Joint, templates: vec!["pair".into()], ..enabled() });
    let orchestrator = token(&h, Role::Orchestrator, "orchestrator", None);
    let (status, v, _) = call(&h.router, "POST", "/api/teams").bearer(&orchestrator).json(spawn(&task)).send().await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    assert!(h.app.with_tracker("shop", |t| t.bus().active_team_of(&task)).unwrap().is_none());
    let mut body = spawn(&task);
    body["decisionOverride"] = json!("The documentation criterion requires a dedicated documenter.");
    let (status, v, _) = call(&h.router, "POST", "/api/teams").bearer(&orchestrator).json(body).send().await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["template"], "standard");
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    let records = h.app.with_server(|db| db.decisions("shop", Some(&task), 10)).unwrap();
    assert!(records[0].application["override"].as_str().unwrap().contains("documenter"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn observe_keeps_the_roster_and_does_not_deliver_the_judgement() {
    let (h, _) = harness().await;
    let task = ready(&h);
    save(&h, profile_policy(Mode::Observe));
    let orchestrator = token(&h, Role::Orchestrator, "orchestrator", None);
    let (status, v, _) = call(&h.router, "POST", "/api/teams").bearer(&orchestrator).json(spawn(&task)).send().await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["template"], "standard");
    assert!(v["members"].as_array().unwrap().iter().all(|m| m["model"].is_null()));
    let mail = h.app.with_tracker("shop", |t| t.bus().pending(None, "orchestrator")).unwrap();
    assert!(mail.iter().all(|m| m.kind != "decision"));
    assert_eq!(h.app.with_server(|db| db.decisions("shop", Some(&task), 100)).unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recommendation_previews_advice_without_creating_a_workspace_or_team() {
    let (h, _) = harness().await;
    let task = ready(&h);
    save(&h, profile_policy(Mode::Advise));
    let (status, v, _) = call(&h.router, "POST", "/api/teams/recommend").json(spawn(&task)).send().await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["plan"]["template"], "pair");
    assert_eq!(v["plan"]["members"][0]["model"], "fake/careful");
    assert!(h.app.with_tracker("shop", |t| t.bus().list(false)).unwrap().is_empty());
    assert_eq!(h.app.with_tracker("shop", |t| Ok(t.get(&task)?.status)).unwrap(), Status::Ready);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_facts_are_refused_and_concurrent_spawns_do_not_duplicate_calls_or_teams() {
    let (h, s) = harness().await;
    let task = ready(&h);
    save(&h, Policy { team: Mode::Select, templates: vec!["pair".into()], ..enabled() });
    let app = h.app.clone();
    let changed = task.clone();
    *s.hook.lock().unwrap() = Some(Box::new(move || {
        app.with_tracker("shop", |t| {
            t.update(
                &Actor::new("anna", Role::Human),
                &changed,
                genie_core::UpdateInput { description: Some("Requirements changed while evaluating".into()), ..Default::default() },
            )
        })
        .unwrap();
    }));
    let orchestrator = token(&h, Role::Orchestrator, "orchestrator", None);
    let (status, v, _) = call(&h.router, "POST", "/api/teams").bearer(&orchestrator).json(spawn(&task)).send().await;
    assert_eq!(status, StatusCode::CONFLICT, "{v}");
    let a = call(&h.router, "POST", "/api/teams").bearer(&orchestrator).json(spawn(&task));
    let b = call(&h.router, "POST", "/api/teams").bearer(&orchestrator).json(spawn(&task));
    let (a, b) = tokio::join!(a.send(), b.send());
    assert_eq!([a.0, b.0].iter().filter(|s| **s == StatusCode::CREATED).count(), 1, "{a:?} {b:?}");
    assert_eq!(h.app.with_tracker("shop", |t| t.bus().list(false)).unwrap().len(), 1);
    assert_eq!(s.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn invalid_policies_and_oversized_inputs_do_not_call_the_provider() {
    let (h, s) = harness().await;
    let task = ready(&h);
    let mut p = enabled();
    p.templates = vec!["research".into()];
    let (status, _, _) = call(&h.router, "PUT", "/api/decisions/policy").json(json!(p)).send().await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    save(&h, enabled());
    let mut r = request(&task);
    r["state"] = json!("x".repeat(40_000));
    let (status, _, _) = call(&h.router, "POST", "/api/decisions/evaluate").json(r).send().await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cli_and_mcp_share_the_tool_and_infer_the_actual_assigned_task() {
    use genie::ops::{self, Auth, Cx, InProcess};
    let (h, s) = harness().await;
    let task = ready(&h);
    save(&h, enabled());
    let cx = Cx {
        api: Box::new(InProcess::new(genie::http::router(h.app.clone()), h.app.cfg.port, Auth::Operator, Some("shop".into()))),
        project: Some("shop".into()),
        task: Some(task.clone()),
        team: None,
        local: true,
    };
    let mut r = request(&task);
    r.as_object_mut().unwrap().remove("task");
    let input = r.to_string();
    let matches = genie::cli::command().try_get_matches_from(["genie", "decision", "evaluate", "--request", &input]).unwrap();
    let (entry, args) = ops::chosen(&matches).unwrap();
    let out = entry.run_cli(args, &cx).await.unwrap();
    assert_eq!(out.data["task"], task);
    h.app
        .with_tracker("shop", |t| {
            t.bus().create(
                "anna",
                "human",
                genie_core::team::NewTeam {
                    id: "custom-team-name".into(),
                    task: task.clone(),
                    cwd: "/tmp".into(),
                    members: vec![genie_core::team::NewMember { name: "agent".into(), role: "analyst".into(), ..Default::default() }],
                    ..Default::default()
                },
            )
        })
        .unwrap();
    let analyst = token(&h, Role::Analyst, "analyst", Some("custom-team-name"));
    let (status, v, _) = call(&h.remote, "POST", "/mcp")
        .bearer(&analyst)
        .no_csrf()
        .json(json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"genie_decision","arguments":{"action":"evaluate","request":r}}
        }))
        .send()
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_ne!(v["result"]["isError"], true, "{v}");
    let result: Value = serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(result["task"], task);
    assert_eq!(s.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn explicit_member_thinking_can_be_copied_from_a_recommendation() {
    use genie::ops::{self, Auth, Cx, InProcess};
    let (h, s) = harness().await;
    let task = ready(&h);
    let cx = Cx {
        api: Box::new(InProcess::new(genie::http::router(h.app.clone()), h.app.cfg.port, Auth::Operator, Some("shop".into()))),
        project: Some("shop".into()),
        task: None,
        team: None,
        local: true,
    };
    let matches =
        genie::cli::command().try_get_matches_from(["genie", "team", "spawn", &task, "--member", "executor:fake/careful:high"]).unwrap();
    let (entry, args) = ops::chosen(&matches).unwrap();
    let out = entry.run_cli(args, &cx).await.unwrap();
    assert_eq!(out.data["members"][0]["model"], "fake/careful");
    assert_eq!(out.data["members"][0]["thinking"], "high");
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn fixing_a_missing_key_and_recovering_an_interrupted_call_do_not_leave_permanent_cache_failures() {
    use genie_core::decisions::NewDecision;
    let (h, s) = harness().await;
    let task = ready(&h);
    save(&h, enabled());
    let user = h.app.with_server(|db| db.create_user("anna", "Anna", None, Some("password-1"), false)).unwrap();
    h.app.with_server(|db| db.delete_global_secret("typesafe")).unwrap();
    let actor = Actor::new("anna", Role::Human);
    let request: genie::decisions::Request = serde_json::from_value(request(&task)).unwrap();
    let first = genie::decisions::evaluate(&h.app, "shop", &actor, Some("anna"), &request, Some("repaired-key")).await.unwrap();
    assert_eq!(first.status, "unavailable");
    assert_eq!(first.cost_usd, Some(0.0));
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    h.app.with_server(|db| db.set_user_secret(user.id, "typesafe", "test-typesafe-key")).unwrap();
    let second = genie::decisions::evaluate(&h.app, "shop", &actor, Some("anna"), &request, Some("repaired-key")).await.unwrap();
    assert_eq!(second.status, "ok");
    assert_ne!(second.id, first.id);
    let cached = genie::decisions::evaluate(&h.app, "shop", &actor, Some("anna"), &request, Some("repaired-key")).await.unwrap();
    assert_eq!(cached.id, second.id);
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    let pending = h
        .app
        .with_server(|db| {
            let record = db.begin_decision(NewDecision {
                project: "shop",
                task: &task,
                actor: "anna",
                purpose: "interrupted",
                request: &json!({}),
                key: Some("interrupted"),
                reserve: 0.0001,
                max_calls: 40,
                max_cost: 0.1,
            })?;
            db.conn().execute("UPDATE decisions SET created='2000-01-01T00:00:00.000Z' WHERE id=?1", [record.id])?;
            Ok(record)
        })
        .unwrap();
    let recovered = genie::decisions::evaluate(&h.app, "shop", &actor, Some("anna"), &request, Some("interrupted")).await.unwrap();
    assert_eq!(recovered.status, "ok");
    let old = h.app.with_server(|db| db.decision("shop", pending.id)).unwrap();
    assert_eq!(old.status, "unavailable");
    assert_eq!(old.cost_usd, None);
    assert_eq!(s.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn automatic_unavailability_keeps_the_proposal_and_fixed_model_filters_incompatible_profiles() {
    let (h, s) = harness().await;
    let task = ready(&h);
    let mut p = profile_policy(Mode::Select);
    p.team = Mode::Off;
    p.model = Mode::Off;
    save(&h, p);
    let orchestrator = token(&h, Role::Orchestrator, "orchestrator", None);
    let (status, _, _) = call(&h.router, "POST", "/api/teams").bearer(&orchestrator).json(spawn(&task)).send().await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    let task = ready(&h);
    save(&h, Policy { team: Mode::Select, templates: vec!["pair".into()], ..enabled() });
    *s.status.lock().unwrap() = Some(StatusCode::SERVICE_UNAVAILABLE);
    let (status, v, _) = call(&h.router, "POST", "/api/teams").bearer(&orchestrator).json(spawn(&task)).send().await;
    assert_eq!(status, StatusCode::CREATED, "{v}");
    assert_eq!(v["template"], "standard");
}

#[tokio::test]
async fn openrouter_uses_its_personal_key_model_and_accounting_namespace() {
    let (h, s) = connection_harness(|c| {
        c.base_url = c.base_url.replace("/v1/", "/api/v1/");
        c.key_provider = "openrouter".into();
        c.model = "typesafe/jev-1.13".into();
    })
    .await;
    let task = ready(&h);
    save(&h, enabled());
    let user = h
        .app
        .with_server(|db| {
            let user = db.create_user("anna", "Anna", None, Some("password-1"), false)?;
            db.set_global_secret("openrouter", "shared-or-key")?;
            db.set_user_secret(user.id, "openrouter", "personal-or-key")?;
            Ok(user)
        })
        .unwrap();
    let user_token = h
        .app
        .with_server(|db| {
            db.set_membership("shop", user.id, genie_core::server_db::ProjectRole::Admin)?;
            db.create_user_token(user.id, "test")
        })
        .unwrap();
    *s.expected_key.lock().unwrap() = Some("personal-or-key".into());
    let r = serde_json::from_value(request(&task)).unwrap();
    let actor = Actor::new("anna", Role::Human);
    let personal = genie::decisions::evaluate(&h.app, "shop", &actor, Some("anna"), &r, None).await.unwrap();
    assert_eq!(personal.status, "ok");
    h.app.with_server(|db| db.delete_user_secret(user.id, "openrouter")).unwrap();
    *s.expected_key.lock().unwrap() = Some("shared-or-key".into());
    let shared = genie::decisions::evaluate(&h.app, "shop", &actor, Some("anna"), &r, None).await.unwrap();
    assert_eq!(shared.status, "ok");
    assert_eq!(s.requests.lock().unwrap()[0]["model"], "typesafe/jev-1.13");
    let rows = h.app.with_tracker("shop", |t| t.usage_of_task(&task)).unwrap();
    let spend = genie::spend::Spend::of(&h.app.prices().unwrap(), &rows);
    assert_eq!(spend.unpriced_tokens, 0);
    assert!(rows.iter().all(|row| row.model == "openrouter/typesafe/jev-1.13"));
    let (_, settings, _) = call(&h.router, "GET", "/api/decisions/policy").bearer(&user_token).send().await;
    assert_eq!(settings["keyProvider"], "openrouter");
    assert!(settings["endpoint"].as_str().unwrap().ends_with("/api/v1/systemone"));
    for key in ["personal-or-key", "shared-or-key", "test-typesafe-key"] {
        assert!(!json!([settings, json!(personal), json!(shared)]).to_string().contains(key));
    }
}

#[tokio::test]
async fn custom_endpoint_uses_only_the_explicit_server_environment_credential() {
    // Use a harmless existing variable as the test credential, without mutating
    // the process environment while concurrent Rust tests run.
    let test_key = std::env::var("HOME").unwrap();
    let (h, s) = connection_harness(|c| {
        c.endpoint = Some(format!("{}custom/classify", c.base_url.trim_end_matches("v1/")));
        c.base_url = "https://unused.invalid/".into();
        c.key_provider = "private-decisions".into();
        c.api_key_env = Some("HOME".into());
        c.model = "custom-jev".into();
    })
    .await;
    *s.expected_key.lock().unwrap() = Some(test_key);
    // Explicit service keys are intentional server grants, including with users.
    let user_token = h
        .app
        .with_server(|db| {
            let user = db.create_user("anna", "Anna", None, Some("password-1"), false)?;
            db.set_membership("shop", user.id, genie_core::server_db::ProjectRole::Admin)?;
            db.create_user_token(user.id, "test")
        })
        .unwrap();
    let task = ready(&h);
    save(&h, enabled());
    let (_, result, _) = call(&h.router, "POST", "/api/decisions/evaluate").bearer(&user_token).json(request(&task)).send().await;
    assert_eq!(result["status"], "ok", "{result}");
    assert_eq!(s.requests.lock().unwrap()[0]["model"], "custom-jev");
    let (_, settings, _) = call(&h.router, "GET", "/api/decisions/policy").bearer(&user_token).send().await;
    assert_eq!(settings["serverKey"], true);
    assert!(settings["endpoint"].as_str().unwrap().ends_with("/custom/classify"));
    assert!(settings.get("apiKeyEnv").is_none());
}

#[test]
fn connection_urls_reject_embedded_credentials_and_insecure_remote_transport() {
    for endpoint in [
        "https://user:secret@example.com/systemone",
        "https://example.com/systemone?key=secret",
        "https://example.com/systemone#secret",
        "http://example.com/systemone",
    ] {
        let c = genie::decisions::Config { endpoint: Some(endpoint.into()), ..Default::default() };
        let error = c.endpoint_url().unwrap_err();
        assert!(!error.contains("secret"));
    }
}

#[tokio::test]
async fn missing_explicit_service_key_never_falls_back_to_a_provider_key() {
    let (h, s) = connection_harness(|c| {
        c.api_key_env = Some("GENIE_JEV_TEST_DELIBERATELY_ABSENT_KEY_1789000000".into());
    })
    .await;
    let task = ready(&h);
    save(&h, enabled());
    let (_, record, _) = call(&h.router, "POST", "/api/decisions/evaluate").json(request(&task)).send().await;
    assert_eq!(record["status"], "unavailable");
    assert_eq!(record["costUsd"], 0.0);
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn task_manager_infers_its_mcp_task_and_cannot_evaluate_or_preview_another_task() {
    let (h, s) = harness().await;
    let task = ready(&h);
    let other = ready(&h);
    save(&h, enabled());
    genie::management::dispatch(&h.app, "shop", &Actor::new("anna", Role::Human), &task).unwrap();
    let manager = h
        .app
        .with_server(|db| {
            db.create_role_token(
                "shop",
                Role::Orchestrator,
                Some("orchestrator"),
                &genie_core::management::manager_name(&task),
                None,
                None,
                chrono::Duration::hours(1),
            )
        })
        .unwrap();
    let mut r = request(&task);
    r.as_object_mut().unwrap().remove("task");
    let (status, v, _) = call(&h.remote, "POST", "/mcp")
        .bearer(&manager)
        .no_csrf()
        .json(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"genie_decision","arguments":{"action":"evaluate","request":r}
        }}))
        .send()
        .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_ne!(v["result"]["isError"], true, "{v}");
    let result: Value = serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(result["task"], task);
    assert_eq!(result["status"], "ok");
    let (status, _, _) = call(&h.router, "POST", "/api/decisions/evaluate").bearer(&manager).json(request(&other)).send().await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = call(&h.router, "POST", "/api/teams/recommend").bearer(&manager).json(spawn(&other)).send().await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
}
