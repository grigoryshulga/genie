//! Task model and workflow rules shared by the server, the CLI and agent tools.
//! Ported from the TypeScript tracker, whose transition table and DoR/DoD
//! checks it keeps.

use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use serde::{Deserialize, Serialize};

use crate::error::GenieError;

/// String-backed enum stored as TEXT and serialised as the same string.
macro_rules! str_enum {
    ($(#[$meta:meta])* $name:ident ($label:literal) { $($variant:ident => $s:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
        #[ts(export)]
        pub enum $name { $(#[serde(rename = $s)] $variant),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];
            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $s),+ }
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.as_str()) }
        }
        impl FromStr for $name {
            type Err = GenieError;
            fn from_str(s: &str) -> Result<Self, GenieError> {
                match s {
                    $($s => Ok($name::$variant),)+
                    _ => Err(GenieError::invalid(format!(concat!("unknown ", $label, " {}"), s))),
                }
            }
        }
        impl ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> { Ok(ToSqlOutput::from(self.as_str())) }
        }
        impl FromSql for $name {
            fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
                value.as_str()?.parse().map_err(|e| FromSqlError::Other(Box::new(e)))
            }
        }
    };
}

str_enum!(
    /// Task lifecycle status.
    Status("status") {
        Inbox => "inbox",                         // submitted by the owner; the orchestrator has not taken it yet
        Draft => "draft",                         // taken by the orchestrator, not analysed yet
        Refining => "refining",                   // scope and acceptance criteria are being clarified
        Ready => "ready",                         // Definition of Ready met; can be handed to a team
        InProgress => "in_progress",              // a team is working on it
        Review => "review",                       // executor submitted the result for review
        ChangesRequested => "changes_requested",  // reviewer/tester sent it back
        Approved => "approved",                   // reviewer approved; waits for orchestrator acceptance
        NeedsOwner => "needs_owner",              // stuck on a decision only the owner can make
        Done => "done",                           // accepted and closed by the orchestrator
        Cancelled => "cancelled",
    }
);

str_enum!(
    /// Who acts: the human owner, the orchestrator or a team member role.
    Role("role") {
        Human => "human",
        Orchestrator => "orchestrator",
        Analyst => "analyst",
        Executor => "executor",
        Reviewer => "reviewer",
        Tester => "tester",
        Documenter => "documenter",
    }
);

str_enum!(
    TaskType("type") {
        Epic => "epic",
        Task => "task",
        Bug => "bug",
        Spike => "spike",
    }
);

str_enum!(
    CommentKind("comment kind") {
        Note => "note",
        Progress => "progress",
        Question => "question",
        Decision => "decision",
        Review => "review",
        Handoff => "handoff",
        Owner => "owner",
    }
);

str_enum!(
    ArtifactKind("artifact kind") {
        Analysis => "analysis",
        Plan => "plan",
        Code => "code",
        Review => "review",
        TestReport => "test-report",
        Diff => "diff",
        Doc => "doc",
        Log => "log",
        Other => "other",
    }
);

str_enum!(
    /// What a team role may do. Each role gets its class's set (`class_capabilities`),
    /// adjusted by the role's `allow` and `deny` in the server configuration.
    Capability("permission") {
        StatusRefine => "status.refine",         // draft → refining
        StatusStart => "status.start",           // ready → in_progress
        StatusRework => "status.rework",         // changes_requested → in_progress
        StatusSubmit => "status.submit",         // in_progress → review
        StatusApprove => "status.approve",       // review → approved
        StatusReturn => "status.return",         // review → changes_requested
        TaskScope => "task.scope",               // title, type, description, criteria, deps, epic
        TaskPlan => "task.plan",
        TaskCheck => "task.check",               // tick acceptance criteria
        TaskCreate => "task.create",             // subtasks of the own task
        TaskBlock => "task.block",
        DocsRead => "docs.read",
        DocsWrite => "docs.write",               // the section policy still applies
        MailTeam => "mail.team",                 // send / ask teammates
        MailOrchestrator => "mail.orchestrator", // write to the orchestrator directly
        TeamPeek => "team.peek",                 // see what a teammate is doing
    }
);

str_enum!(
    /// Whether a team works. A stopped team keeps its roster, mail and log.
    TeamState("team state") {
        Active => "active",
        Stopped => "stopped",
    }
);

