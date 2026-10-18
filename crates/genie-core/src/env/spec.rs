//! Environment profiles and recipes as the project writes them, and their resolution.
//!
//! A recipe describes one stack (its tools, preparation steps, caches and commands); a
//! profile says which repositories use which recipe and what else a workspace needs. JSON is
//! read with `deny_unknown_fields`, so a misspelled key is an error rather than a silent
//! default. `resolve` checks the two together and returns every problem it finds, so a
//! profile can be fixed in one pass. It runs no command and reads no file.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// A recipe: the reusable description of one stack (a built-in or a project's recipe file).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Recipe {
    pub id: String,
    /// Bumped when the recipe changes meaning; it enters every snapshot key that uses it.
    pub revision: u32,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub params: BTreeMap<String, Param>,
    #[serde(default)]
    pub tools: BTreeMap<String, Tool>,
    #[serde(default)]
    pub caches: Vec<Cache>,
    #[serde(default)]
    pub prepare: Vec<Step>,
    #[serde(default)]
    pub commands: BTreeMap<String, String>,
    #[serde(default)]
    pub requires: Requirements,
}

/// A parameter of a recipe, substituted as `{{name}}`. Without a default a component must set it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Param {
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub description: String,
}

/// A tool the environment must provide. `version` "" or "*" accepts any; otherwise the first
/// version-like token of the probe's output must equal it or start with `"{version}."`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Tool {
    #[serde(default)]
    pub version: String,
    pub probe: String,
}

/// A download cache: a directory the package manager fills, shared by the workspaces of one project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Cache {
    pub name: String,
    /// Where the cache sits in the environment: `~/...` or an absolute path.
    pub path: String,
    #[serde(default)]
    pub integrity: Integrity,
}

/// Whether the package manager verifies downloaded content against lock-file hashes. Only a
/// verified cache may be shared more widely than a project.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Integrity {
    #[default]
    Unverified,
    Verified,
}

/// One preparation step: a command whose outputs become an immutable snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Step {
    pub id: String,
    /// Run with `sh -c`.
    pub run: String,
    /// Relative to the component's repository; empty is its root.
    #[serde(default)]
    pub cwd: String,
    /// Relative to the repository, or `~/...` for a path in the home directory.
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub inputs: Inputs,
    /// Names of profile environment variables this step sees; their values are part of the key.
    #[serde(default)]
    pub env: Vec<String>,
    /// Defaults to the profile's `network.prepare`.
    #[serde(default)]
    pub network: Option<NetworkMode>,
    /// A command that must succeed before the outputs are published.
    #[serde(default)]
    pub verify: Option<String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

/// The files a step's key and context are made of: patterns, or the whole repository.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Inputs {
    Files(Vec<String>),
    Keyword(InputsKeyword),
}

/// The keyword form of `inputs`: `"repository"` takes the whole tree of the repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputsKeyword {
    Repository,
}

impl Inputs {
    /// The input patterns, or `None` when the whole repository is the input.
    pub fn files(&self) -> Option<&[String]> {
        match self {
            Inputs::Files(files) => Some(files),
            Inputs::Keyword(_) => None,
        }
    }
}

impl Default for Inputs {
    /// A step that does not enumerate its inputs depends on the whole repository: correct, but
    /// rarely reused, so a recipe should list its files once it is tuned.
    fn default() -> Self {
        Inputs::Keyword(InputsKeyword::Repository)
    }
}

/// What a step may reach over the network. `Registries` parses but no runner supports it yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    None,
    Registries,
    Full,
}

impl NetworkMode {
    pub fn as_str(self) -> &'static str {
        match self {
            NetworkMode::None => "none",
            NetworkMode::Registries => "registries",
            NetworkMode::Full => "full",
        }
    }
}

impl std::fmt::Display for NetworkMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A profile: what one kind of workspace needs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub base: Base,
    #[serde(default)]
    pub requires: Requirements,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub components: Vec<Component>,
    #[serde(default)]
    pub services: BTreeMap<String, Service>,
    #[serde(default)]
    pub processes: BTreeMap<String, Process>,
    #[serde(default)]
    pub checks: BTreeMap<String, String>,
    #[serde(default)]
    pub network: NetworkPolicy,
    #[serde(default)]
    pub secrets: BTreeMap<String, SecretGrant>,
}

/// The image every step runs in: a reference, or packages added to it, or a repository's Containerfile.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Base {
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub packages: Vec<String>,
    /// `<repository>:<path>`; the file is read from the locked commit.
    #[serde(default)]
    pub containerfile: Option<String>,
}

/// One repository of the profile and the recipe that prepares it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Component {
    pub repo: String,
    pub recipe: String,
    /// Values of the recipe's parameters.
    #[serde(default)]
    pub params: BTreeMap<String, String>,
    /// Steps appended after the recipe's own.
    #[serde(default)]
    pub prepare: Vec<Step>,
    /// Commands that override or add to the recipe's.
    #[serde(default)]
    pub commands: BTreeMap<String, String>,
}

/// A long-running service from an image, started before the processes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Service {
    pub image: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub ready: Option<Probe>,
    #[serde(default)]
    pub data: DataPolicy,
    /// Name of a volume kept across workspaces, when `data` is `persistent`.
    #[serde(default)]
    pub volume: Option<String>,
}

/// What happens to a service's data: deleted with the service, kept with the workspace, or kept per name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataPolicy {
    Ephemeral,
    #[default]
    Workspace,
    Persistent,
}

/// A process started from a repository once its services are ready.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Process {
    /// The repository the command runs in; none is the workspace root.
    #[serde(default)]
    pub repo: Option<String>,
    pub command: String,
    /// Services and processes that must be ready first.
    #[serde(default)]
    pub after: Vec<String>,
    #[serde(default)]
    pub ready: Option<Probe>,
}

/// A readiness check: exactly one of `command`, `http` or `tcp`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Probe {
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub http: Option<String>,
    #[serde(default)]
    pub tcp: Option<String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

/// A secret delivered as a file to the named steps (`<repository>/<step>`) or to the work phase (`work`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecretGrant {
    pub steps: Vec<String>,
    /// Absolute path of the file inside the environment.
    pub file: String,
}

/// Network modes of the two phases: preparing steps and the work phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct NetworkPolicy {
    pub prepare: NetworkMode,
    pub work: NetworkMode,
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        NetworkPolicy { prepare: NetworkMode::Full, work: NetworkMode::Full }
    }
}

/// What the environment needs from its runner. Several sources merge into one set of requirements.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Requirements {
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub arch: Vec<String>,
    /// A size such as `8GiB`.
    #[serde(default)]
    pub memory: Option<String>,
    #[serde(default)]
    pub cpus: Option<f64>,
    #[serde(default)]
    pub pids: Option<u64>,
    /// A size such as `50GiB`.
    #[serde(default)]
    pub disk: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// A repository of the project, as the resolution needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    pub name: String,
    /// Where the repository sits in the workspace (`.` is the root).
    pub mount: String,
}

