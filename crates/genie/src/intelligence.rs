//! Task intelligence application services. Policy-derived protections are evaluated
//! independently of Jev authority, including for explicit human dispatches.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, Instant};

use genie_core::intelligence::{
    IntelligencePolicy, ModelSelection, RoutingPlan, Signal, SignalName, SignalProfile, SignalSource, Snapshot,
};
use genie_core::{Actor, Role, Task};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::agent_config::{AgentConfig, Stage};
use crate::decisions::{self, Policy, Request, choice};
use crate::runtime::Roster;
use crate::state::{App, AppError, AppResult};
use crate::tasks::{self, Caller, Touch};

pub const EXTRACTOR: &str = "task-signals-v1";
pub fn fingerprint(value: &impl Serialize) -> String {
    genie_core::server_db::hash_secret(&serde_json::to_string(value).unwrap_or_default())
}

pub fn validate_policy(app: &App, project: &str, policy: &IntelligencePolicy) -> AppResult<()> {
    policy.validate()?;
    let agents = app.agents();
    for role in policy.checks.values() {
        let definition = agents.role_for(project, role)?;
        if definition.class != Role::Reviewer || !definition.stages.contains(&Stage::Delivery) {
            return Err(AppError::Bad("mandatory intelligence checks must name a delivery reviewer role".into()));
        }
    }
    if let Some(id) = &policy.fallback_template {
        let t = agents.team_for(project, id)?;
        if t.stage != Stage::Delivery || t.members.len() > app.cfg.limits.max_members_per_team {
            return Err(AppError::Bad("safe fallback must be a delivery template within member limits".into()));
        }
        let required = policy.required(&[]);
        if required.iter().any(|c| !t.members.iter().any(|m| policy.checks.get(c) == Some(&m.role))) {
            return Err(AppError::Bad("fallback template must satisfy every protected component check".into()));
        }
    }
    Ok(())
}

/// Bounded read-only inventory: paths and test manifests, never file content or
/// code comments. Both elapsed time and entry count are capped; symlinks are skipped.
fn inventory(root: &Path) -> Vec<String> {
    type Cache = std::collections::HashMap<std::path::PathBuf, (Instant, Vec<String>)>;
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Cache>> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Ok(entries) = cache.lock() {
        if let Some((at, facts)) = entries.get(root).filter(|(at, _)| at.elapsed() < Duration::from_secs(60)) {
            let _ = at;
            return facts.clone();
        }
    }
    let facts = scan_inventory(root);
    if let Ok(mut entries) = cache.lock() {
        entries.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(60));
        if entries.len() < 64 {
            entries.insert(root.to_path_buf(), (Instant::now(), facts.clone()));
        }
    }
    facts
}
fn scan_inventory(root: &Path) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_millis(200);
    let mut budget = 256usize;
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut facts = Vec::new();
    while let Some((dir, depth)) = pending.pop() {
        if budget == 0 || Instant::now() >= deadline {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.take(budget).flatten() {
            if budget == 0 || Instant::now() >= deadline {
                break;
            }
            budget -= 1;
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            let Ok(relative) = path.strip_prefix(root) else { continue };
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || matches!(name.as_ref(), "target" | "node_modules" | "dist") {
                continue;
            }
            if kind.is_file() && matches!(name.as_ref(), "Cargo.toml" | "package.json" | "pyproject.toml" | "Makefile" | "go.mod") {
                facts.push(format!("manifest:{}", relative.display()));
            }
            if kind.is_dir() && depth < 2 {
                pending.push((path, depth + 1));
            }
        }
    }
    facts.sort();
    facts
}

fn brief_input(task: &Task) -> Value {
    json!({"title":task.title,"description":task.description,"plan":task.plan,"acceptance":task.acceptance.iter().map(|a|&a.text).collect::<Vec<_>>(),"labels":task.labels})
}
pub(crate) fn dispatch_snapshot(app: &App, project: &str, actor: &Actor, task: &Task, policy: &Policy) -> AppResult<Snapshot> {
    let previous = app.with_tracker(project, |t| t.intelligence_history(&task.id))?;
    let fingerprint = fingerprint(&brief_input(task));
    let paths = previous
        .snapshots
        .last()
        .filter(|s| s.profile.brief_fingerprint == fingerprint)
        .map(|s| s.profile.affected_paths.clone())
        .unwrap_or_default();
    assess(app, project, actor, task, policy, paths, None)
}