str_enum!(
    /// Whether a team member may run. Only an `active` member of an active team runs;
    /// `paused` and `error` keep their mail, `stopped` (with its team) does not.
    MemberState("member state") {
        Active => "active",
        Paused => "paused",     // a person holds it
        Stopped => "stopped",   // its team was stopped on purpose
        Error => "error",       // it gave up; someone restarts it
    }
);

str_enum!(
    /// What a member is doing right now (runtime bookkeeping, shown on the board).
    Activity("activity") {
        Idle => "idle",
        Working => "working",
        Error => "error",
    }
);

str_enum!(
    /// How a task's delivery to a repository stands.
    DeliveryState("delivery state") {
        Pending => "pending",       // nothing pushed yet
        Published => "published",   // a branch was pushed (and maybe a request opened)
        Merged => "merged",
        Abandoned => "abandoned",   // its request was closed unmerged
    }
);

str_enum!(
    /// A pull/merge request on the host.
    RequestState("request state") {
        Open => "open",
        Merged => "merged",
        Closed => "closed",
    }
);

str_enum!(
    /// The checks (CI) of a watched commit as the watcher records them: the host's answer, or
    /// `stalled` — they stayed `pending` past `runtime.ciPendingSecs` and the team was told.
    CheckState("checks state") {
        None => "none",         // the host shows no checks
        Pending => "pending",
        Passed => "passed",
        Failed => "failed",
        Stalled => "stalled",
    }
);

pub const CLOSED: &[Status] = &[Status::Done, Status::Cancelled];

/// Statuses that mean a team is actually working on a task (they start its epic).
pub const WORKING: &[Status] = &[Status::InProgress, Status::Review, Status::ChangesRequested, Status::Approved];

pub const MEMBER_ROLES: &[Role] = &[Role::Analyst, Role::Executor, Role::Reviewer, Role::Tester, Role::Documenter];

/// Transitions that only the orchestrator (or the human) may make.
pub const ORCHESTRATOR_ONLY: &[Status] = &[Status::Draft, Status::Ready, Status::NeedsOwner, Status::Done, Status::Cancelled];

/// Team verdicts the orchestrator must not fake: it needs `force` to set them itself.
pub const TEAM_ONLY: &[Status] = &[Status::Review, Status::Approved];

/// The transitions a team member may make and the permission each one needs.
/// "human" may do anything; "orchestrator" anything except the team's verdicts (see `Actor::may_move`).
pub const TEAM_TRANSITIONS: &[(Status, Status, Capability)] = &[
    (Status::Draft, Status::Refining, Capability::StatusRefine),
    (Status::Ready, Status::InProgress, Capability::StatusStart),
    (Status::ChangesRequested, Status::InProgress, Capability::StatusRework),
    (Status::InProgress, Status::Review, Capability::StatusSubmit),
    (Status::Review, Status::ChangesRequested, Capability::StatusReturn),
    (Status::Review, Status::Approved, Capability::StatusApprove),
];

/// Permissions every team class has.
pub const COMMON_CAPABILITIES: &[Capability] = &[
    Capability::TaskBlock,
    Capability::DocsRead,
    Capability::DocsWrite,
    Capability::MailTeam,
    Capability::MailOrchestrator,
    Capability::TeamPeek,
];

/// The permissions of a role class: exactly the built-in process rules.
pub fn class_capabilities(role: Role) -> Vec<Capability> {
    use Capability::*;
    let own: &[Capability] = match role {
        Role::Human | Role::Orchestrator => return Capability::ALL.to_vec(),
        Role::Analyst => &[StatusRefine, StatusStart, TaskScope, TaskPlan, TaskCreate],
        Role::Executor => &[StatusStart, StatusRework, StatusSubmit, TaskPlan],
        Role::Reviewer => &[StatusApprove, StatusReturn, TaskCheck],
        Role::Tester => &[StatusReturn],
        Role::Documenter => &[],
    };
    Capability::ALL.iter().copied().filter(|c| own.contains(c) || COMMON_CAPABILITIES.contains(c)).collect()
}

/// A class's permissions with a role's `allow` and `deny` applied, in catalogue order.
pub fn adjust_capabilities(base: &[Capability], allow: &[Capability], deny: &[Capability]) -> Vec<Capability> {
    Capability::ALL.iter().copied().filter(|c| (base.contains(c) || allow.contains(c)) && !deny.contains(c)).collect()
}