/// A profile with its recipes applied: what a runner is asked to prepare and run.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolved {
    pub components: Vec<ResolvedComponent>,
    pub services: BTreeMap<String, Service>,
    pub processes: BTreeMap<String, Process>,
    pub checks: BTreeMap<String, String>,
    /// The profile's requirements merged with those of the recipes.
    pub requires: Requirements,
    pub network: NetworkPolicy,
    pub env: BTreeMap<String, String>,
    pub secrets: BTreeMap<String, SecretGrant>,
    pub base: Base,
}

/// One repository with its recipe applied: tools, caches, steps and commands.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedComponent {
    pub repo: String,
    pub mount: String,
    pub recipe: String,
    pub revision: u32,
    /// SHA-256 of the recipe file, as the catalog read it.
    pub recipe_hash: String,
    pub tools: BTreeMap<String, Tool>,
    pub caches: Vec<Cache>,
    pub steps: Vec<ResolvedStep>,
    pub commands: BTreeMap<String, String>,
}

/// A preparation step with parameters substituted and defaults applied.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedStep {
    /// `<repository>/<step id>`: how grants and later steps name it.
    pub qualified: String,
    pub id: String,
    pub run: String,
    pub cwd: String,
    pub outputs: Vec<String>,
    pub inputs: Inputs,
    /// The profile's values of the variables the step names (names it has no value for are absent).
    pub env: BTreeMap<String, String>,
    pub network: NetworkMode,
    /// `(secret name, file)` granted to this step.
    pub secrets: Vec<(String, String)>,
    pub verify: Option<String>,
    pub timeout_secs: Option<u64>,
}

/// Check a profile against the recipes it names and the project's repositories.
///
/// `recipes` returns a recipe with the SHA-256 of its file. Returns the resolved environment, or
/// every problem found, each naming what is wrong.
pub fn resolve(
    profile: &Profile,
    recipes: &dyn Fn(&str) -> Option<(Recipe, String)>,
    repos: &[RepoInfo],
) -> std::result::Result<Resolved, Vec<String>> {
    let mut problems = Vec::new();
    check_base(&profile.base, repos, &mut problems);
    for name in profile.env.keys() {
        if !valid_env_name(name) {
            push(&mut problems, format!("env {name:?}: a variable name is letters, digits and underscores, not a digit first"));
        }
    }

    let mut components = Vec::new();
    let mut requirements = vec![profile.requires.clone()];
    let mut repos_seen = BTreeSet::new();
    for c in &profile.components {
        if !repos_seen.insert(c.repo.as_str()) {
            push(&mut problems, format!("component {}: the repository has more than one component", c.repo));
        }
        let Some(mount) = repos.iter().find(|r| r.name == c.repo).map(|r| r.mount.clone()) else {
            push(&mut problems, format!("component {}: the project has no repository named {}", c.repo, c.repo));
            continue;
        };
        let Some((recipe, hash)) = recipes(&c.recipe) else {
            push(&mut problems, format!("component {}: no recipe named {}", c.repo, c.recipe));
            continue;
        };
        requirements.push(recipe.requires.clone());
        components.push(resolve_component(profile, c, &recipe, hash, mount, &mut problems));
    }

    let steps: BTreeSet<String> = components.iter().flat_map(|c| c.steps.iter().map(|s| s.qualified.clone())).collect();
    check_services_and_processes(profile, repos, &mut problems);
    for (name, value) in &profile.checks {
        if !valid_name(name) {
            push(&mut problems, format!("check {name:?}: names are lowercase letters, digits and dashes"));
        }
        if value.trim().is_empty() {
            push(&mut problems, format!("check {name}: the command is empty"));
        }
    }
    for (name, grant) in &profile.secrets {
        if !valid_secret_name(name) {
            push(&mut problems, format!("secret {name:?}: a name is letters, digits, underscores and dashes"));
        }
        if !grant.file.starts_with('/') {
            push(&mut problems, format!("secret {name}: the file {:?} must be an absolute path", grant.file));
        }
        for step in &grant.steps {
            if step != "work" && !steps.contains(step) {
                push(&mut problems, format!("secret {name}: {step} is not a step of this environment (use <repository>/<step> or work)"));
            }
        }
    }
    let requires = merge_requirements(&requirements, &mut problems);

    for c in &mut components {
        for step in &mut c.steps {
            step.secrets = profile
                .secrets
                .iter()
                .filter(|(_, g)| g.steps.contains(&step.qualified))
                .map(|(name, g)| (name.clone(), g.file.clone()))
                .collect();
        }
    }

    if !problems.is_empty() {
        return Err(problems);
    }
    Ok(Resolved {
        components,
        services: profile.services.clone(),
        processes: profile.processes.clone(),
        checks: profile.checks.clone(),
        requires,
        network: profile.network,
        env: profile.env.clone(),
        secrets: profile.secrets.clone(),
        base: profile.base.clone(),
    })
}

/// Apply one recipe to one component: its parameters, tools, caches, steps and commands.
fn resolve_component(
    profile: &Profile,
    c: &Component,
    recipe: &Recipe,
    recipe_hash: String,
    mount: String,
    problems: &mut Vec<String>,
) -> ResolvedComponent {
    for name in c.params.keys() {
        if !recipe.params.contains_key(name) {
            push(problems, format!("component {}: recipe {} declares no parameter {name}", c.repo, recipe.id));
        }
    }
    let mut values = BTreeMap::new();
    for (name, param) in &recipe.params {
        if let Some(value) = c.params.get(name).or(param.default.as_ref()) {
            values.insert(name.clone(), value.clone());
        }
    }
    let mut s = Subst { who: &c.repo, values, used: BTreeSet::new(), problems };

    let mut tools = BTreeMap::new();
    for (name, tool) in &recipe.tools {
        if !valid_name(name) {
            s.fail(format!("tool {name:?}: names are lowercase letters, digits and dashes"));
        }
        let probe = s.text(&tool.probe);
        if probe.trim().is_empty() {
            s.fail(format!("tool {name} has no probe command"));
        }
        tools.insert(name.clone(), Tool { version: s.text(&tool.version), probe });
    }

    let mut caches: Vec<Cache> = Vec::new();
    for cache in &recipe.caches {
        let name = s.text(&cache.name);
        let path = s.text(&cache.path);
        if !valid_name(&name) {
            s.fail(format!("cache {name:?}: names are lowercase letters, digits and dashes"));
        }
        if !cache_path_ok(&path) {
            s.fail(format!("cache {name}: the path {path:?} must start with ~/ or / and contain no .."));
        }
        if caches.iter().any(|other| other.name == name) {
            s.fail(format!("cache {name} is defined twice"));
        }
        caches.push(Cache { name, path, integrity: cache.integrity });
    }

    let mut steps: Vec<ResolvedStep> = Vec::new();
    let mut ids = BTreeSet::new();
    for step in recipe.prepare.iter().chain(c.prepare.iter()) {
        if !valid_name(&step.id) {
            s.fail(format!("step {:?}: ids are lowercase letters, digits and dashes", step.id));
        }
        if !ids.insert(step.id.as_str()) {
            s.fail(format!("step {} is defined twice", step.id));
        }
        steps.push(resolve_step(profile, &c.repo, step, &mut s));
    }

    let mut commands = BTreeMap::new();
    for (name, command) in recipe.commands.iter().chain(c.commands.iter()) {
        let command = s.text(command);
        if command.trim().is_empty() {
            s.fail(format!("command {name} is empty"));
        }
        commands.insert(name.clone(), command);
    }

    // A value the component sets must reach a placeholder, or it does nothing and is most likely a typo.
    let used = std::mem::take(&mut s.used);
    for name in c.params.keys() {
        if recipe.params.contains_key(name) && !used.contains(name) {
            s.fail(format!("parameter {name} is not used by recipe {}", recipe.id));
        }
    }

    ResolvedComponent {
        repo: c.repo.clone(),
        mount,
        recipe: recipe.id.clone(),
        revision: recipe.revision,
        recipe_hash,
        tools,
        caches,
        steps,
        commands,
    }
}

