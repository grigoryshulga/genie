//! A common human interaction view and response path. References point to authoritative
//! clarifications, proposals, questionnaire items and execution fences; no answer is copied.

use genie_core::conversations::{ProposalAction, ProposalStatus};
use genie_core::{Actor, GenieError, Role};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::http::ctx::Access;
use crate::state::{App, AppError, AppResult};

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, rename = "HumanInteraction")]
#[serde(rename_all = "camelCase")]
pub struct Interaction {
    pub id: String,
    pub project: String,
    pub task: Option<String>,
    pub kind: String,
    pub version: String,
    pub prompt: String,
    pub state: String,
    pub actions: Vec<Action>,
    pub recipient: Option<i64>,
    #[ts(type = "unknown")]
    pub resolution: Option<Value>,
    pub options: Vec<String>,
    pub deliveries: Vec<genie_core::human_delivery::InteractionDeliveryReceipt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema, clap::ValueEnum, ts_rs::TS)]
#[ts(export, rename = "HumanInteractionAction")]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Answer,
    Approve,
    Reject,
    Resume,
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Answer => "answer",
            Self::Approve => "approve",
            Self::Reject => "reject",
            Self::Resume => "resume",
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, rename = "HumanInteractionResponse", optional_fields)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: String,
    pub action: Action,
    pub text: Option<String>,
}

enum Subject<'a> {
    Clarification(&'a str, i64),
    Proposal(&'a str),
    Questionnaire(i64, i64),
    Pause(&'a str, i64),
}

fn subject<'a>(project: &str, id: &'a str) -> AppResult<Subject<'a>> {
    let parts = id.split(':').collect::<Vec<_>>();
    let bad = || AppError::Bad("invalid interaction reference".into());
    let positive = |v: &str| v.parse::<i64>().ok().filter(|n| *n > 0).ok_or_else(bad);
    match parts.as_slice() {
        [p, "clarification", task, n] if *p == project => Ok(Subject::Clarification(task, positive(n)?)),
        [p, "proposal", proposal] if *p == project => Ok(Subject::Proposal(proposal)),
        [p, "questionnaire", qn, n] if *p == project => Ok(Subject::Questionnaire(positive(qn)?, positive(n)?)),
        [p, "pause", task, generation] if *p == project => Ok(Subject::Pause(task, positive(generation)?)),
        _ => Err(bad()),
    }
}

/// A cached Access or a copied login never grants authority to a new human decision.
pub fn access(app: &App, old: &Access) -> AppResult<Access> {
    let uid = old
        .user
        .as_ref()
        .filter(|_| !old.agent && old.actor.role == Role::Human)
        .map(|u| u.id)
        .ok_or_else(|| AppError::Forbidden("a personal account is required to answer or decide".into()))?;
    // Native operator access is minted by Ctx from the local server connection, never
    // from a remote token or request body. Preserve its existing task-management authority.
    if uid == 0 && old.user.as_ref().is_some_and(|u| u.is_admin && !u.disabled) && old.role.can_write() {
        return Ok(old.clone());
    }
    for_user(app, &old.project, uid)
}

pub(crate) fn for_user(app: &App, project: &str, uid: i64) -> AppResult<Access> {
    let current = app
        .with_server(|db| {
            let user = db.user(uid)?;
            let role = db.project_role(project, &user)?.filter(|r| !user.disabled && r.can_write());
            Ok(role.map(|role| (user, role)))
        })?
        .ok_or_else(|| AppError::Forbidden("the person no longer has write access to this project".into()))?;
    let (user, role) = current;
    Ok(Access {
        project: project.into(),
        actor: Actor::new(&user.login, Role::Human),
        role,
        user: Some(user),
        agent: false,
        agent_team: None,
        agent_job: None,
        agent_role_id: None,
    })
}

fn principal_id(principal: &str) -> Option<i64> {
    principal.strip_prefix("user:")?.parse().ok()
}

/// Task questions are shared project work. Private conversation history is never needed to
/// answer them, and every transport supplies the same immutable human principal as source.
pub fn answer_clarification(app: &App, old: &Access, task: &str, question: i64, text: &str, version: Option<&str>) -> AppResult<()> {
    let a = access(app, old)?;
    app.with_tracker(&a.project, |t| t.tx(|| answer_in(t, &a, task, question, text, version)))?;
    crate::tasks::changed(app);
    Ok(())
}