pub fn assess(
    app: &App,
    project: &str,
    actor: &Actor,
    task: &Task,
    p: &Policy,
    paths: Vec<String>,
    trigger: Option<i64>,
) -> AppResult<Snapshot> {
    if paths.len() > 128 || paths.iter().any(|p| !genie_core::intelligence::valid_path(p)) {
        return Err(AppError::Bad("affected paths must be bounded relative repository paths".into()));
    }
    let brief_version = app.with_tracker(project, |t| Ok(t.management().manager(&task.id)?.map_or(0, |m| m.brief_version)))?;
    // Hash only brief facts; operational timestamps, status, acceptance checkmarks,
    // mail and token usage do not change the assessed requirements.
    let brief = brief_input(task);
    let policy = &p.intelligence;
    let mut profile = SignalProfile {
        schema_version: 1,
        brief_version,
        brief_fingerprint: fingerprint(&brief),
        project_profile_version: fingerprint(policy),
        extractor_version: EXTRACTOR.into(),
        signals: SignalName::ALL.into_iter().map(|n| Signal::unknown(n, EXTRACTOR)).collect(),
        affected_paths: paths,
    };
    let set = |profile: &mut SignalProfile,
               name: SignalName,
               value: &str,
               source: SignalSource,
               evidence: Vec<String>,
               confidence: Option<f64>| {
        if let Some(s) = profile.signals.iter_mut().find(|s| s.name == name) {
            s.value = value.into();
            s.source = source;
            s.evidence_refs = evidence;
            s.assessor_version = if source == SignalSource::Policy { policy.version.clone() } else { EXTRACTOR.into() };
            s.confidence = confidence;
        }
    };
    if policy.enabled {
        for component in &policy.components {
            if profile.affected_paths.is_empty()
                || profile
                    .affected_paths
                    .iter()
                    .any(|path| component.paths.iter().any(|prefix| genie_core::intelligence::path_matches(path, prefix)))
            {
                let mut evidence = profile
                    .signals
                    .iter()
                    .find(|s| s.name == SignalName::Sensitivity && s.source == SignalSource::Policy)
                    .map(|s| s.evidence_refs.clone())
                    .unwrap_or_default();
                evidence.push(format!("project-policy:{}:component:{}", policy.version, component.id));
                let old = profile.value(SignalName::Sensitivity);
                let sensitivity =
                    if old != "unknown" && old != component.sensitivity { "protected".to_string() } else { component.sensitivity.clone() };
                set(&mut profile, SignalName::Sensitivity, &sensitivity, SignalSource::Policy, evidence.clone(), None);
                if component.sensitivity != "ordinary" {
                    set(&mut profile, SignalName::BlastRadius, "high", SignalSource::Policy, evidence, None);
                }
            }
        }
        let high_unknown = profile.value(SignalName::BlastRadius) == "high" && profile.value(SignalName::Scope) == "unknown";
        let mechanical = if high_unknown || policy.semantic_assessment {
            let repo = app.with_server(|db| Ok(db.project(project)?.repo))?;
            repo.as_deref().map(Path::new).map(inventory).unwrap_or_default()
        } else {
            Vec::new()
        };
        if !profile.affected_paths.is_empty() {
            let evidence = profile.affected_paths.iter().take(32).map(|p| format!("file:{p}")).collect();
            let components: BTreeSet<_> = profile
                .affected_paths
                .iter()
                .flat_map(|path| {
                    policy
                        .components
                        .iter()
                        .filter(move |c| c.paths.iter().any(|prefix| genie_core::intelligence::path_matches(path, prefix)))
                        .map(|c| &c.id)
                })
                .collect();
            let scope = if components.len() > 1 { "cross_component" } else { "component" };
            set(&mut profile, SignalName::Scope, scope, SignalSource::Human, evidence, None);
        } else if high_unknown {
            let evidence = if mechanical.is_empty() {
                vec!["preflight:insufficient_repository_evidence".into()]
            } else {
                mechanical.iter().take(32).cloned().collect()
            };
            set(&mut profile, SignalName::Scope, "unknown", SignalSource::Deterministic, evidence, None);
        }
        if policy.semantic_assessment && p.allow_external && app.cfg.decisions.enabled {
            let questions=[SignalName::Ambiguity,SignalName::Scope,SignalName::Complexity,SignalName::Verification,SignalName::Novelty].into_iter().map(|name| {
                let key=serde_json::to_value(name).unwrap().as_str().unwrap().to_string();
                let mut criteria:BTreeMap<String,String>=name.values().iter().map(|v|(v.to_string(),v.replace('_'," "))).collect();
                criteria.insert("unknown".into(),"Insufficient supplied evidence; do not invent repository facts".into());
                (key,choice("Classify only from supplied brief and mechanical evidence. Unknown is not low risk; confidence is not implementation success probability.",criteria))
            }).collect();
            let request = Request {
                task: Some(task.id.clone()),
                purpose: "task-signals-v1".into(),
                state: json!({"brief":brief,"mechanicalFacts":mechanical,"affectedPaths":profile.affected_paths}),
                questions,
            };
            let key = fingerprint(&json!({"request":request,"policy":p,"connection":app.cfg.decisions}));
            let initiator = decisions::initiator(app, project, actor, &task.id);
            if let Ok(record) = crate::state::block_on(decisions::evaluate(app, project, actor, initiator.as_deref(), &request, Some(&key)))
            {
                for name in
                    [SignalName::Ambiguity, SignalName::Scope, SignalName::Complexity, SignalName::Verification, SignalName::Novelty]
                {
                    let key = serde_json::to_value(name).unwrap().as_str().unwrap().to_string();
                    if name == SignalName::Scope && !profile.affected_paths.is_empty() {
                        continue;
                    }
                    if let Some(value) = decisions::selected(&record, &key) {
                        set(
                            &mut profile,
                            name,
                            value,
                            SignalSource::Jev,
                            vec![format!("decision:{}", record.id)],
                            record.result["answers"][&key]["confidence"].as_f64(),
                        );
                    }
                }
            }
        }
    }
    // Reuse identical snapshots, but a material trigger always creates evidence of
    // that discovery. This is not mutation of an earlier assessment.
    app.with_tracker(project, |t| {
        let history = t.intelligence_history(&task.id)?;
        if trigger.is_none() {
            if let Some(previous) = history.snapshots.last() {
                let mut current = profile.clone();
                let mut old = previous.profile.clone();
                for s in &mut current.signals {
                    s.assessed_at.clear();
                }
                for s in &mut old.signals {
                    s.assessed_at.clear();
                }
                if current == old {
                    return Ok(previous.clone());
                }
            }
        }
        t.append_signal_snapshot(actor, &task.id, &profile, trigger)
    })
}