/// One step with its placeholders filled in and its paths and names checked.
fn resolve_step(profile: &Profile, repo: &str, step: &Step, s: &mut Subst<'_>) -> ResolvedStep {
    let run = s.text(&step.run);
    let cwd = s.text(&step.cwd);
    let outputs: Vec<String> = step.outputs.iter().map(|o| s.text(o)).collect();
    let inputs = match &step.inputs {
        Inputs::Files(files) => Inputs::Files(files.iter().map(|f| s.text(f)).collect()),
        Inputs::Keyword(keyword) => Inputs::Keyword(*keyword),
    };
    let verify = step.verify.as_ref().map(|v| s.text(v));
    let id = &step.id;

    if run.trim().is_empty() {
        s.fail(format!("step {id}: the command is empty"));
    }
    if !cwd.is_empty() && !repo_path_ok(&cwd) {
        s.fail(format!("step {id}: cwd {cwd:?} must be a path inside the repository"));
    }
    for output in &outputs {
        let ok = match output.strip_prefix("~/") {
            Some(rest) => relative_ok(rest),
            None => repo_path_ok(output),
        };
        if !ok {
            s.fail(format!("step {id}: output {output:?} must be inside the repository or start with ~/ and contain no .."));
        }
    }
    if let Inputs::Files(files) = &inputs {
        for file in files {
            if !repo_path_ok(file) {
                s.fail(format!("step {id}: input {file:?} must be a pattern inside the repository"));
            }
        }
    }
    for name in &step.env {
        if !valid_env_name(name) {
            s.fail(format!("step {id}: env {name:?} is not a variable name"));
        }
    }
    if verify.as_deref().is_some_and(|v| v.trim().is_empty()) {
        s.fail(format!("step {id}: the verify command is empty"));
    }
    if step.timeout_secs == Some(0) {
        s.fail(format!("step {id}: timeoutSecs must be positive"));
    }

    let env = step.env.iter().filter_map(|n| profile.env.get(n).map(|v| (n.clone(), v.clone()))).collect();
    ResolvedStep {
        qualified: format!("{repo}/{id}"),
        id: id.clone(),
        run,
        cwd,
        outputs,
        inputs,
        env,
        network: step.network.unwrap_or(profile.network.prepare),
        secrets: Vec::new(),
        verify,
        timeout_secs: step.timeout_secs,
    }
}

/// Substitutes the parameters of one component and remembers which ones were used.
/// Problems are recorded and the text is kept as it is, so every problem is reported at once.
struct Subst<'a> {
    who: &'a str,
    values: BTreeMap<String, String>,
    used: BTreeSet<String>,
    problems: &'a mut Vec<String>,
}

impl Subst<'_> {
    fn text(&mut self, text: &str) -> String {
        match placeholders(text) {
            Ok(names) => self.used.extend(names.into_iter().map(str::to_string)),
            Err(e) => {
                self.fail(e);
                return text.to_string();
            }
        }
        match substitute(text, &self.values) {
            Ok(value) => value,
            Err(e) => {
                self.fail(e);
                text.to_string()
            }
        }
    }

    fn fail(&mut self, message: String) {
        let message = format!("component {}: {message}", self.who);
        push(self.problems, message);
    }
}

/// Records a problem once: the same mistake in several strings is reported a single time.
fn push(problems: &mut Vec<String>, message: String) {
    if !problems.contains(&message) {
        problems.push(message);
    }
}

fn check_base(base: &Base, repos: &[RepoInfo], problems: &mut Vec<String>) {
    if base.image.as_deref().is_some_and(|i| i.trim().is_empty()) {
        push(problems, "base: the image is empty".into());
    }
    if base.image.is_some() && base.containerfile.is_some() {
        push(problems, "base: set image or containerfile, not both".into());
    }
    if let Some(file) = &base.containerfile {
        let ok = match file.split_once(':') {
            Some((repo, path)) => repos.iter().any(|r| r.name == repo) && relative_ok(path),
            None => false,
        };
        if !ok {
            push(problems, format!("base: containerfile {file:?} must be <repository>:<path> of a repository in the project"));
        }
    }
    for package in &base.packages {
        if package.is_empty() || package.chars().any(char::is_whitespace) {
            push(problems, format!("base: package {package:?} must be a name without spaces"));
        }
    }
}

fn check_services_and_processes(profile: &Profile, repos: &[RepoInfo], problems: &mut Vec<String>) {
    for (name, service) in &profile.services {
        if !valid_name(name) {
            push(problems, format!("service {name:?}: names are lowercase letters, digits and dashes"));
        }
        if service.image.trim().is_empty() {
            push(problems, format!("service {name}: an image is required"));
        }
        if let Some(e) = service.ready.as_ref().and_then(probe_problem) {
            push(problems, format!("service {name}: {e}"));
        }
        if let Some(volume) = &service.volume
            && !valid_name(volume)
        {
            push(problems, format!("service {name}: volume {volume:?} names are lowercase letters, digits and dashes"));
        }
    }
    for (name, process) in &profile.processes {
        if !valid_name(name) {
            push(problems, format!("process {name:?}: names are lowercase letters, digits and dashes"));
        }
        if profile.services.contains_key(name) {
            push(problems, format!("process {name}: a service has the same name"));
        }
        if let Some(repo) = &process.repo
            && !repos.iter().any(|r| &r.name == repo)
        {
            push(problems, format!("process {name}: the project has no repository named {repo}"));
        }
        if process.command.trim().is_empty() {
            push(problems, format!("process {name}: the command is empty"));
        }
        if let Some(e) = process.ready.as_ref().and_then(probe_problem) {
            push(problems, format!("process {name}: {e}"));
        }
        for dep in &process.after {
            if !profile.services.contains_key(dep) && !profile.processes.contains_key(dep) {
                push(problems, format!("process {name}: after {dep:?} is neither a service nor a process"));
            }
        }
    }
    if let Some(cycle) = find_cycle(&profile.processes) {
        push(problems, format!("processes wait for each other: {}", cycle.join(" -> ")));
    }
}