pub(crate) fn answer_in(
    t: &genie_core::Tracker,
    a: &Access,
    task: &str,
    question: i64,
    text: &str,
    version: Option<&str>,
) -> genie_core::Result<()> {
    let uid = a.user.as_ref().ok_or_else(|| GenieError::Denied("a human account is required".into()))?.id;
    t.tx(|| {
        let q = t
            .management()
            .clarifications(task)?
            .into_iter()
            .find(|q| q.id == question)
            .ok_or_else(|| GenieError::not_found("clarification"))?;
        if version.is_some_and(|v| v != q.brief_version.to_string()) {
            return Err(GenieError::invalid("the clarification version is stale"));
        }
        let source = if uid == 0 { format!("requester:operator:{}", a.actor.name) } else { format!("requester:user:{uid}") };
        t.management().answer(&a.actor, task, question, text.trim(), &source)
    })
}

pub fn show(app: &App, old: &Access, id: &str) -> AppResult<Interaction> {
    let a = access(app, old)?;
    let project = a.project.clone();
    let mut item = Interaction {
        id: id.into(),
        project: project.clone(),
        task: None,
        kind: String::new(),
        version: String::new(),
        prompt: String::new(),
        state: "open".into(),
        actions: Vec::new(),
        recipient: None,
        resolution: None,
        options: Vec::new(),
        deliveries: Vec::new(),
    };
    match subject(&project, id)? {
        Subject::Clarification(task, question) => {
            let (q, current, recipient) = app.with_tracker(&project, |t| {
                let q = t
                    .management()
                    .clarifications(task)?
                    .into_iter()
                    .find(|q| q.id == question)
                    .ok_or_else(|| GenieError::not_found("clarification"))?;
                let current = t.management().manager(task)?.is_some_and(|m| m.state == "active" && m.brief_version == q.brief_version);
                let recipient = t
                    .conversations()
                    .conversation_for_task(task)?
                    .map(|c| t.conversations().conversation(&c))
                    .transpose()?
                    .and_then(|c| principal_id(&c.principal));
                Ok((q, current, recipient))
            })?;
            item.task = Some(q.task);
            item.kind = "clarification".into();
            item.version = q.brief_version.to_string();
            item.prompt = q.question;
            item.recipient = recipient;
            if q.state == "answered" {
                item.state = "answered".into();
                item.resolution = Some(json!({"answer":q.answer,"source":q.source,"at":q.answered}));
            } else if q.state != "owner" || !current {
                item.state = "stale".into();
            } else {
                item.actions.push(Action::Answer);
            }
        }
        Subject::Proposal(proposal) => {
            let (p, task, recipient, current) = app.with_tracker(&project, |t| {
                let p = t.conversations().proposal(proposal)?;
                let current = p.status != ProposalStatus::Pending || p.version == crate::conversations::task_version(t, &p.task)?;
                let recipient = principal_id(&t.conversations().conversation(&p.conversation_id)?.principal);
                let task = t.get(&p.task)?;
                Ok((p, task, recipient, current))
            })?;
            item.task = Some(p.task);
            item.kind = "proposal".into();
            item.version = p.version;
            item.recipient = recipient;
            item.prompt = format!(
                "{}: {}\n\n{}\n\nPlan: {}\nArtifacts: {}",
                if p.action == ProposalAction::Dispatch { "Dispatch" } else { "Accept result" },
                task.title,
                task.description,
                task.plan,
                task.artifacts.len()
            );
            if p.status != ProposalStatus::Pending {
                item.state = p.status.as_str().into();
                let actor = app.with_tracker(&project, |t| t.conversations().decision_actor(&p.id))?;
                item.resolution = Some(
                    json!({"decision":p.status,"at":p.decided_at,"actor":actor.as_ref().map(|a|&a.0),"role":actor.as_ref().map(|a|&a.1)}),
                );
            } else if !current {
                item.state = "stale".into();
            } else {
                item.actions = vec![Action::Approve, Action::Reject];
            }
        }
        Subject::Questionnaire(id, n) => {
            let qn = app
                .with_server(|db| {
                    let qn = db.questionnaire(id)?;
                    crate::questions::authorize(db, &qn, a.user.as_ref().expect("human").id)?;
                    if qn.project != project {
                        return Err(GenieError::not_found("questionnaire"));
                    }
                    Ok(qn)
                })
                .map_err(crate::questions::authorization_error)?;
            let q = qn.questions.iter().find(|q| q.n == n).ok_or_else(|| GenieError::not_found("questionnaire item"))?;
            item.options = q.options.clone();
            item.task = qn.task;
            item.kind = "questionnaire".into();
            item.version = format!("{}:{n}", qn.created);
            item.prompt = format!(
                "{}{}{}",
                q.text,
                if q.why.is_empty() { String::new() } else { format!("\n\n{}", q.why) },
                if q.options.is_empty() { String::new() } else { format!("\n\n{}", q.options.join(" / ")) }
            );
            item.recipient = Some(qn.recipient);
            if q.answer.is_some() {
                item.state = "answered".into();
                let recording = app.with_server(|db| db.answer_record_status(qn.id))?;
                item.resolution = Some(json!({"answer":q.answer,"source":q.answered_via,"at":q.answered_at,"recording":recording}));
            } else if qn.status != "open" {
                item.state = qn.status;
            } else if qn.due.is_some_and(|d| d <= genie_core::db::now()) {
                item.state = "expired".into();
            } else {
                item.actions.push(Action::Answer);
            }
        }
        Subject::Pause(task, generation) => {
            let state =
                app.with_tracker(&project, |t| t.execution().state(task))?.ok_or_else(|| GenieError::not_found("execution pause"))?;
            item.task = Some(state.task);
            item.kind = "execution_pause".into();
            item.version = generation.to_string();
            item.prompt = state.note;
            if generation == state.generation && state.paused {
                item.actions.push(Action::Resume);
            } else if !state.paused && generation.checked_add(1) == Some(state.generation) {
                item.state = "resumed".into();
                item.resolution = Some(json!({"actor":state.actor,"at":state.changed_at}));
            } else {
                item.state = "stale".into();
            }
        }
    }
    item.deliveries = app.with_server(|db| db.interaction_receipts(&project, id, &item.version, a.user.as_ref().expect("human").id))?;
    Ok(item)
}

