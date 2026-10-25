//! What an agent may do in a repository.
//!
//! A repository has a [`Policy`] (set by a project administrator). What one agent
//! gets is the [`Effective`] policy: the repository's policy narrowed by the
//! agent's role and by what the task names. Layers only narrow. The same value
//! answers the proxy (may this ref be pushed?), the request commands (may a
//! pull/merge request be opened or merged?), the prompt and the web UI, so
//! what an agent is told is what the server enforces.

use genie_core::repos::ProjectRepo;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What may be pushed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Push {
    /// Nothing: the repository is read-only.
    None,
    /// Only the task's branches, and the result goes to a pull/merge request.
    PrOnly,
    /// The allowed branches, without a request being required.
    Branches,
    /// The allowed branches, which may include ones that are usually protected once `protected` says so.
    Direct,
}

/// Who merges a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Merge {
    Human,
    /// An agent merges (`genie pr merge`) once the task is approved and the host's conditions hold.
    AgentAfterApproval,
    /// The server merges by itself when the task is approved and the host's conditions hold.
    Auto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ChangeRequestPolicy {
    /// May an agent open a pull/merge request from its branch?
    pub open: bool,
    /// Branches a request may target; empty: the default branch only.
    pub base: Vec<String>,
    pub merge: Merge,
    /// `merge`, `squash` or `rebase` (host default when unset).
    pub method: Option<String>,
    /// Merge only when the CI passed.
    pub require_ci: bool,
    /// Approvals the host must show before an agent or the server merges.
    pub approvals: u32,
}

impl Default for ChangeRequestPolicy {
    fn default() -> Self {
        ChangeRequestPolicy { open: true, base: Vec::new(), merge: Merge::Human, method: None, require_ci: true, approvals: 1 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Policy {
    /// Clone and fetch.
    pub read: bool,
    pub push: Push,
    /// The task's branch (`{task}` is the task id).
    pub branch: String,
    /// Branches that may be pushed; unset: the task's branch and what is under it (`pr_only`, `branches`), any (`direct`).
    pub branches: Option<Vec<String>>,
    /// Never pushed, deleted or force-pushed. `{default}` is the default branch.
    pub protected: Vec<String>,
    pub force_push: bool,
    pub delete_branches: bool,
    pub change_request: ChangeRequestPolicy,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            read: true,
            push: Push::PrOnly,
            branch: "genie/{task}".into(),
            branches: None,
            protected: vec!["{default}".into()],
            force_push: false,
            delete_branches: false,
            change_request: ChangeRequestPolicy::default(),
        }
    }
}

impl Policy {
    /// The policy stored for a repository (`null` and `{}`: the defaults). Unknown keys are errors, so a typo cannot silently loosen anything.
    pub fn parse(v: &Value) -> Result<Policy, String> {
        let p: Policy = match v {
            Value::Null => Policy::default(),
            other => serde_json::from_value(other.clone()).map_err(|e| format!("policy: {e}"))?,
        };
        p.validate()?;
        Ok(p)
    }

    fn validate(&self) -> Result<(), String> {
        if !self.branch.contains("{task}") {
            return Err("policy: `branch` must contain {task}, so that tasks do not share a branch".into());
        }
        if self.branch.starts_with('-') || self.branch.contains("..") || self.branch.contains(' ') {
            return Err("policy: `branch` is not a valid branch name".into());
        }
        if self.change_request.merge != Merge::Human && !self.change_request.open {
            return Err("policy: an agent or the server can merge only a request an agent opened (`change_request.open`)".into());
        }
        if let Some(m) = &self.change_request.method
            && !matches!(m.as_str(), "merge" | "squash" | "rebase")
        {
            return Err("policy: `change_request.method` is merge, squash or rebase".into());
        }
        if self.push == Push::PrOnly && self.branches.as_ref().is_some_and(|b| b.iter().any(|p| p == "*")) {
            return Err("policy: `pr_only` with `branches: [\"*\"]` would allow every branch; list the branches".into());
        }
        Ok(())
    }
}

/// What the agent's role allows in repositories at most.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleGit {
    None,
    Read,
    Write,
}