/// The first cycle in the `after` edges between processes, as its names from the first repeated one.
fn find_cycle(processes: &BTreeMap<String, Process>) -> Option<Vec<String>> {
    fn visit(
        name: &str,
        processes: &BTreeMap<String, Process>,
        state: &mut BTreeMap<String, bool>,
        stack: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        // `false` on the stack: being visited; `true`: finished without a cycle.
        match state.get(name) {
            Some(false) => {
                let start = stack.iter().position(|n| n == name).unwrap_or(0);
                let mut cycle = stack[start..].to_vec();
                cycle.push(name.to_string());
                return Some(cycle);
            }
            Some(true) => return None,
            None => {}
        }
        state.insert(name.to_string(), false);
        stack.push(name.to_string());
        if let Some(process) = processes.get(name) {
            for dep in &process.after {
                if processes.contains_key(dep)
                    && let Some(cycle) = visit(dep, processes, state, stack)
                {
                    return Some(cycle);
                }
            }
        }
        stack.pop();
        state.insert(name.to_string(), true);
        None
    }
    let mut state = BTreeMap::new();
    for name in processes.keys() {
        if let Some(cycle) = visit(name, processes, &mut state, &mut Vec::new()) {
            return Some(cycle);
        }
    }
    None
}

/// What is wrong with a probe, if anything: exactly one kind, no empty value, a positive timeout.
pub fn probe_problem(probe: &Probe) -> Option<String> {
    let kinds = [probe.command.is_some(), probe.http.is_some(), probe.tcp.is_some()].into_iter().filter(|k| *k).count();
    if kinds != 1 {
        return Some("a readiness probe has exactly one of command, http or tcp".into());
    }
    let empty = [&probe.command, &probe.http, &probe.tcp].into_iter().flatten().any(|v| v.trim().is_empty());
    if empty {
        return Some("a readiness probe has an empty value".into());
    }
    if probe.timeout_secs == Some(0) {
        return Some("a readiness probe's timeoutSecs must be positive".into());
    }
    None
}

/// Merge the requirements of the profile and its recipes: the largest numbers and sizes, the
/// architectures every source accepts, the union of capabilities. Conflicting systems are an error.
fn merge_requirements(all: &[Requirements], problems: &mut Vec<String>) -> Requirements {
    let mut out = Requirements::default();
    for r in all {
        if let Some(os) = &r.os {
            match &out.os {
                None => out.os = Some(os.clone()),
                Some(prev) if prev != os => push(problems, format!("requires: os {prev} conflicts with {os}")),
                Some(_) => {}
            }
        }
    }
    let mut arch: Option<BTreeSet<String>> = None;
    for r in all.iter().filter(|r| !r.arch.is_empty()) {
        let set: BTreeSet<String> = r.arch.iter().cloned().collect();
        arch = Some(match arch {
            None => set,
            Some(prev) => prev.intersection(&set).cloned().collect(),
        });
    }
    if let Some(arch) = arch {
        if arch.is_empty() {
            push(problems, "requires: no architecture satisfies every recipe and the profile".into());
        }
        out.arch = arch.into_iter().collect();
    }
    out.memory = largest_size("memory", all.iter().filter_map(|r| r.memory.as_deref()), problems);
    out.disk = largest_size("disk", all.iter().filter_map(|r| r.disk.as_deref()), problems);
    for r in all {
        if let Some(cpus) = r.cpus {
            if cpus.is_finite() && cpus > 0.0 {
                out.cpus = Some(out.cpus.map_or(cpus, |prev| prev.max(cpus)));
            } else {
                push(problems, format!("requires: cpus {cpus} must be a positive number"));
            }
        }
        if let Some(pids) = r.pids {
            if pids > 0 {
                out.pids = Some(out.pids.map_or(pids, |prev| prev.max(pids)));
            } else {
                push(problems, "requires: pids must be positive".into());
            }
        }
    }
    let mut capabilities = BTreeSet::new();
    for r in all {
        for capability in &r.capabilities {
            if valid_name(capability) {
                capabilities.insert(capability.clone());
            } else {
                push(problems, format!("requires: capability {capability:?} names are lowercase letters, digits and dashes"));
            }
        }
    }
    out.capabilities = capabilities.into_iter().collect();
    out
}

/// The largest of several sizes, kept as the text it was written in.
fn largest_size<'a>(what: &str, sizes: impl Iterator<Item = &'a str>, problems: &mut Vec<String>) -> Option<String> {
    let mut best: Option<(u64, &str)> = None;
    for text in sizes {
        match parse_size(text) {
            Ok(bytes) if best.is_none_or(|(b, _)| bytes > b) => best = Some((bytes, text)),
            Ok(_) => {}
            Err(e) => push(problems, format!("requires {what}: {e}")),
        }
    }
    best.map(|(_, text)| text.to_string())
}

/// A size such as `8GiB`, `512MiB`, `100G` (the same as GiB) or a plain byte count. Units are
/// binary and case-insensitive; sizes must be positive and must not overflow.
pub fn parse_size(text: &str) -> std::result::Result<u64, String> {
    let t = text.trim();
    let bad = || format!("{text:?} is not a size: write bytes, or a binary unit such as 512MiB, 8GiB or 100G");
    let digits_end = t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
    let (digits, unit) = t.split_at(digits_end);
    if digits.is_empty() {
        return Err(bad());
    }
    let number: u64 = digits.parse().map_err(|_| bad())?;
    let multiplier: u64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kib" => 1 << 10,
        "m" | "mib" => 1 << 20,
        "g" | "gib" => 1 << 30,
        "t" | "tib" => 1 << 40,
        "gb" | "mb" | "kb" | "tb" => {
            return Err(format!("{text:?} is ambiguous: write a binary unit such as {digits}{}iB", unit.trim()[..1].to_ascii_uppercase()));
        }
        _ => return Err(bad()),
    };
    let bytes = number.checked_mul(multiplier).ok_or_else(|| format!("{text:?} is too large"))?;
    if bytes == 0 {
        return Err(format!("{text:?}: a size must be positive"));
    }
    Ok(bytes)
}

/// A name of a profile, recipe, service, process or check: lowercase letters, digits and dashes,
/// starting with a letter or digit, at most 63 characters. Names become directory and container names.
pub fn valid_name(s: &str) -> bool {
    s.len() <= 63
        && s.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A shell-style environment variable name: letters, digits and underscores, not a digit first.
pub fn valid_env_name(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A secret's name: letters, digits, underscores and dashes, not starting with a dash, 1 to 64 characters.
pub fn valid_secret_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && !s.starts_with('-') && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A path relative to the repository (or to a directory in the home): no leading `/`, no `..`.
fn relative_ok(path: &str) -> bool {
    !path.is_empty() && !path.starts_with('/') && !path.split('/').any(|part| part == "..")
}

/// A relative path that does not start with `~`, which would name a home directory.
fn repo_path_ok(path: &str) -> bool {
    relative_ok(path) && !path.starts_with('~')
}

/// A cache lives in the home (`~/...`) or at an absolute path, with no `..`.
fn cache_path_ok(path: &str) -> bool {
    match path.strip_prefix("~/") {
        Some(rest) => relative_ok(rest),
        None => path.strip_prefix('/').is_some_and(relative_ok),
    }
}

/// The `{{name}}` placeholders of `text`, in order. An unclosed or empty placeholder is an error.
fn placeholders(text: &str) -> std::result::Result<Vec<&str>, String> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let end = after.find("}}").ok_or_else(|| format!("unclosed {{{{ in {text:?}"))?;
        let name = after[..end].trim();
        if name.is_empty() {
            return Err(format!("empty {{{{}}}} in {text:?}"));
        }
        names.push(name);
        rest = &after[end + 2..];
    }
    Ok(names)
}

