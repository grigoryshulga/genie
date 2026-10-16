//! Turn project-approved alternatives into bounded choices, then apply only the
//! authority granted by each dimension's mode. Used by every team spawn entrance.
use super::{Mode, Policy, Request, choice, evaluate, policy, selected};
use crate::agent_config::{AgentConfig, Stage};
use crate::runtime::{Roster, SpawnRequest, asked_roster};
use crate::state::{App, AppError, AppResult};
use genie_core::decisions::Decision;
use genie_core::{Role, Status, Task};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub struct Prepared {
    pub roster: Roster,
    pub records: Vec<Decision>,
    pub notice: Vec<String>,
    pub policy: Policy,
}

fn plan(r: &Roster) -> Value {
    json!({"template":r.template.as_ref().map(|t| &t.id),"members":r.asked.iter().map(|(key,m)| json!({
        "key":key,"role":m.role,"model":m.model,"thinking":m.thinking,"instructions":m.instructions
    })).collect::<Vec<_>>()})
}

fn automatic(mode: Mode) -> bool {
    matches!(mode, Mode::Joint | Mode::Select)
}

fn judge(
    app: &App,
    project: &str,
    req: &SpawnRequest,
    task: &Task,
    p: &Policy,
    state: Value,
    questions: BTreeMap<String, super::Question>,
) -> AppResult<Decision> {
    let request = Request { task: Some(task.id.clone()), purpose: "team-plan".into(), state, questions };
    let fingerprint = json!({"request":request,"policy":p,"model":app.cfg.decisions.model});
    let key = genie_core::server_db::hash_secret(&fingerprint.to_string());
    let initiator = req.initiator.clone().or_else(|| super::initiator(app, project, &req.by, &task.id));
    crate::state::block_on(evaluate(app, project, &req.by, initiator.as_deref(), &request, Some(&key)))
}