impl RoleGit {
    /// From the role's `git` field, else from what its `files` allow.
    pub fn of(git: Option<&str>, files: crate::agent_config::FileAccess) -> RoleGit {
        use crate::agent_config::FileAccess;
        let by_files = match files {
            FileAccess::Write => RoleGit::Write,
            FileAccess::Read => RoleGit::Read,
            FileAccess::None => RoleGit::None,
        };
        let asked = match git {
            Some("write") => RoleGit::Write,
            Some("read") => RoleGit::Read,
            Some("none") => RoleGit::None,
            _ => by_files,
        };
        asked.min(by_files)
    }
}

impl PartialOrd for RoleGit {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RoleGit {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let n = |r: &RoleGit| match r {
            RoleGit::None => 0,
            RoleGit::Read => 1,
            RoleGit::Write => 2,
        };
        n(self).cmp(&n(other))
    }
}

/// The repository's policy as it applies to one agent working on one task.
#[derive(Debug, Clone)]
pub struct Effective {
    pub repo: String,
    pub mount: String,
    pub default_branch: String,
    pub task: Option<String>,
    pub read: bool,
    pub write: bool,
    /// Why the agent cannot write (when it cannot).
    pub write_denied: Option<String>,
    pub policy: Policy,
}

/// The effective policy of `repo` for an agent whose role allows `role`, working on `task`
/// (`None` for the orchestrator), given the access the task names for the repository.
pub fn effective(repo: &ProjectRepo, task: Option<&str>, role: RoleGit, task_access: Option<&str>) -> Result<Effective, String> {
    let mut policy = Policy::parse(&repo.policy).map_err(|e| format!("repository {}: {e}", repo.name))?;
    if repo.connection.is_some() {
        // A repository reached through a provider connection never sees a rewritten or deleted branch,
        // whatever its stored policy says (older policies and presets cannot widen this).
        policy.force_push = false;
        policy.delete_branches = false;
    }
    let read = policy.read && role != RoleGit::None;
    let write_denied = if !read {
        Some("this role or the policy gives no access to the repository".to_string())
    } else if role != RoleGit::Write {
        Some("the role works read-only".to_string())
    } else if repo.access != "write" {
        Some("the project allows read access only".to_string())
    } else if policy.push == Push::None {
        Some("the repository's policy forbids pushing".to_string())
    } else if task.is_none() {
        Some("there is no task to push for".to_string())
    } else if task_access != Some("write") {
        Some("the task does not name this repository for writing".to_string())
    } else {
        None
    };
    Ok(Effective {
        repo: repo.name.clone(),
        mount: repo.mount.clone(),
        default_branch: repo.default_branch.clone(),
        task: task.map(str::to_string),
        read,
        write: write_denied.is_none(),
        write_denied,
        policy,
    })
}

impl Effective {
    fn render(&self, pattern: &str) -> Option<String> {
        let mut out = pattern.replace("{default}", &self.default_branch).replace("{repo}", &self.repo);
        if out.contains("{task}") {
            out = out.replace("{task}", self.task.as_deref()?);
        }
        Some(out)
    }

    /// The task's own branch in this repository.
    pub fn task_branch(&self) -> Option<String> {
        self.render(&self.policy.branch)
    }

    /// Branch patterns the agent may push (rendered for its task).
    pub fn allowed_branches(&self) -> Vec<String> {
        match (&self.policy.branches, self.policy.push) {
            (Some(list), _) => list.iter().filter_map(|p| self.render(p)).collect(),
            (None, Push::Direct) => vec!["*".into()],
            (None, _) => self.task_branch().map(|b| vec![format!("{b}/*"), b]).unwrap_or_default(),
        }
    }