pub fn is_privileged(role: Role) -> bool {
    matches!(role, Role::Human | Role::Orchestrator)
}

pub fn is_member_role(role: Role) -> bool {
    MEMBER_ROLES.contains(&role)
}

pub fn can_transition(role: Role, from: Status, to: Status) -> bool {
    Actor::new("", role).may_move(from, to)
}

pub fn allowed_transitions(role: Role, from: Status) -> Vec<Status> {
    Status::ALL.iter().copied().filter(|&to| can_transition(role, from, to)).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub name: String,
    /// The class: what the history shows and what the workflow rules key on.
    pub role: Role,
    /// Permissions of a configured team role; `None` means the class's set.
    #[serde(skip)]
    pub caps: Option<Vec<Capability>>,
}

impl Actor {
    pub fn new(name: impl Into<String>, role: Role) -> Self {
        Actor { name: name.into(), role, caps: None }
    }

    /// An agent acting in a configured role: its class plus the role's own permissions.
    pub fn with_caps(name: impl Into<String>, role: Role, caps: Vec<Capability>) -> Self {
        Actor { name: name.into(), role, caps: Some(caps) }
    }

    /// People and the orchestrator hold every permission; team roles hold theirs.
    pub fn can(&self, cap: Capability) -> bool {
        if is_privileged(self.role) {
            return true;
        }
        match &self.caps {
            Some(caps) => caps.contains(&cap),
            None => class_capabilities(self.role).contains(&cap),
        }
    }