pub fn required(policy: &IntelligencePolicy, profile: &SignalProfile) -> Vec<String> {
    if policy.enabled { policy.required(&profile.affected_paths) } else { vec![] }
}
pub(crate) fn roster_satisfies(agents: &AgentConfig, roster: &Roster, policy: &IntelligencePolicy, checks: &[String]) -> bool {
    checks.iter().all(|check| {
        policy.checks.get(check).is_some_and(|role| {
            roster.asked.iter().any(|(_, m)| &m.role == role && agents.roles.get(role).is_some_and(|r| r.class == Role::Reviewer))
        })
    })
}
pub(crate) fn eligible(app: &App, project: &str, p: &Policy, snapshot: &Snapshot, roster: &Roster) -> AppResult<Vec<String>> {
    let agents = app.agents();
    let checks = required(&p.intelligence, &snapshot.profile);
    let mut ids: BTreeSet<String> = p.templates.iter().cloned().collect();
    if let Some(t) = &roster.template {
        ids.insert(t.id.clone());
    }
    if let Some(t) = &p.intelligence.fallback_template {
        ids.insert(t.clone());
    }
    let classes: Vec<_> = roster
        .asked
        .iter()
        .filter_map(|(_, m)| agents.roles.get(&m.role))
        .map(|r| r.class)
        .filter(|class| matches!(class, Role::Executor | Role::Reviewer))
        .collect();
    let workspace = roster.template.as_ref().map(|t| t.workspace).unwrap_or(crate::agent_config::Workspace::Worktree);
    Ok(ids
        .into_iter()
        .filter(|id| {
            agents.team_for(project, id).is_ok_and(|t| {
                t.stage == Stage::Delivery
                    && classes.iter().all(|class| t.members.iter().any(|m| agents.roles.get(&m.role).is_some_and(|r| r.class == *class)))
                    && t.workspace == workspace
                    && t.members.len() <= app.cfg.limits.max_members_per_team
                    && checks.iter().all(|c| t.members.iter().any(|m| p.intelligence.checks.get(c) == Some(&m.role)))
                    && t.members.iter().all(|m| agents.role_for(project, &m.role).is_ok_and(|r| r.stages.contains(&Stage::Delivery)))
            })
        })
        .collect())
}
pub(crate) fn enforce(agents: &AgentConfig, roster: &Roster, p: &Policy, snapshot: &Snapshot) -> AppResult<()> {
    if !p.intelligence.enabled {
        return Ok(());
    }
    if snapshot.profile.value(SignalName::Ambiguity) == "needs_owner_decision" {
        return Err(AppError::Conflict("task assessment requires an owner decision; clarify the brief before dispatch".into()));
    }
    let checks = required(&p.intelligence, &snapshot.profile);
    if !roster_satisfies(agents, roster, &p.intelligence, &checks) {
        return Err(AppError::Conflict(format!(
            "team does not satisfy mandatory policy checks: {}; add the configured independent reviewer roles",
            checks.join(", ")
        )));
    }
    Ok(())
}
pub(crate) fn models(agents: &AgentConfig, roster: &Roster) -> Vec<ModelSelection> {
    roster
        .asked
        .iter()
        .map(|(_, m)| {
            let role = agents.roles.get(&m.role);
            ModelSelection {
                role: m.role.clone(),
                model: m.model.clone().or_else(|| role.and_then(|r| r.model.clone())),
                thinking: m.thinking.clone().or_else(|| role.and_then(|r| r.thinking.clone())),
            }
        })
        .collect()
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssessBody {
    #[serde(default)]
    pub affected_paths: Vec<String>,
}
pub fn assess_task(app: &App, caller: &Caller, id: &str, body: AssessBody) -> AppResult<Snapshot> {
    tasks::authorize(app, caller, id, Touch::Edit)?;
    if !caller.is_person() && !body.affected_paths.is_empty() {
        return Err(AppError::Forbidden(
            "only people can narrow policy scope with affected paths; agents report discoveries through reassessment".into(),
        ));
    }
    let gate = app.decision_gates.task(&caller.project, id);
    let _guard = gate.lock().map_err(|_| AppError::Internal("assessment lock poisoned".into()))?;
    let task = app.with_tracker(&caller.project, |t| t.get(id))?;
    let p = decisions::policy(app, &caller.project)?;
    assess(app, &caller.project, &caller.actor, &task, &p, body.affected_paths, None)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReassessBody {
    pub request_id: String,
    pub reason: String,
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub affected_paths: Vec<String>,
}
pub fn reassess(app: &App, caller: &Caller, id: &str, body: ReassessBody) -> AppResult<Value> {
    use genie_core::intelligence::{DecisionKind, RoutingStatus};
    tasks::authorize(app, caller, id, Touch::Edit)?;
    if caller.actor.role != Role::Orchestrator && !caller.is_person() {
        return Err(AppError::Forbidden("runtime routing is owned by the task manager or a person".into()));
    }
    if body.request_id.is_empty()
        || body.request_id.chars().any(char::is_control)
        || body.request_id.len() > 80
        || body.evidence_refs.is_empty()
        || body.evidence_refs.len() > 32
        || body.evidence_refs.iter().any(|r| r.trim().is_empty() || r.len() > 512 || r.chars().any(char::is_control))
        || body.affected_paths.len() > 128
        || body.affected_paths.iter().any(|p| !genie_core::intelligence::valid_path(p))
    {
        return Err(AppError::Bad("reassessment needs a bounded requestId, evidence references and valid relative paths".into()));
    }
    if !["sensitive_change", "scope_expansion", "irreversible_change", "verification_gap", "review_return", "requirement_conflict"]
        .contains(&body.reason.as_str())
    {
        return Err(AppError::Bad("unknown material reassessment reason".into()));
    }
    let task = app.with_tracker(&caller.project, |t| t.get(id))?;
    let gate = app.decision_gates.task(&caller.project, &task.id);
    let _guard = gate.lock().map_err(|_| AppError::Internal("reassessment lock poisoned".into()))?;
    let key = format!("reassess:{}", body.request_id);
    let input_key = format!("input:{}", fingerprint(&body));
    let history = app.with_tracker(&caller.project, |t| t.intelligence_history(&task.id))?;
    // The request id is retained as an evidence reason code, so replays return the
    // applied result without starting another provider call or adding members.
    if let Some(previous) = history.routing.iter().find(|r| r.plan.reason_codes.contains(&key)) {
        if !previous.plan.reason_codes.contains(&input_key) {
            return Err(AppError::Conflict("requestId was already used for a different material discovery".into()));
        }
        return Ok(json!({"routing":previous,"reused":true}));
    }
    let p = decisions::policy(app, &caller.project)?;
    if !p.intelligence.enabled {
        return Err(AppError::Conflict("task intelligence is disabled for this project".into()));
    }
    let mut paths = history.snapshots.last().map(|s| s.profile.affected_paths.clone()).unwrap_or_default();
    // An agent may expand known scope, but cannot turn unknown scope into a claim
    // that protected components are unaffected. A person can make that explicit.
    if caller.is_person() || !paths.is_empty() {
        paths.extend(body.affected_paths);
        paths.sort();
        paths.dedup();
    }
    let team_id = task.team.as_deref().ok_or_else(|| AppError::Conflict("reassessment requires an active delivery team".into()))?;
    let team = app.with_tracker(&caller.project, |t| t.bus().get(team_id))?;
    if team.state != genie_core::TeamState::Active {
        return Err(AppError::Conflict("reassessment requires an active team".into()));
    }
    let checks = if p.intelligence.enabled { p.intelligence.required(&paths) } else { vec![] };
    let mut roles: BTreeSet<String> = checks.iter().filter_map(|c| p.intelligence.checks.get(c).cloned()).collect();
    if matches!(body.reason.as_str(), "scope_expansion" | "verification_gap" | "review_return") {
        roles.insert("reviewer".into());
    }
    let missing: Vec<_> = roles.into_iter().filter(|role| !team.members.iter().any(|m| &m.role == role)).collect();
    if missing.is_empty() && !["sensitive_change", "irreversible_change", "requirement_conflict"].contains(&body.reason.as_str()) {
        if let Some(last) = history.routing.iter().rev().find(|r| matches!(r.plan.decision_kind, DecisionKind::Reassessment)) {
            if chrono::DateTime::parse_from_rfc3339(&last.created_at)
                .is_ok_and(|at| chrono::Utc::now().signed_duration_since(at).num_seconds() < i64::from(p.intelligence.cooldown_secs))
            {
                return Err(AppError::Conflict(
                    "reassessment cooldown is active; required safety upgrades are still allowed immediately".into(),
                ));
            }
        }
    }
    if team.members.len() + missing.len() > app.cfg.limits.max_members_per_team {
        return Err(AppError::Conflict(
            "required safety upgrade exceeds the team member limit; pause and request an owner decision".into(),
        ));
    }
    let event = app.with_tracker(&caller.project, |t| {
        t.tx(|| {
            t.append_event(
                "task.material_discovery",
                Some(&task.id),
                &caller.actor.name,
                caller.actor.role.as_str(),
                json!({"requestId":body.request_id,"reason":body.reason,"evidenceRefs":body.evidence_refs}),
            )
        })
    })?;
    let mut snapshot = assess(app, &caller.project, &caller.actor, &task, &p, paths, Some(event))?;
    // Runtime discoveries can strengthen a semantic judgement; they never clear a
    // policy signal. Preserve the discovery as a separate immutable revision.
    let change = match body.reason.as_str() {
        "scope_expansion" => Some((SignalName::Scope, "cross_component")),
        "requirement_conflict" => Some((SignalName::Ambiguity, "needs_owner_decision")),
        "irreversible_change" => Some((SignalName::Reversibility, "low")),
        "verification_gap" => Some((SignalName::Verification, "missing")),
        "sensitive_change" => Some((SignalName::Sensitivity, "protected")),
        _ => None,
    };
    if let Some((name, value)) = change {
        if let Some(s) = snapshot.profile.signals.iter_mut().find(|s| s.name == name && s.source != SignalSource::Policy) {
            s.value = value.into();
            s.source = SignalSource::Runtime;
            s.evidence_refs = body.evidence_refs.clone();
            s.assessed_at = genie_core::db::now();
        }
        snapshot =
            app.with_tracker(&caller.project, |t| t.append_signal_snapshot(&caller.actor, &task.id, &snapshot.profile, Some(event)))?;
    }
    if app.with_tracker(&caller.project, |t| Ok(serde_json::to_value(t.get(&task.id)?)?))?
        != serde_json::to_value(&task).map_err(genie_core::GenieError::from)?
        || decisions::policy(app, &caller.project)? != p
    {
        return Err(AppError::Conflict("task or policy changed during reassessment; retry against current facts".into()));
    }
    let blocked = body.reason == "requirement_conflict"
        || (body.reason == "irreversible_change" && !checks.iter().any(|c| c == "recovery_plan"))
        || (body.reason == "sensitive_change" && checks.is_empty());
    if !blocked {
        for role in &missing {
            crate::runtime::add_member(
                app,
                &caller.project,
                team_id,
                crate::config::MemberSpec {
                    role: role.clone(),
                    name: None,
                    model: None,
                    thinking: None,
                    instructions: Some(format!(
                        "Mandatory independent check after {}. Evidence: {}",
                        body.reason,
                        body.evidence_refs.join(", ")
                    )),
                },
                &caller.actor.name,
            )?;
        }
    }
    if blocked {
        for member in &team.members {
            app.with_tracker(&caller.project, |t| t.bus().set_paused(team_id, &member.name, true, &caller.actor.name))?;
            crate::sessions::reset(
                app,
                &crate::runtime::AgentKey::Member { project: caller.project.clone(), team: team_id.into(), member: member.name.clone() },
            );
        }
    }
    if blocked && task.status != genie_core::Status::NeedsOwner {
        tasks::set_status(
            app,
            caller,
            &task.id,
            tasks::StatusBody::to(
                genie_core::Status::NeedsOwner,
                Some(format!(
                    "Task intelligence discovered {}. Execution is paused; resolve the requirement or configure mandatory safety checks before resuming. Evidence: {}",
                    body.reason,
                    body.evidence_refs.join(", ")
                )),
            ),
        )?;
    }
    let final_team = app.with_tracker(&caller.project, |t| t.bus().get(team_id))?;
    let agents = app.agents();
    let selections = final_team
        .members
        .iter()
        .map(|m| ModelSelection {
            role: m.role.clone(),
            model: m.model.clone().or_else(|| agents.roles.get(&m.role).and_then(|r| r.model.clone())),
            thinking: m.thinking.clone().or_else(|| agents.roles.get(&m.role).and_then(|r| r.thinking.clone())),
        })
        .collect();
    let mut reasons = vec![key.clone(), input_key, body.reason.clone()];
    if !blocked && !missing.is_empty() {
        reasons.push("topology_upgrade".into());
    }
    if blocked {
        reasons.push("owner_decision_required".into());
    }
    let plan = RoutingPlan {
        decision_kind: DecisionKind::Reassessment,
        router_provider: "server_policy".into(),
        router_version: EXTRACTOR.into(),
        policy_version: fingerprint(&p),
        eligible_templates: final_team.template.iter().cloned().collect(),
        selected_template: final_team.template.clone(),
        selected_models: selections,
        required_checks: checks,
        decision_status: if blocked { RoutingStatus::Abstained } else { RoutingStatus::Selected },
        override_reason: None,
        reason_codes: reasons,
        provider_decision_ids: vec![],
    };
    let routing = app.with_tracker(&caller.project, |t| {
        t.tx(|| {
            let a = t.begin_attempt(&caller.actor, &task.id)?;
            t.append_routing(&caller.actor, &task.id, Some(a.id), snapshot.id, &key, &plan)
        })
    })?;
    tasks::changed(app);
    Ok(json!({"snapshot":snapshot,"routing":routing,"team":final_team,"requiresOwner":blocked}))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShadowBody {
    pub snapshot_id: i64,
    pub candidate: Policy,
}
pub fn shadow(app: &App, caller: &Caller, id: &str, body: ShadowBody) -> AppResult<Value> {
    tasks::authorize(app, caller, id, Touch::Edit)?;
    let task = app.with_tracker(&caller.project, |t| t.get(id))?;
    let current = decisions::policy(app, &caller.project)?;
    decisions::validate_policy(app, &caller.project, &body.candidate)?;
    let snapshot = app.with_tracker(&caller.project, |t| t.signal_snapshot(&task.id, body.snapshot_id))?;
    let history = app.with_tracker(&caller.project, |t| t.intelligence_history(&task.id))?;
    let baseline = history.routing.iter().rev().find(|r| r.snapshot_id == snapshot.id);
    let checks = body.candidate.intelligence.required(&snapshot.profile.affected_paths);
    let agents = app.agents();
    let eligible: Vec<_> = body
        .candidate
        .templates
        .iter()
        .filter(|id| {
            agents
                .team_for(&caller.project, id)
                .is_ok_and(|t| checks.iter().all(|c| t.members.iter().any(|m| body.candidate.intelligence.checks.get(c) == Some(&m.role))))
        })
        .cloned()
        .collect();
    let mut criteria: BTreeMap<String, String> = eligible.iter().map(|id| (id.clone(), format!("Configured template {id}"))).collect();
    criteria.insert("unknown".into(), "Insufficient evidence; abstain".into());
    let frozen =
        json!({"snapshot":snapshot,"candidate":body.candidate,"baseline":baseline,"eligibleTemplates":eligible,"requiredChecks":checks});
    let mut record = None;
    if current.allow_external && app.cfg.decisions.enabled && criteria.len() > 1 {
        let request = Request {
            task: Some(task.id.clone()),
            purpose: "shadow-policy-v1".into(),
            state: frozen.clone(),
            questions: BTreeMap::from([(
                "team".into(),
                choice(
                    "Shadow evaluation only. Choose among eligible templates from frozen evidence; do not predict unobserved outcomes.",
                    criteria,
                ),
            )]),
        };
        let key = fingerprint(&json!({"request":request,"currentGrant":current,"connection":app.cfg.decisions}));
        let initiator = decisions::initiator(app, &caller.project, &caller.actor, &task.id);
        record =
            crate::state::block_on(decisions::evaluate(app, &caller.project, &caller.actor, initiator.as_deref(), &request, Some(&key)))
                .ok();
    }
    let selected = record.as_ref().and_then(|r| decisions::selected(r, "team")).filter(|id| eligible.iter().any(|e| e == *id));
    let recommendation = if selected.is_some() {
        "Inspect the candidate plan against comparable accepted-work cost and review cohorts before a human administrator changes production policy."
    } else {
        "Gather missing decision-time evidence or repair provider availability before proposing a routing policy change."
    };
    let evidence = json!({"schemaVersion":1,"mode":"shadow","frozenInput":frozen,"providerDecisionId":record.as_ref().map(|r|r.id),"status":record.as_ref().map_or("unavailable",|r|r.status.as_str()),"selectedTemplate":selected,"baselineTemplate":baseline.and_then(|r|r.plan.selected_template.as_deref()),"recommendation":recommendation,"limitations":["Descriptive comparison; no counterfactual quality or cost prediction","Small cohorts require human evaluation"],"productionPolicyChanged":false});
    let evaluation = app.with_tracker(&caller.project, |t| t.append_policy_evaluation(&caller.actor, &task.id, snapshot.id, &evidence))?;
    Ok(json!(evaluation))
}

/// Evidence-only retrospective proposals. Never writes a production policy.
pub fn recommendations(history: &genie_core::intelligence::IntelligenceHistory) -> Value {
    let upgrades: Vec<_> =
        history.routing.iter().filter(|r| r.plan.reason_codes.iter().any(|c| c == "topology_upgrade")).map(|r| r.id).collect();
    let fallback: Vec<_> = history
        .routing
        .iter()
        .filter(|r| {
            matches!(
                r.plan.decision_status,
                genie_core::intelligence::RoutingStatus::Fallback | genie_core::intelligence::RoutingStatus::Abstained
            )
        })
        .map(|r| r.id)
        .collect();
    let mut proposals = Vec::new();
    if !upgrades.is_empty() {
        proposals.push(json!({"proposal":"Review whether the initial template should include the specialist required by these discoveries.","routingIds":upgrades,"requiresHumanApproval":true}));
    }
    if !fallback.is_empty() {
        proposals.push(json!({"proposal":"Investigate fallback/abstention evidence and evaluate a candidate policy in shadow mode before enabling it.","routingIds":fallback,"requiresHumanApproval":true}));
    }
    json!({"proposals":proposals,"snapshotIds":history.snapshots.iter().map(|s|s.id).collect::<Vec<_>>(),"attemptIds":history.attempts.iter().map(|a|a.id).collect::<Vec<_>>(),"evaluationIds":history.evaluations.iter().map(|e|e.id).collect::<Vec<_>>(),"limitations":["A single task is insufficient to establish a policy improvement.","No counterfactual or production outcome is inferred."],"policyChanged":false})
}

pub(crate) fn record_live_plan(
    t: &genie_core::Tracker,
    agents: &AgentConfig,
    p: &Policy,
    actor: &Actor,
    team_id: &str,
    kind: genie_core::intelligence::DecisionKind,
    reason: &str,
) -> genie_core::Result<()> {
    let team = t.bus().get(team_id)?;
    let history = t.intelligence_history(&team.task)?;
    let Some(snapshot) = history.snapshots.last() else {
        return Ok(());
    };
    let Some(attempt) = t.open_attempt(&team.task)? else {
        return Ok(());
    };
    let models = team
        .members
        .iter()
        .map(|m| ModelSelection {
            role: m.role.clone(),
            model: m.model.clone().or_else(|| agents.roles.get(&m.role).and_then(|r| r.model.clone())),
            thinking: m.thinking.clone().or_else(|| agents.roles.get(&m.role).and_then(|r| r.thinking.clone())),
        })
        .collect();
    let plan = RoutingPlan {
        decision_kind: kind,
        router_provider: "explicit".into(),
        router_version: EXTRACTOR.into(),
        policy_version: fingerprint(p),
        eligible_templates: team.template.iter().cloned().collect(),
        selected_template: if reason == "manual_model_change" { team.template.clone() } else { None },
        selected_models: models,
        required_checks: required(&p.intelligence, &snapshot.profile),
        decision_status: genie_core::intelligence::RoutingStatus::Overridden,
        override_reason: Some(reason.into()),
        reason_codes: vec![reason.into()],
        provider_decision_ids: vec![],
    };
    let event = t.append_event(
        "task.routing_input_changed",
        Some(&team.task),
        &actor.name,
        actor.role.as_str(),
        json!({"team":team_id,"reason":reason}),
    )?;
    t.append_routing(actor, &team.task, Some(attempt.id), snapshot.id, &format!("change:{event}"), &plan)?;
    Ok(())
}