    /// Is the branch protected? When the default branch is not known yet, `main` and `master` stand in for it.
    pub fn is_protected(&self, branch: &str) -> bool {
        let defaults: Vec<&str> = if self.default_branch.is_empty() { vec!["main", "master"] } else { vec![self.default_branch.as_str()] };
        self.policy.protected.iter().any(|p| {
            if p.contains("{default}") {
                defaults.iter().any(|d| self.render(&p.replace("{default}", d)).is_some_and(|r| glob_match(&r, branch)))
            } else {
                self.render(p).is_some_and(|r| glob_match(&r, branch))
            }
        })
    }

    /// May the agent update `refname` (a push command)? `Err` is the reason, worded for the agent.
    pub fn check_push(&self, refname: &str, delete: bool) -> Result<(), String> {
        if let Some(why) = &self.write_denied {
            return Err(format!("{}: pushing is not allowed here: {why}", self.repo));
        }
        let Some(branch) = refname.strip_prefix("refs/heads/") else {
            return Err(format!("{}: only branches can be pushed, not {refname}", self.repo));
        };
        if self.is_protected(branch) {
            return Err(format!("{}: branch {branch} is protected: push to your task's branch and open a request", self.repo));
        }
        if delete && !self.policy.delete_branches {
            return Err(format!("{}: deleting branches is not allowed", self.repo));
        }
        let allowed = self.allowed_branches();
        if !allowed.iter().any(|p| glob_match(p, branch)) {
            return Err(format!("{}: branch {branch} is not allowed; push to {}", self.repo, allowed.join(" or ")));
        }
        Ok(())
    }

    /// May the agent open a request from its branch into `base`?
    pub fn check_open_request(&self, base: &str) -> Result<(), String> {
        if let Some(why) = &self.write_denied {
            return Err(format!("{}: requests are not allowed here: {why}", self.repo));
        }
        if !self.policy.change_request.open {
            return Err(format!("{}: opening requests is not allowed by the repository's policy", self.repo));
        }
        let bases = &self.policy.change_request.base;
        let ok = if bases.is_empty() {
            base == self.default_branch
        } else {
            bases.iter().filter_map(|b| self.render(b)).any(|b| glob_match(&b, base))
        };
        if ok {
            Ok(())
        } else if bases.is_empty() {
            Err(format!("{}: requests go to {} only", self.repo, self.default_branch))
        } else {
            Err(format!("{}: requests may target only {}", self.repo, bases.join(", ")))
        }
    }

    /// May an agent merge the task's request now (`approved`: the task passed review)?
    pub fn check_merge(&self, approved: bool) -> Result<(), String> {
        match self.policy.change_request.merge {
            Merge::Human => Err(format!(
                "{0}: a person merges requests here; move the task to needs_owner with `--action ask-for-merge-pr --repo {0}`",
                self.repo
            )),
            Merge::Auto => {
                Err(format!("{}: the server merges the request itself once the task is approved and the checks pass", self.repo))
            }
            Merge::AgentAfterApproval if self.write_denied.is_some() => {
                Err(format!("{}: merging is not allowed for this agent", self.repo))
            }
            Merge::AgentAfterApproval if !approved => Err(format!("{}: merge only after the reviewer approved the task", self.repo)),
            Merge::AgentAfterApproval => Ok(()),
        }
    }

    /// The rules in words, for the agent's prompt and the web UI.
    pub fn describe(&self) -> String {
        let mut out = format!("`{}` (in `{}`): ", self.repo, self.mount);
        if !self.read {
            out.push_str("no access.");
            return out;
        }
        if !self.write {
            out.push_str(&format!("read only ({}).", self.write_denied.as_deref().unwrap_or("read only")));
            return out;
        }
        let branch = self.task_branch().unwrap_or_default();
        out.push_str("read and write. ");
        match self.policy.push {
            Push::PrOnly => out.push_str(&format!(
                "Push only to `{branch}` (and branches under it); the result goes to a pull/merge request into {}.",
                self.base_text()
            )),
            Push::Branches => out.push_str(&format!(
                "Push to {}; a request is not required.",
                self.allowed_branches().iter().map(|b| format!("`{b}`")).collect::<Vec<_>>().join(", ")
            )),
            Push::Direct => out.push_str(&format!(
                "Push directly to {} (protected: {}).",
                self.allowed_branches().iter().map(|b| format!("`{b}`")).collect::<Vec<_>>().join(", "),
                self.protected_text()
            )),
            Push::None => {}
        }
        out.push_str(" Force-push and branch deletion are ");
        out.push_str(if self.policy.force_push || self.policy.delete_branches { "restricted by the policy." } else { "not allowed." });
        if self.policy.change_request.open && self.policy.push != Push::Direct {
            out.push_str(" Open the request with `genie pr open`.");
        }
        out.push_str(match self.policy.change_request.merge {
            Merge::Human => " A person merges it.",
            Merge::AgentAfterApproval => " After the reviewer approves the task and the checks pass, merge it with `genie pr merge`.",
            Merge::Auto => " The server merges it once the task is approved and the checks pass.",
        });
        out
    }