    /// Whether this actor may move a task from `from` to `to` (without `force`).
    pub fn may_move(&self, from: Status, to: Status) -> bool {
        if from == to {
            return false;
        }
        if self.role == Role::Human {
            return true;
        }
        if to == Status::Inbox {
            return false;
        }
        if self.role == Role::Orchestrator {
            return !TEAM_ONLY.contains(&to);
        }
        if ORCHESTRATOR_ONLY.contains(&to) {
            return false;
        }
        TEAM_TRANSITIONS.iter().any(|(f, t, cap)| *f == from && *t == to && self.can(*cap))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct AcceptanceCriterion {
    pub id: i64,
    pub text: String,
    pub done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
pub struct Comment {
    pub id: i64,
    pub at: String,
    pub author: String,
    pub role: Role,
    pub kind: CommentKind,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
pub struct Artifact {
    /// Per-task number (#1, #2…).
    pub id: i64,
    pub at: String,
    pub author: String,
    pub role: Role,
    pub kind: ArtifactKind,
    pub name: String,
    pub size: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
pub struct HistoryEntry {
    pub at: String,
    pub actor: String,
    pub role: Role,
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
pub struct NeedsOwner {
    pub question: String,
    pub by: String,
    pub at: String,
    /// Status to return to once the owner has answered.
    pub previous: Status,
    /// What the owner can do right in the decision box besides answering in words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<OwnerAction>,
}

/// An action an agent attaches to its question for the owner. The kind picks the buttons
/// the web shows; a free answer is always possible. New kinds are added here (with their
/// checks in `validate`) and as a card in the web, which shows a kind it does not know
/// as a plain question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[ts(export)]
pub enum OwnerAction {
    /// A question with options to pick from.
    AskOwnerQuestion { options: Vec<String> },
    /// Merge the task's request in a repository (or look at it first).
    AskForMergePr {
        /// The repository (the server picks the task's only open request when it is empty).
        #[serde(default)]
        repo: String,
        /// The request's number and page on the host, filled in by the server.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        number: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        url: Option<String>,
    },
    /// A question answered in words.
    AskFreeForm,
}

impl OwnerAction {
    pub const KINDS: &'static [&'static str] = &["ask-owner-question", "ask-for-merge-pr", "ask-free-form"];
    pub const MAX_OPTIONS: usize = 6;

    pub fn kind(&self) -> &'static str {
        match self {
            OwnerAction::AskOwnerQuestion { .. } => "ask-owner-question",
            OwnerAction::AskForMergePr { .. } => "ask-for-merge-pr",
            OwnerAction::AskFreeForm => "ask-free-form",
        }
    }

    /// Read an action as agents send it: `{"kind": "ask-owner-question", "options": […]}`
    /// (`validate` checks it when the task moves).
    pub fn parse(v: &serde_json::Value) -> Result<OwnerAction, GenieError> {
        let kind = v.get("kind").and_then(|k| k.as_str()).unwrap_or_default();
        if !Self::KINDS.contains(&kind) {
            return Err(GenieError::invalid(format!("unknown action \"{kind}\" (known: {})", Self::KINDS.join(", "))));
        }
        serde_json::from_value(v.clone()).map_err(|e| GenieError::invalid(format!("action {kind}: {e}")))
    }

    /// Tidy the action and check what it needs; the server fills in the rest (a request's number).
    pub fn validate(&mut self) -> Result<(), GenieError> {
        match self {
            OwnerAction::AskOwnerQuestion { options } => {
                let mut seen: Vec<String> = Vec::new();
                for o in options.iter().map(|o| o.trim()).filter(|o| !o.is_empty()) {
                    if o.chars().count() > 120 {
                        return Err(GenieError::invalid("ask-owner-question: an option is at most 120 characters"));
                    }
                    if !seen.iter().any(|s| s == o) {
                        seen.push(o.to_string());
                    }
                }
                if seen.len() < 2 || seen.len() > Self::MAX_OPTIONS {
                    return Err(GenieError::invalid(format!(
                        "ask-owner-question takes 2 to {} different options (the owner can always answer in words)",
                        Self::MAX_OPTIONS
                    )));
                }
                *options = seen;
            }
            OwnerAction::AskForMergePr { repo, .. } => {
                *repo = repo.trim().to_string();
                if repo.is_empty() {
                    return Err(GenieError::invalid("ask-for-merge-pr needs the repository whose request to merge"));
                }
            }
            OwnerAction::AskFreeForm => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
pub struct Blocked {
    pub reason: String,
    pub by: String,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
pub struct Worktree {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct Task {
    pub id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub task_type: TaskType,
    pub status: Status,
    /// 0 = urgent … 4 = low
    pub priority: i64,
    pub description: String,
    pub acceptance: Vec<AcceptanceCriterion>,
    /// Implementation plan (markdown), usually written by the analyst.
    pub plan: String,
    /// Running implementation notes / final summary (markdown).
    pub notes: String,
    /// How the result gets integrated, agreed before dispatch.
    pub merge_strategy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub children: Vec<String>,
    /// Tasks that must be done before this one can start.
    pub deps: Vec<String>,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    /// The person responsible for the task (a login), besides the team working on it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<Worktree>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<Blocked>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs_owner: Option<NeedsOwner>,
    pub comments: Vec<Comment>,
    pub artifacts: Vec<Artifact>,
    pub history: Vec<HistoryEntry>,
    pub created: String,
    pub updated: String,
}

/// Lightweight row for lists and boards.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct TaskSummary {
    pub id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub task_type: TaskType,
    pub status: Status,
    pub priority: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub labels: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked: Option<Blocked>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs_owner: Option<NeedsOwner>,
    pub acceptance_done: i64,
    pub acceptance_total: i64,
    pub deps: Vec<String>,
    pub open_deps: Vec<String>,
    pub children: i64,
    /// Children that are done or cancelled (epic progress).
    pub children_closed: i64,
    pub comments: i64,
    pub artifacts: i64,
    pub created: String,
    pub updated: String,
}

/// Definition of Ready: problems that prevent moving a task to `ready`.
pub fn readiness_problems(task: &Task, existing_deps: &HashSet<String>) -> Vec<String> {
    let mut problems = Vec::new();
    if task.description.trim().is_empty() {
        problems.push("description is empty".to_string());
    }
    if task.acceptance.is_empty() {
        problems.push("no acceptance criteria".to_string());
    }
    if task.task_type == TaskType::Epic {
        problems.push("epics are not handed to teams; split it into tasks".to_string());
    }
    for dep in &task.deps {
        if !existing_deps.contains(dep) {
            problems.push(format!("dependency {dep} does not exist"));
        }
    }
    problems
}

/// Optional artifact gates (`gates` in the config).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gates {
    /// Require a test-report artifact before in_progress → review.
    #[serde(default)]
    pub require_test_report: bool,
    /// Require a review artifact before review → approved.
    #[serde(default)]
    pub require_review_artifact: bool,
}

/// Definition of Done: problems that prevent moving a task to `done`.
pub fn done_problems(task: &Task, child_statuses: &[Status]) -> Vec<String> {
    let mut problems = Vec::new();
    let open: Vec<String> = task.acceptance.iter().filter(|a| !a.done).map(|a| format!("#{}", a.id)).collect();
    if !open.is_empty() {
        problems.push(format!("unchecked acceptance criteria: {}", open.join(", ")));
    }
    if task.task_type == TaskType::Epic {
        let unfinished = child_statuses.iter().filter(|s| !CLOSED.contains(s)).count();
        if unfinished > 0 {
            problems.push(format!("{unfinished} unfinished child task(s)"));
        }
    } else if task.status != Status::Approved {
        problems.push(format!("status is {}; a reviewer must approve it first", task.status));
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_table_sanity() {
        assert!(can_transition(Role::Analyst, Status::Draft, Status::Refining));
        assert!(!can_transition(Role::Analyst, Status::Refining, Status::Ready));
        assert!(can_transition(Role::Orchestrator, Status::Approved, Status::Done));
        assert!(!can_transition(Role::Orchestrator, Status::InProgress, Status::Review));
        assert!(!can_transition(Role::Orchestrator, Status::Review, Status::Approved));
        assert!(can_transition(Role::Human, Status::Review, Status::Approved));
        assert!(!can_transition(Role::Executor, Status::Review, Status::Approved));
        assert!(!can_transition(Role::Orchestrator, Status::Draft, Status::Inbox));
        assert!(can_transition(Role::Human, Status::Draft, Status::Inbox));
    }

    #[test]
    fn class_permissions_reproduce_the_old_transition_table() {
        // The table before permissions were configurable: (from, to, roles).
        let old: &[(Status, Status, &[Role])] = &[
            (Status::Draft, Status::Refining, &[Role::Analyst]),
            (Status::Ready, Status::InProgress, &[Role::Executor, Role::Analyst]),
            (Status::ChangesRequested, Status::InProgress, &[Role::Executor]),
            (Status::InProgress, Status::Review, &[Role::Executor]),
            (Status::Review, Status::ChangesRequested, &[Role::Reviewer, Role::Tester]),
            (Status::Review, Status::Approved, &[Role::Reviewer]),
        ];
        for role in MEMBER_ROLES {
            for from in Status::ALL {
                for to in Status::ALL {
                    let expected = from != to
                        && !ORCHESTRATOR_ONLY.contains(to)
                        && *to != Status::Inbox
                        && old.iter().any(|(f, t, r)| f == from && t == to && r.contains(role));
                    assert_eq!(can_transition(*role, *from, *to), expected, "{role}: {from} → {to}");
                }
            }
        }
    }

    #[test]
    fn configured_permissions_adjust_the_class() {
        let tester = class_capabilities(Role::Tester);
        let qa =
            Actor::with_caps("murphy", Role::Tester, adjust_capabilities(&tester, &[Capability::TaskCheck], &[Capability::StatusReturn]));
        assert!(qa.can(Capability::TaskCheck));
        assert!(!qa.may_move(Status::Review, Status::ChangesRequested), "denied by the role");
        let researcher = Actor::with_caps(
            "poirot",
            Role::Analyst,
            adjust_capabilities(&class_capabilities(Role::Analyst), &[Capability::StatusSubmit], &[]),
        );
        assert!(researcher.may_move(Status::InProgress, Status::Review));
        assert!(!researcher.may_move(Status::Review, Status::Done), "orchestrator-only statuses stay out of reach");
        assert!(Actor::with_caps("o", Role::Orchestrator, vec![]).can(Capability::TaskCheck), "the orchestrator is not limited by caps");
        assert_eq!("status.approve".parse::<Capability>().unwrap(), Capability::StatusApprove);
        assert_eq!("fly".parse::<Capability>().unwrap_err().to_string(), "unknown permission fly");
    }

    #[test]
    fn enums_round_trip_as_strings() {
        for s in Status::ALL {
            assert_eq!(s.as_str().parse::<Status>().unwrap(), *s);
            assert_eq!(serde_json::to_string(s).unwrap(), format!("\"{}\"", s.as_str()));
        }
        assert_eq!("test-report".parse::<ArtifactKind>().unwrap(), ArtifactKind::TestReport);
        assert_eq!("nope".parse::<Status>().unwrap_err().to_string(), "unknown status nope");
        assert_eq!("nope".parse::<ArtifactKind>().unwrap_err().to_string(), "unknown artifact kind nope");
    }
}