/// `text` with every `{{name}}` replaced by its value. A name without a value is an error that
/// names it: the recipe declares no such parameter, or it has no default and nothing sets it.
pub fn substitute(text: &str, values: &BTreeMap<String, String>) -> std::result::Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after.find("}}").ok_or_else(|| format!("unclosed {{{{ in {text:?}"))?;
        let name = after[..end].trim();
        if name.is_empty() {
            return Err(format!("empty {{{{}}}} in {text:?}"));
        }
        let value = values.get(name).ok_or_else(|| {
            format!("parameter {name} has no value: the recipe does not declare it, or it has no default and the component does not set it")
        })?;
        out.push_str(value);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn step(id: &str, run: &str) -> Step {
        Step {
            id: id.into(),
            run: run.into(),
            cwd: String::new(),
            outputs: Vec::new(),
            inputs: Inputs::default(),
            env: Vec::new(),
            network: None,
            verify: None,
            timeout_secs: None,
        }
    }

    fn recipe(id: &str) -> Recipe {
        Recipe {
            id: id.into(),
            revision: 1,
            description: String::new(),
            params: BTreeMap::new(),
            tools: BTreeMap::new(),
            caches: Vec::new(),
            prepare: Vec::new(),
            commands: BTreeMap::new(),
            requires: Requirements::default(),
        }
    }

    fn component(repo: &str, recipe: &str) -> Component {
        Component { repo: repo.into(), recipe: recipe.into(), params: BTreeMap::new(), prepare: Vec::new(), commands: BTreeMap::new() }
    }

    fn repos() -> Vec<RepoInfo> {
        vec![RepoInfo { name: "api".into(), mount: "api".into() }, RepoInfo { name: "web".into(), mount: "web".into() }]
    }

    /// Recipes by id; each comes with a made-up file hash.
    fn lookup(recipes: Vec<Recipe>) -> impl Fn(&str) -> Option<(Recipe, String)> {
        move |id: &str| recipes.iter().find(|r| r.id == id).map(|r| (r.clone(), format!("hash-{}", r.id)))
    }

    fn resolve_with(profile: &Profile, recipes: Vec<Recipe>) -> std::result::Result<Resolved, Vec<String>> {
        resolve(profile, &lookup(recipes), &repos())
    }

    /// The problems of a profile that must not resolve, failing the test otherwise.
    fn problems(profile: &Profile, recipes: Vec<Recipe>) -> Vec<String> {
        match resolve_with(profile, recipes) {
            Ok(r) => panic!("the profile should not resolve: {r:?}"),
            Err(p) => p,
        }
    }

    /// The problems that mention `needle`: a test checks that the problem exists, not the whole list.
    fn has(problems: &[String], needle: &str) -> bool {
        problems.iter().any(|p| p.contains(needle))
    }

    fn one_component_profile(recipe_id: &str) -> Profile {
        Profile { components: vec![component("api", recipe_id)], ..Default::default() }
    }

    #[test]
    fn a_recipe_and_profile_read_from_json_use_camel_case_and_defaults() {
        let recipe: Recipe = serde_json::from_str(
            r#"{
                "id": "node-npm", "revision": 2,
                "params": { "node": { "default": "22" }, "dir": {} },
                "tools": { "node": { "probe": "node --version", "version": "{{node}}" } },
                "caches": [{ "name": "npm", "path": "~/.npm", "integrity": "verified" }],
                "prepare": [{ "id": "install", "run": "npm ci", "outputs": ["{{dir}}node_modules"],
                              "inputs": ["package.json", "package-lock.json"], "timeoutSecs": 600 }],
                "commands": { "test": "npm test" },
                "requires": { "arch": ["x86_64"], "memory": "4GiB" }
            }"#,
        )
        .unwrap();
        assert_eq!(recipe.revision, 2);
        assert_eq!(recipe.params["dir"].default, None);
        assert_eq!(recipe.caches[0].integrity, Integrity::Verified);
        assert_eq!(recipe.prepare[0].timeout_secs, Some(600));
        assert_eq!(recipe.prepare[0].inputs.files().map(|f| f.len()), Some(2));
        assert_eq!(recipe.requires.memory.as_deref(), Some("4GiB"));

        let profile: Profile = serde_json::from_str(r#"{"components": [{"repo": "api", "recipe": "custom"}]}"#).unwrap();
        assert_eq!(profile.network, NetworkPolicy { prepare: NetworkMode::Full, work: NetworkMode::Full });
        assert_eq!(profile.components[0].recipe, "custom");
        let partial: Profile = serde_json::from_str(r#"{"network": {"work": "none"}}"#).unwrap();
        assert_eq!(
            partial.network,
            NetworkPolicy { prepare: NetworkMode::Full, work: NetworkMode::None },
            "a partial policy keeps the other defaults"
        );
    }

    #[test]
    fn unknown_keys_are_rejected_at_every_level() {
        assert!(serde_json::from_str::<Recipe>(r#"{"id": "x", "revision": 1, "stpes": []}"#).is_err());
        assert!(serde_json::from_str::<Profile>(r#"{"comonents": []}"#).is_err());
        assert!(
            serde_json::from_str::<Step>(r#"{"id": "a", "run": "x", "scope": "project"}"#).is_err(),
            "caches have a scope, steps do not"
        );
        assert!(serde_json::from_str::<Probe>(r#"{"htp": "http://x"}"#).is_err());
        assert!(serde_json::from_str::<Requirements>(r#"{"memroy": "1GiB"}"#).is_err());
        assert!(serde_json::from_str::<NetworkPolicy>(r#"{"prepare": "full", "mode": "x"}"#).is_err());
        assert!(serde_json::from_str::<Inputs>(r#""everything""#).is_err());
        assert_eq!(serde_json::from_str::<Inputs>(r#""repository""#).unwrap(), Inputs::Keyword(InputsKeyword::Repository));
    }

    #[test]
    fn placeholders_are_substituted_from_recipe_defaults_and_component_values() {
        let mut rec = recipe("node-npm");
        rec.params.insert("node".into(), Param { default: Some("22".into()), description: String::new() });
        rec.params.insert("dir".into(), Param { default: Some(String::new()), description: String::new() });
        rec.tools.insert("node".into(), Tool { version: "{{node}}".into(), probe: "node --version".into() });
        rec.prepare.push(Step {
            outputs: vec!["{{dir}}node_modules".into()],
            inputs: Inputs::Files(vec!["{{dir}}package.json".into()]),
            ..step("install", "npm ci --prefix {{dir}}")
        });
        rec.commands.insert("build".into(), "npm run build --if-present".into());
        let mut c = component("web", "node-npm");
        c.params.insert("node".into(), "20".into());
        c.params.insert("dir".into(), "front/".into());
        let profile = Profile { components: vec![c], ..Default::default() };

        let resolved = resolve_with(&profile, vec![rec]).unwrap();
        let comp = &resolved.components[0];
        assert_eq!(comp.tools["node"].version, "20", "the component's value wins over the default");
        let install = &comp.steps[0];
        assert_eq!(install.qualified, "web/install");
        assert_eq!(install.run, "npm ci --prefix front/");
        assert_eq!(install.outputs, vec!["front/node_modules".to_string()]);
        assert_eq!(install.inputs, Inputs::Files(vec!["front/package.json".into()]));
        assert_eq!(comp.recipe_hash, "hash-node-npm");
        assert_eq!(comp.mount, "web");
    }

    #[test]
    fn a_placeholder_without_a_value_names_the_component_and_the_parameter() {
        let mut rec = recipe("needs-node");
        rec.params.insert("node".into(), Param { default: None, description: String::new() });
        rec.tools.insert("node".into(), Tool { version: "{{node}}".into(), probe: "node --version".into() });
        let p = problems(&one_component_profile("needs-node"), vec![rec.clone()]);
        assert!(has(&p, "component api: parameter node has no value"), "{p:?}");

        // An undeclared placeholder is the same error, for the name the recipe never declared.
        let mut bare = recipe("typo");
        bare.commands.insert("build".into(), "make {{target}}".into());
        let p = problems(&one_component_profile("typo"), vec![bare]);
        assert!(has(&p, "component api: parameter target has no value"), "{p:?}");

        // Set by the component, the same placeholder resolves.
        let mut c = component("api", "needs-node");
        c.params.insert("node".into(), "22".into());
        let profile = Profile { components: vec![c], ..Default::default() };
        assert!(resolve_with(&profile, vec![rec]).is_ok());
    }

    #[test]
    fn a_component_parameter_the_recipe_does_not_declare_or_use_is_an_error() {
        let mut rec = recipe("plain");
        rec.commands.insert("build".into(), "make".into());
        let mut c = component("api", "plain");
        c.params.insert("ghost".into(), "1".into());
        let p = problems(&Profile { components: vec![c], ..Default::default() }, vec![rec.clone()]);
        assert!(has(&p, "recipe plain declares no parameter ghost"), "{p:?}");

        let mut declared = recipe("declared");
        declared.params.insert("node".into(), Param { default: Some("22".into()), description: String::new() });
        declared.commands.insert("build".into(), "make".into());
        let mut c = component("api", "declared");
        c.params.insert("node".into(), "20".into());
        let p = problems(&Profile { components: vec![c], ..Default::default() }, vec![declared]);
        assert!(has(&p, "parameter node is not used by recipe declared"), "{p:?}");
    }

    #[test]
    fn a_malformed_placeholder_is_reported_not_silently_kept() {
        let mut rec = recipe("broken");
        rec.commands.insert("build".into(), "make {{target".into());
        let p = problems(&one_component_profile("broken"), vec![rec]);
        assert!(has(&p, "unclosed {{"), "{p:?}");
        assert_eq!(placeholders("a {{ b }} c {{d}}").unwrap(), vec!["b", "d"]);
        assert!(placeholders("{{}}").is_err());
        assert!(substitute("{{}}", &BTreeMap::new()).is_err());
    }

    #[test]
    fn substitute_replaces_every_occurrence_and_keeps_other_text() {
        let v = values(&[("node", "22"), ("dir", "web/")]);
        assert_eq!(substitute("{{dir}}node_modules {{ node }} {{node}}", &v).unwrap(), "web/node_modules 22 22");
        assert_eq!(substitute("no placeholders", &v).unwrap(), "no placeholders");
        assert_eq!(substitute("", &v).unwrap(), "");
        let err = substitute("{{missing}}", &v).unwrap_err();
        assert!(err.contains("parameter missing has no value"), "{err}");
    }

    #[test]
    fn a_custom_component_runs_only_the_profile_steps() {
        let mut c = component("api", "custom");
        c.prepare.push(Step { outputs: vec!["build".into()], ..step("bootstrap", "./scripts/bootstrap.sh") });
        c.commands.insert("test".into(), "./scripts/test.sh".into());
        let profile = Profile { components: vec![c], ..Default::default() };
        let custom = recipe("custom");
        let resolved = resolve_with(&profile, vec![custom]).unwrap();
        let comp = &resolved.components[0];
        assert_eq!(comp.steps.len(), 1);
        assert_eq!(comp.steps[0].qualified, "api/bootstrap");
        assert_eq!(comp.steps[0].inputs, Inputs::default(), "custom steps take the whole repository");
        assert_eq!(comp.commands["test"], "./scripts/test.sh");
        assert_eq!(comp.caches.len(), 0);
    }

    #[test]
    fn recipe_steps_come_before_profile_steps_and_commands_are_overridden() {
        let mut rec = recipe("ordered");
        rec.prepare.push(step("fetch", "fetch"));
        rec.commands.insert("build".into(), "recipe build".into());
        rec.commands.insert("test".into(), "recipe test".into());
        let mut c = component("api", "ordered");
        c.prepare.push(step("extra", "extra"));
        c.commands.insert("build".into(), "component build".into());
        let resolved = resolve_with(&Profile { components: vec![c], ..Default::default() }, vec![rec]).unwrap();
        let comp = &resolved.components[0];
        assert_eq!(comp.steps.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["fetch", "extra"]);
        assert_eq!(comp.commands["build"], "component build");
        assert_eq!(comp.commands["test"], "recipe test");
    }

    #[test]
    fn step_defaults_and_profile_env_values_resolve() {
        let mut profile = one_component_profile("custom");
        profile.env.insert("NODE_ENV".into(), "production".into());
        profile.network.prepare = NetworkMode::None;
        let mut s = step("install", "make");
        s.env = vec!["NODE_ENV".into(), "UNSET".into()];
        profile.components[0].prepare.push(s);
        let mut explicit = step("fetch", "make fetch");
        explicit.network = Some(NetworkMode::Full);
        profile.components[0].prepare.push(explicit);
        let resolved = resolve_with(&profile, vec![recipe("custom")]).unwrap();
        let steps = &resolved.components[0].steps;
        assert_eq!(steps[0].network, NetworkMode::None, "defaults to the profile's prepare network");
        assert_eq!(steps[1].network, NetworkMode::Full);
        assert_eq!(steps[0].env.get("NODE_ENV").map(String::as_str), Some("production"));
        assert!(!steps[0].env.contains_key("UNSET"), "a name without a profile value is not set");
    }

    #[test]
    fn secrets_are_granted_to_the_steps_that_name_them() {
        let mut profile = one_component_profile("custom");
        profile.components[0].prepare.push(step("install", "make"));
        profile.components[0].prepare.push(step("build", "make build"));
        profile.secrets.insert("npm_token".into(), SecretGrant { steps: vec!["api/install".into()], file: "/run/secrets/npm".into() });
        profile.secrets.insert("deploy".into(), SecretGrant { steps: vec!["work".into()], file: "/run/secrets/deploy".into() });
        let resolved = resolve_with(&profile, vec![recipe("custom")]).unwrap();
        let steps = &resolved.components[0].steps;
        assert_eq!(steps[0].secrets, vec![("npm_token".to_string(), "/run/secrets/npm".to_string())]);
        assert!(steps[1].secrets.is_empty());
        assert!(resolved.secrets.contains_key("deploy"), "work grants stay in the resolved profile");
    }

    #[test]
    fn requirements_merge_to_the_largest_numbers_and_common_architectures() {
        let mut rec = recipe("big");
        rec.requires = Requirements {
            arch: vec!["x86_64".into(), "aarch64".into()],
            memory: Some("12GiB".into()),
            cpus: Some(6.0),
            pids: Some(9000),
            disk: Some("1TiB".into()),
            capabilities: vec!["gpu".into()],
            ..Default::default()
        };
        let mut profile = one_component_profile("big");
        profile.requires = Requirements {
            arch: vec!["aarch64".into()],
            memory: Some("8GiB".into()),
            cpus: Some(2.0),
            capabilities: vec!["nested".into(), "gpu".into()],
            ..Default::default()
        };
        let resolved = resolve_with(&profile, vec![rec]).unwrap();
        let r = &resolved.requires;
        assert_eq!(r.arch, vec!["aarch64".to_string()], "the intersection of the lists");
        assert_eq!(r.memory.as_deref(), Some("12GiB"));
        assert_eq!(r.cpus, Some(6.0));
        assert_eq!(r.pids, Some(9000));
        assert_eq!(r.disk.as_deref(), Some("1TiB"));
        assert_eq!(r.capabilities, vec!["gpu".to_string(), "nested".to_string()], "the union");
    }

    #[test]
    fn a_profile_without_components_resolves_to_nothing_to_prepare() {
        let profile = Profile { description: "bare".into(), ..Default::default() };
        let resolved = resolve_with(&profile, vec![]).unwrap();
        assert!(resolved.components.is_empty());
        assert_eq!(resolved.network, NetworkPolicy::default());
    }

    #[test]
    fn every_validation_problem_is_reported_together() {
        let mut bad_step = step("Bad Id", "");
        bad_step.cwd = "../escape".into();
        bad_step.outputs = vec!["/abs".into(), "~/ok".into(), "../up".into(), "x/../y".into()];
        bad_step.inputs = Inputs::Files(vec!["/etc/passwd".into(), "~/x".into()]);
        bad_step.verify = Some(" ".into());
        bad_step.timeout_secs = Some(0);
        bad_step.env = vec!["1BAD".into()];
        let mut profile = Profile {
            components: vec![component("nope", "custom"), component("api", "missing-recipe")],
            base: Base { image: Some("debian".into()), containerfile: Some("ghost:Containerfile".into()), packages: vec![] },
            ..Default::default()
        };
        profile.components.push(Component { prepare: vec![bad_step, step("dup", "x"), step("dup", "y")], ..component("web", "custom") });
        profile.components.push(component("web", "custom"));
        profile.env.insert("bad-name".into(), "x".into());
        profile.services.insert(
            "Db".into(),
            Service {
                image: " ".into(),
                env: BTreeMap::new(),
                command: vec![],
                ready: Some(Probe { command: Some("a".into()), http: Some("b".into()), tcp: None, timeout_secs: None }),
                data: DataPolicy::Workspace,
                volume: Some("Bad Vol".into()),
            },
        );
        let p = problems(&profile, vec![recipe("custom")]);
        for needle in [
            "no repository named nope",
            "no recipe named missing-recipe",
            "more than one component",
            "base: set image or containerfile, not both",
            "containerfile \"ghost:Containerfile\" must be <repository>:<path>",
            "env \"bad-name\"",
            "service \"Db\": names are lowercase",
            "service Db: an image is required",
            "a readiness probe has exactly one",
            "\"Bad Id\": ids are lowercase",
            "step dup is defined twice",
            "cwd \"../escape\"",
            "output \"/abs\"",
            "output \"../up\"",
            "output \"x/../y\"",
            "input \"/etc/passwd\"",
            "input \"~/x\"",
            "the verify command is empty",
            "timeoutSecs must be positive",
            "env \"1BAD\" is not a variable name",
            "volume \"Bad Vol\"",
        ] {
            assert!(has(&p, needle), "missing problem {needle:?} in {p:#?}");
        }
        assert!(!has(&p, "output \"~/ok\""), "a home output is valid");
    }

    #[test]
    fn base_containerfile_must_name_a_project_repository_and_one_base_source() {
        let mut profile =
            Profile { base: Base { containerfile: Some("api:docker/Containerfile".into()), ..Default::default() }, ..Default::default() };
        assert!(resolve_with(&profile, vec![]).is_ok());
        profile.base.containerfile = Some("ghost:Containerfile".into());
        assert!(has(&problems(&profile, vec![]), "must be <repository>:<path>"));
        profile.base = Base { packages: vec!["libpq dev".into(), "".into()], ..Default::default() };
        let p = problems(&profile, vec![]);
        assert!(has(&p, "package \"libpq dev\" must be a name without spaces"), "{p:?}");
        assert!(has(&p, "package \"\" must be"), "{p:?}");
        profile.base = Base { image: Some(" ".into()), ..Default::default() };
        assert!(has(&problems(&profile, vec![]), "base: the image is empty"));
    }

    #[test]
    fn services_and_processes_are_checked_and_ordered_without_cycles() {
        let probe = |http: &str| Probe { command: None, http: Some(http.into()), tcp: None, timeout_secs: Some(5) };
        let mut profile = Profile::default();
        profile.services.insert(
            "db".into(),
            Service {
                image: "postgres:16".into(),
                env: BTreeMap::new(),
                command: vec![],
                ready: Some(Probe { command: Some("pg_isready".into()), http: None, tcp: None, timeout_secs: None }),
                data: DataPolicy::Persistent,
                volume: Some("pgdata".into()),
            },
        );
        profile.processes.insert(
            "api".into(),
            Process {
                repo: Some("api".into()),
                command: "cargo run".into(),
                after: vec!["db".into(), "web".into()],
                ready: Some(probe("http://127.0.0.1:8080/health")),
            },
        );
        profile.processes.insert("web".into(), Process { repo: None, command: "npm start".into(), after: vec!["api".into()], ready: None });
        let p = problems(&profile, vec![]);
        assert!(has(&p, "processes wait for each other: api -> web -> api"), "{p:?}");

        profile.processes.get_mut("web").unwrap().after = vec!["db".into()];
        profile.processes.get_mut("api").unwrap().after = vec!["db".into(), "missing".into()];
        profile.processes.get_mut("api").unwrap().repo = Some("nowhere".into());
        let p = problems(&profile, vec![]);
        assert!(has(&p, "after \"missing\" is neither a service nor a process"), "{p:?}");
        assert!(has(&p, "process api: the project has no repository named nowhere"), "{p:?}");
        assert!(!has(&p, "wait for each other"));

        profile.processes.get_mut("api").unwrap().after = vec!["api".into()];
        assert!(has(&problems(&profile, vec![]), "processes wait for each other: api -> api"));

        let mut ok = Profile::default();
        ok.services.insert(
            "db".into(),
            Service {
                image: "postgres:16".into(),
                env: BTreeMap::new(),
                command: vec![],
                ready: None,
                data: DataPolicy::default(),
                volume: None,
            },
        );
        ok.processes.insert("api".into(), Process { repo: None, command: "run".into(), after: vec!["db".into()], ready: None });
        resolve_with(&ok, vec![]).unwrap();
        assert_eq!(DataPolicy::default(), DataPolicy::Workspace);

        let mut same = Profile::default();
        same.services.insert(
            "api".into(),
            Service { image: "x".into(), env: BTreeMap::new(), command: vec![], ready: None, data: DataPolicy::Ephemeral, volume: None },
        );
        same.processes.insert("api".into(), Process { repo: None, command: "run".into(), after: vec![], ready: None });
        assert!(has(&problems(&same, vec![]), "a service has the same name"));
    }

    #[test]
    fn probes_have_exactly_one_kind_with_a_value_and_a_positive_timeout() {
        let p = |command: Option<&str>, http: Option<&str>, tcp: Option<&str>, timeout| Probe {
            command: command.map(String::from),
            http: http.map(String::from),
            tcp: tcp.map(String::from),
            timeout_secs: timeout,
        };
        assert_eq!(probe_problem(&p(Some("true"), None, None, None)), None);
        assert_eq!(probe_problem(&p(None, None, Some("db:5432"), Some(3))), None);
        assert!(probe_problem(&p(None, None, None, None)).unwrap().contains("exactly one"));
        assert!(probe_problem(&p(Some("a"), None, Some("b:1"), None)).unwrap().contains("exactly one"));
        assert!(probe_problem(&p(Some(" "), None, None, None)).unwrap().contains("empty value"));
        assert!(probe_problem(&p(None, Some("http://x"), None, Some(0))).unwrap().contains("positive"));
    }

    #[test]
    fn requirement_conflicts_and_bad_sizes_are_reported() {
        let mut rec = recipe("bad");
        rec.requires =
            Requirements { os: Some("windows".into()), memory: Some("8GB".into()), cpus: Some(0.0), pids: Some(0), ..Default::default() };
        let mut profile = one_component_profile("bad");
        profile.requires.os = Some("linux".into());
        profile.requires.arch = vec!["x86_64".into()];
        profile.requires.capabilities = vec!["Not Valid".into()];
        let mut other = recipe("other");
        other.requires.arch = vec!["aarch64".into()];
        profile.components.push(component("web", "other"));
        let p = problems(&profile, vec![rec, other]);
        for needle in [
            "os linux conflicts with windows",
            "8GB\" is ambiguous",
            "cpus 0 must be a positive number",
            "pids must be positive",
            "no architecture satisfies",
            "capability \"Not Valid\"",
        ] {
            assert!(has(&p, needle), "missing {needle:?} in {p:#?}");
        }
    }

    #[test]
    fn secrets_must_have_valid_names_absolute_files_and_known_steps() {
        let mut profile = one_component_profile("custom");
        profile.components[0].prepare.push(step("install", "make"));
        profile.secrets.insert("bad name".into(), SecretGrant { steps: vec![], file: "/x".into() });
        profile
            .secrets
            .insert("token".into(), SecretGrant { steps: vec!["api/install".into(), "api/ghost".into()], file: "relative".into() });
        let p = problems(&profile, vec![recipe("custom")]);
        assert!(has(&p, "secret \"bad name\""), "{p:?}");
        assert!(has(&p, "file \"relative\" must be an absolute path"), "{p:?}");
        assert!(has(&p, "api/ghost is not a step of this environment"), "{p:?}");
        assert!(!has(&p, "api/install"), "an existing step is not a problem");
    }

    #[test]
    fn a_repository_with_two_components_and_a_bad_recipe_are_reported() {
        let mut bad = recipe("bad-tools");
        bad.tools.insert("node".into(), Tool { version: String::new(), probe: " ".into() });
        bad.caches.push(Cache { name: "Bad Cache".into(), path: "relative".into(), integrity: Integrity::Unverified });
        bad.caches.push(Cache { name: "Bad Cache".into(), path: "~/.x".into(), integrity: Integrity::Unverified });
        bad.commands.insert("build".into(), "".into());
        let profile = Profile { components: vec![component("api", "bad-tools"), component("api", "custom")], ..Default::default() };
        let p = problems(&profile, vec![bad]);
        for needle in [
            "the repository has more than one component",
            "tool node has no probe command",
            "cache \"Bad Cache\": names are",
            "cache Bad Cache: the path \"relative\"",
            "cache Bad Cache is defined twice",
            "command build is empty",
        ] {
            assert!(has(&p, needle), "missing {needle:?} in {p:#?}");
        }
    }

    #[test]
    fn parse_size_reads_binary_units_and_bytes_and_refuses_the_rest() {
        assert_eq!(parse_size("8GiB").unwrap(), 8 << 30);
        assert_eq!(parse_size("512MiB").unwrap(), 512 << 20);
        assert_eq!(parse_size("100G").unwrap(), 100 << 30);
        assert_eq!(parse_size("50gib").unwrap(), 50 << 30, "case-insensitive");
        assert_eq!(parse_size(" 4 KiB ").unwrap(), 4096);
        assert_eq!(parse_size("2T").unwrap(), 2 << 40);
        assert_eq!(parse_size("1024").unwrap(), 1024);
        assert_eq!(parse_size("1b").unwrap(), 1);
        for bad in ["", "GiB", "1.5GiB", "8 GiB x", "-1G", "0", "0G", "8XB", "100GB", "99999999999999999999T", "18446744073709551615K"] {
            assert!(parse_size(bad).is_err(), "{bad:?} should be refused");
        }
        let msg = parse_size("8GB").unwrap_err();
        assert!(msg.contains("8GiB"), "the error suggests the binary unit: {msg}");
    }

    #[test]
    fn names_follow_their_rules() {
        assert!(valid_name("default") && valid_name("rust-cargo") && valid_name("a1") && valid_name("1a"));
        assert!(
            !valid_name("")
                && !valid_name("-a")
                && !valid_name("A")
                && !valid_name("a_b")
                && !valid_name("a/b")
                && !valid_name(&"a".repeat(64))
        );
        assert!(
            valid_env_name("NODE_ENV") && valid_env_name("_x") && !valid_env_name("1X") && !valid_env_name("a-b") && !valid_env_name("")
        );
        assert!(
            valid_secret_name("npm_token")
                && valid_secret_name("DEPLOY-KEY")
                && !valid_secret_name("-x")
                && !valid_secret_name("a b")
                && !valid_secret_name("")
        );
        assert!(!valid_secret_name(&"a".repeat(65)));
    }

    #[test]
    fn a_cycle_of_three_is_reported_from_its_first_name() {
        let mut profile = Profile::default();
        for (name, dep) in [("a", "b"), ("b", "c"), ("c", "a")] {
            profile.processes.insert(name.into(), Process { repo: None, command: "x".into(), after: vec![dep.into()], ready: None });
        }
        assert!(has(&problems(&profile, vec![]), "processes wait for each other: a -> b -> c -> a"));
    }

    #[test]
    fn a_repository_path_must_stay_inside_the_repository() {
        assert!(relative_ok("a/b") && relative_ok("./a") && relative_ok("a..b"));
        assert!(!relative_ok("") && !relative_ok("/a") && !relative_ok("a/../b") && !relative_ok(".."));
        assert!(repo_path_ok("a") && !repo_path_ok("~/a"));
        assert!(
            cache_path_ok("~/.cargo/registry")
                && cache_path_ok("/var/cache/x")
                && !cache_path_ok("/")
                && !cache_path_ok("~/../x")
                && !cache_path_ok("rel")
        );
    }

    #[test]
    fn requirements_with_no_sources_stay_empty() {
        let mut problems = Vec::new();
        let merged = merge_requirements(&[Requirements::default()], &mut problems);
        assert_eq!(merged, Requirements::default());
        assert!(problems.is_empty());
    }
}