pub fn prepare(
    app: &App,
    project: &str,
    req: &mut SpawnRequest,
    task: &Task,
    agents: &AgentConfig,
    mut roster: Roster,
    preview: bool,
) -> AppResult<Prepared> {
    let p = policy(app, project)?;
    let mut records = Vec::new();
    let mut notice = Vec::new();
    // Manual choices are pinned. Refinement and the idea conversation keep their
    // own workflow; automatic participation covers new delivery teams only.
    if (!preview && req.by.role == Role::Human)
        || !app.cfg.decisions.enabled
        || !p.allow_external
        || matches!(task.status, Status::Inbox | Status::Draft | Status::Refining)
        || [p.team, p.model, p.thinking].iter().all(|m| *m == Mode::Off)
    {
        return Ok(Prepared { roster, records, notice, policy: p });
    }
    let original = plan(&roster);
    let original_roster = roster.clone();
    let original_request = req.clone();
    if req.decision_override.is_some() && ![p.team, p.model, p.thinking].contains(&Mode::Joint) {
        return Err(AppError::Bad("an agent may override only a joint decision; select mode does not grant that authority".into()));
    }
    if req.decision_override.as_deref().is_some_and(|s| s.trim().len() < 12 || s.len() > 2000) {
        return Err(AppError::Bad("decision override requires 12–2000 characters of justification".into()));
    }
    let task_state = json!({"id":task.id,"title":task.title,"description":task.description,"acceptance":task.acceptance,
        "plan":task.plan,"labels":task.labels,"updated":task.updated,"mergeStrategy":task.merge_strategy});
    let mut disagreements = Vec::new();
    if p.team != Mode::Off && !p.templates.is_empty() {
        let mut criteria = BTreeMap::from([
            ("proposed".into(), format!("Keep the manager's proposed team: {original}")),
            ("need_evidence".into(), "No listed team is clearly appropriate; keep the manager's proposal and seek more evidence".into()),
        ]);
        let mut alternatives = BTreeMap::new();
        let workspace = roster.template.as_ref().map(|t| t.workspace).unwrap_or(crate::agent_config::Workspace::Worktree);
        let required: Vec<_> = roster
            .asked
            .iter()
            .filter_map(|(_, m)| agents.roles.get(&m.role))
            .filter(|r| matches!(r.class, Role::Executor | Role::Reviewer))
            .map(|r| r.class)
            .collect();
        for (i, id) in p.templates.iter().enumerate() {
            let candidate = agents.team_for(project, id)?;
            if candidate.workspace != workspace
                || candidate.stage != Stage::Delivery
                || candidate.members.len() > app.cfg.limits.max_members_per_team
                || required
                    .iter()
                    .any(|class| !candidate.members.iter().any(|m| agents.roles.get(&m.role).is_some_and(|r| r.class == *class)))
                || candidate.members.iter().any(|m| {
                    agents.role_for(project, &m.role).is_err()
                        || !agents.roles[&m.role].stages.contains(&Stage::Delivery)
                        || (m.model.is_none() && agents.roles[&m.role].model.is_none())
                })
                || roster.template.as_ref().is_some_and(|t| t.id == *id)
            {
                continue;
            }
            let key = format!("team_{i}");
            criteria.insert(
                key.clone(),
                format!(
                    "{}: {}. Members: {}",
                    candidate.title,
                    candidate.description,
                    candidate
                        .members
                        .iter()
                        .map(|m| format!("{}: {}", m.role, agents.roles[&m.role].description))
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            );
            alternatives.insert(key, id.clone());
        }
        if !alternatives.is_empty() {
            let questions = BTreeMap::from([(
                "team".into(),
                choice(
                    "Which listed team is sufficient for the task, keeping independent review and avoiding unnecessary coordination?",
                    criteria,
                ),
            )]);
            match judge(app, project, req, task, &p, json!({"task":task_state,"proposed":original}), questions) {
                Ok(record) => {
                    if let Some(id) = selected(&record, "team").and_then(|key| alternatives.get(key)) {
                        if p.team == Mode::Joint {
                            disagreements.push(format!("team → {id}"));
                        }
                        if (preview && p.team != Mode::Observe) || p.team == Mode::Select {
                            req.template = Some(id.clone());
                            req.members.clear();
                            req.models.clear();
                            roster = asked_roster(agents, project, req)?;
                        }
                    }
                    if p.team != Mode::Observe {
                        notice.push(format!("Jev team decision #{}: {}", record.id, record.result));
                    }
                    records.push(record);
                }
                Err(error) if p.team != Mode::Observe => {
                    notice.push(format!("Jev team evaluation unavailable; keeping the proposal: {error}"))
                }
                Err(_) => {}
            }
        }
    }
    let mut questions = BTreeMap::new();
    let mut candidates = BTreeMap::new();
    let roster_before_profiles = plan(&roster);
    if p.model != Mode::Off || p.thinking != Mode::Off {
        for (i, (_, member)) in roster.asked.iter().enumerate() {
            let role = agents.role_for(project, &member.role)?;
            let model = member.model.as_ref().or(role.model.as_ref());
            let thinking = member.thinking.as_ref().or(role.thinking.as_ref());
            let mut criteria = BTreeMap::from([
                ("proposed".into(), format!("Keep the proposed pair: model={model:?}, thinking={thinking:?}")),
                ("need_evidence".into(), "The available facts do not justify changing the proposed pair".into()),
            ]);
            let mut profiles = BTreeMap::new();
            for (j, profile) in p.profiles.iter().enumerate() {
                if !profile.roles.contains(&member.role) {
                    continue;
                }
                // Off is fixed. When one dimension may apply automatically, the other
                // must remain fixed unless it has the same authority; never splice an
                // unvalidated thinking level from a pair chosen for a different model.
                let model_fixed = p.model == Mode::Off || (automatic(p.thinking) && !automatic(p.model));
                let thinking_fixed = p.thinking == Mode::Off || (automatic(p.model) && !automatic(p.thinking));
                if (model_fixed && model != Some(&profile.model))
                    || (thinking_fixed && thinking != profile.thinking.as_ref())
                    || (model == Some(&profile.model) && thinking == profile.thinking.as_ref())
                {
                    continue;
                }
                let key = format!("profile_{j}");
                criteria.insert(
                    key.clone(),
                    format!("{}: {} (model={}, thinking={:?})", profile.id, profile.description, profile.model, profile.thinking),
                );
                profiles.insert(key, profile.clone());
            }
            if !profiles.is_empty() {
                let id = format!("member_{i}");
                questions.insert(id.clone(),choice(&format!("Which allowed model/thinking pair fits the assignment of {} ({}) for this task? Use the supplied capabilities and constraints, not assumptions about model names.",member.role,role.description),criteria));
                candidates.insert(id, (i, profiles));
            }
        }
    }
    if !questions.is_empty() {
        let mut state = json!({"task":task_state,"team":roster_before_profiles});
        state["roleDefaults"] = json!(
            roster
                .asked
                .iter()
                .map(|(_, m)| {
                    let r = &agents.roles[&m.role];
                    json!({"role":m.role,"model":r.model,"thinking":r.thinking,"description":r.description})
                })
                .collect::<Vec<_>>()
        );
        match judge(app, project, req, task, &p, state, questions) {
            Ok(record) => {
                for (id, (i, profiles)) in candidates {
                    if let Some(profile) = selected(&record, &id).and_then(|key| profiles.get(key)) {
                        let member = &mut roster.asked[i].1;
                        let role = agents.role_for(project, &member.role)?;
                        let model_change = member.model.as_ref().or(role.model.as_ref()) != Some(&profile.model);
                        let thinking_change = member.thinking.as_ref().or(role.thinking.as_ref()) != profile.thinking.as_ref();
                        if (model_change && p.model == Mode::Joint) || (thinking_change && p.thinking == Mode::Joint) {
                            disagreements.push(format!("{} → profile {}", member.role, profile.id));
                        }
                        // A complete curated pair is applied together only when every
                        // changed dimension permits it. Partial application is unsafe.
                        let allows = |m: Mode| m == Mode::Select || (preview && matches!(m, Mode::Advise | Mode::Joint));
                        if (!model_change || allows(p.model)) && (!thinking_change || allows(p.thinking)) {
                            member.model = Some(profile.model.clone());
                            // Explicit off pins no-thinking when a role has a thinking default.
                            member.thinking = Some(profile.thinking.clone().unwrap_or_else(|| "off".into()));
                        }
                    }
                }
                if [p.model, p.thinking].iter().any(|m| matches!(m, Mode::Advise | Mode::Joint | Mode::Select)) {
                    notice.push(format!("Jev profile decision #{}: {}", record.id, record.result));
                }
                records.push(record);
            }
            Err(error) if [p.model, p.thinking].iter().any(|m| matches!(m, Mode::Advise | Mode::Joint | Mode::Select)) => {
                notice.push(format!("Jev profile evaluation unavailable; keeping the proposal: {error}"))
            }
            Err(_) => {}
        }
    }
    if !preview && !disagreements.is_empty() && req.decision_override.as_deref().is_none_or(|s| s.trim().len() < 12) {
        for record in &records {
            app.with_server(|db| {
                db.apply_decision(
                    project,
                    record.id,
                    &json!({"state":"needs-reconsideration","proposed":original,"disagreements":disagreements}),
                )
            })?;
        }
        return Err(AppError::Conflict(format!(
            "Jev requests reconsideration ({}). Read `genie decision list --task {}`; use `genie team recommend` to inspect a proposal, or --decision-override with a specific justification (at least 12 characters).",
            disagreements.join(", "),
            task.id
        )));
    }
    if !preview && req.decision_override.is_some() {
        if disagreements.is_empty() {
            return Err(AppError::Bad("there is no joint disagreement to override; omit decisionOverride".into()));
        }
        // An override keeps the whole original compatible plan, not a mixture of pairs.
        *req = original_request;
        roster = original_roster;
    }
    for record in &records {
        app.with_server(|db|db.apply_decision(project,record.id,&json!({"state":if preview {"preview"}else{"prepared"},"proposed":original,"plan":plan(&roster),"override":req.decision_override,"by":req.by.name})))?;
    }
    Ok(Prepared { roster, records, notice, policy: p })
}

pub fn view(prepared: &Prepared) -> Value {
    json!({"plan":plan(&prepared.roster),"decisions":prepared.records,"notices":prepared.notice,"preview":true})
}