    fn base_text(&self) -> String {
        if self.policy.change_request.base.is_empty() {
            format!("`{}`", self.default_branch)
        } else {
            self.policy.change_request.base.iter().map(|b| format!("`{b}`")).collect::<Vec<_>>().join(", ")
        }
    }

    fn protected_text(&self) -> String {
        let list: Vec<String> = self.policy.protected.iter().filter_map(|p| self.render(p)).map(|p| format!("`{p}`")).collect();
        if list.is_empty() { "none".into() } else { list.join(", ") }
    }
}

/// `*` matches any run of characters (slashes included), `?` one character.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti, mut star, mut mark) = (0usize, 0usize, None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn repo(policy: Value) -> ProjectRepo {
        ProjectRepo {
            project: "shop".into(),
            name: "api".into(),
            host: "gitlab".into(),
            remote: "acme/api".into(),
            mount: "services/api".into(),
            default_branch: "main".into(),
            access: "write".into(),
            policy,
            created: String::new(),
            connection: None,
            provider_repo_id: None,
        }
    }

    fn eff(policy: Value) -> Effective {
        effective(&repo(policy), Some("S-7"), RoleGit::Write, Some("write")).unwrap()
    }

    #[test]
    fn globs() {
        assert!(glob_match("genie/*", "genie/S-7/part"));
        assert!(glob_match("genie/S-7", "genie/S-7"));
        assert!(!glob_match("genie/S-7", "genie/S-70"));
        assert!(glob_match("release/*", "release/1.0"));
        assert!(glob_match("?ain", "main"));
        assert!(glob_match("*", ""));
        assert!(!glob_match("a*b", "ac"));
    }

    #[test]
    fn by_default_only_the_task_branch_can_be_pushed() {
        let e = eff(json!({}));
        assert_eq!(e.task_branch().as_deref(), Some("genie/S-7"));
        assert!(e.check_push("refs/heads/genie/S-7", false).is_ok());
        assert!(e.check_push("refs/heads/genie/S-7/part", false).is_ok());
        assert!(e.check_push("refs/heads/main", false).unwrap_err().contains("protected"));
        assert!(e.check_push("refs/heads/genie/S-8", false).unwrap_err().contains("not allowed"));
        assert!(e.check_push("refs/heads/feature", false).is_err());
        assert!(e.check_push("refs/tags/v1", false).unwrap_err().contains("only branches"));
        assert!(e.check_push("refs/heads/genie/S-7", true).unwrap_err().contains("deleting"));
    }

    #[test]
    fn the_default_branch_stays_protected_even_when_listed_as_allowed() {
        let e = eff(json!({ "push": "branches", "branches": ["main", "genie/{task}"] }));
        assert!(e.check_push("refs/heads/main", false).is_err());
        let open = eff(json!({ "push": "direct", "protected": [] }));
        assert!(open.check_push("refs/heads/main", false).is_ok(), "direct push to main needs an explicit empty `protected`");
        let some = eff(json!({ "push": "direct", "protected": ["release/*", "{default}"] }));
        assert!(some.check_push("refs/heads/release/1", false).is_err());
        assert!(some.check_push("refs/heads/feature/x", false).is_ok());
    }

    #[test]
    fn layers_only_narrow() {
        let r = repo(json!({}));
        let reading = effective(&r, Some("S-7"), RoleGit::Read, Some("write")).unwrap();
        assert!(reading.read && !reading.write);
        assert!(reading.check_push("refs/heads/genie/S-7", false).unwrap_err().contains("read-only"));
        let unnamed = effective(&r, Some("S-7"), RoleGit::Write, None).unwrap();
        assert!(unnamed.read && !unnamed.write, "a task must name a repository to write to it");
        let orchestrator = effective(&r, None, RoleGit::Write, Some("write")).unwrap();
        assert!(!orchestrator.write);
        let mut ro = repo(json!({}));
        ro.access = "read".into();
        assert!(!effective(&ro, Some("S-7"), RoleGit::Write, Some("write")).unwrap().write);
        let none = effective(&r, Some("S-7"), RoleGit::None, Some("write")).unwrap();
        assert!(!none.read && !none.write);
        let locked = eff(json!({ "push": "none" }));
        assert!(locked.read && !locked.write);
    }

    #[test]
    fn requests_follow_the_policy() {
        let e = eff(json!({}));
        assert!(e.check_open_request("main").is_ok());
        assert!(e.check_open_request("release").unwrap_err().contains("main only"));
        assert!(e.check_merge(true).unwrap_err().contains("person merges"));
        let m = eff(json!({ "change_request": { "merge": "agent_after_approval", "base": ["main", "release/*"] } }));
        assert!(m.check_open_request("release/2").is_ok());
        assert!(m.check_merge(false).unwrap_err().contains("approved"));
        assert!(m.check_merge(true).is_ok());
        let closed = eff(json!({ "change_request": { "open": false } }));
        assert!(closed.check_open_request("main").is_err());
    }

    #[test]
    fn a_broken_policy_is_refused_not_guessed() {
        assert!(Policy::parse(&json!({ "pushh": "direct" })).is_err(), "unknown keys are errors");
        assert!(Policy::parse(&json!({ "push": "sometimes" })).is_err());
        assert!(Policy::parse(&json!({ "branch": "genie/fixed" })).is_err(), "the branch must be per task");
        assert!(Policy::parse(&json!({ "change_request": { "open": false, "merge": "auto" } })).is_err());
        assert!(Policy::parse(&json!({ "push": "pr_only", "branches": ["*"] })).is_err());
        assert!(Policy::parse(&Value::Null).is_ok());
        assert!(effective(&repo(json!({ "push": "x" })), None, RoleGit::Write, None).is_err());
    }

    #[test]
    fn a_role_limits_but_never_widens() {
        use crate::agent_config::FileAccess;
        assert_eq!(RoleGit::of(None, FileAccess::Write), RoleGit::Write);
        assert_eq!(RoleGit::of(None, FileAccess::Read), RoleGit::Read);
        assert_eq!(RoleGit::of(Some("write"), FileAccess::Read), RoleGit::Read);
        assert_eq!(RoleGit::of(Some("read"), FileAccess::Write), RoleGit::Read);
        assert_eq!(RoleGit::of(Some("write"), FileAccess::None), RoleGit::None);
    }

    #[test]
    fn the_description_tells_what_is_enforced() {
        let d = eff(json!({})).describe();
        assert!(d.contains("genie/S-7") && d.contains("A person merges it"), "{d}");
        let ro = effective(&repo(json!({})), Some("S-7"), RoleGit::Read, None).unwrap().describe();
        assert!(ro.contains("read only"), "{ro}");
    }

    #[test]
    fn a_connected_repository_never_allows_force_push_or_deleting_branches() {
        let mut r = repo(serde_json::json!({"force_push": true, "delete_branches": true}));
        let plain = effective(&r, Some("S-7"), RoleGit::Write, Some("write")).unwrap();
        assert!(plain.policy.force_push && plain.policy.delete_branches);
        r.connection = Some("gc_1".into());
        let connected = effective(&r, Some("S-7"), RoleGit::Write, Some("write")).unwrap();
        assert!(!connected.policy.force_push && !connected.policy.delete_branches);
    }
}