/// One project view; questionnaires remain private to their intended recipient.
pub fn list(app: &App, old: &Access, all: bool) -> AppResult<Vec<Interaction>> {
    let a = access(app, old)?;
    let mut refs = app.with_tracker(&a.project, |t| {
        let mut refs = Vec::new();
        let mut stmt =
            t.conn().prepare("SELECT task, id FROM clarifications WHERE state = 'owner' OR (?1 AND state='answered') ORDER BY id")?;
        for row in stmt.query_map([all], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
            let (task, id) = row?;
            refs.push(format!("{}:clarification:{task}:{id}", a.project));
        }
        let mut stmt = t.conn().prepare("SELECT id FROM conversation_proposals WHERE ?1 OR status='pending' ORDER BY created_at, id")?;
        for row in stmt.query_map([all], |r| r.get::<_, String>(0))? {
            refs.push(format!("{}:proposal:{}", a.project, row?));
        }
        let mut stmt = t.conn().prepare("SELECT task, generation FROM task_execution WHERE paused = 1")?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
            let (task, generation) = row?;
            refs.push(format!("{}:pause:{task}:{generation}", a.project));
        }
        Ok(refs)
    })?;
    let uid = a.user.as_ref().expect("human").id;
    let questionnaire_refs = app.with_server(|db| {
        let mut stmt = db.conn().prepare("SELECT q.questionnaire, q.n FROM questions q JOIN questionnaires s ON s.id = q.questionnaire WHERE s.project = ?1 AND s.recipient = ?2 AND (?3 OR (q.answer IS NULL AND s.status='open')) ORDER BY s.id, q.n")?;
        Ok(stmt.query_map(rusqlite::params![a.project,uid,all], |r| Ok(format!("{}:questionnaire:{}:{}",a.project,r.get::<_,i64>(0)?,r.get::<_,i64>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)
    })?;
    refs.extend(questionnaire_refs);
    let mut items = Vec::new();
    for id in refs {
        match show(app, &a, &id) {
            Ok(item) if all || item.state == "open" => items.push(item),
            Ok(_) | Err(AppError::Genie(GenieError::NotFound(_))) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(items)
}

pub fn respond(app: &App, old: &Access, id: &str, response: Response, via: &str) -> AppResult<Interaction> {
    let a = access(app, old)?;
    let item = show(app, &a, id)?;
    if item.version != response.version {
        return Err(AppError::Conflict("the interaction version is stale".into()));
    }
    if !matches!(response.action, Action::Answer) && response.text.is_some() {
        return Err(AppError::Bad("only an answer accepts text".into()));
    }
    match (subject(&a.project, id)?, response.action) {
        (Subject::Clarification(task, question), Action::Answer) => {
            answer_clarification(app, &a, task, question, response.text.as_deref().unwrap_or_default(), Some(&response.version))?
        }
        (Subject::Questionnaire(qn, n), Action::Answer) => {
            crate::questions::answer(app, a.user.as_ref().expect("human").id, qn, n, response.text.as_deref().unwrap_or_default(), via)?;
        }
        (Subject::Proposal(proposal), action @ (Action::Approve | Action::Reject)) => {
            crate::conversations::decide(app, &a, proposal, matches!(action, Action::Approve))?;
        }
        (Subject::Pause(task, generation), Action::Resume) => {
            crate::tasks::resume_interaction(app, &a.caller(), task, generation)?;
        }
        _ => return Err(AppError::Bad("this action does not belong to this interaction".into())),
    }
    crate::tasks::changed(app);
    show(app, &a, id)
}
