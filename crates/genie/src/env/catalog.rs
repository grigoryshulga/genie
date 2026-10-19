//! Recipes and profiles of a project, as files on disk.
//!
//! Built-in recipes ship in `config/environments/recipes/` and are embedded in the binary. A
//! project's recipe file `<data>/environments/<project>/recipes/<id>.json` replaces the built-in
//! recipe of the same id; its content must declare that id. Profiles live beside them in
//! `<data>/environments/<project>/profiles/<name>.json`. Files are read as bytes, so the SHA-256
//! of a file is the identity that snapshot keys and the lock record.
//!
//! Problems with project files are reported, never skipped: a broken recipe file fails every
//! resolution of the project until it is fixed.

use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use genie_core::env::key::sha256_hex;
use genie_core::env::spec::{self, Profile, Recipe, RepoInfo, Resolved, valid_name};
use genie_core::server_db::valid_slug;

/// The built-in recipes, by id. Each file's id must equal its name (checked when it is read).
const BUILTIN: &[(&str, &str)] = &[
    ("rust-cargo", include_str!("../../../../config/environments/recipes/rust-cargo.json")),
    ("node-npm", include_str!("../../../../config/environments/recipes/node-npm.json")),
    ("python-uv", include_str!("../../../../config/environments/recipes/python-uv.json")),
    ("jvm-gradle", include_str!("../../../../config/environments/recipes/jvm-gradle.json")),
    ("custom", include_str!("../../../../config/environments/recipes/custom.json")),
];

/// Where a recipe came from: the release or the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Builtin,
    Project,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Builtin => "builtin",
            Source::Project => "project",
        }
    }
}

/// A recipe as the catalog serves it: its content, the SHA-256 of the file it was read from, and its source.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogRecipe {
    pub recipe: Recipe,
    pub hash: String,
    pub source: Source,
}

impl CatalogRecipe {
    fn new(recipe: Recipe, bytes: &[u8], source: Source) -> Self {
        CatalogRecipe { recipe, hash: sha256_hex(bytes), source }
    }
}

/// All recipes a project can use: built-ins, with the project's files replacing them by id.
///
/// Returns the recipes that load and every problem found, so one broken file does not hide the
/// others. A recipe with a problem is missing from the list.
pub fn recipes(data: &Path, project: &str) -> (Vec<CatalogRecipe>, Vec<String>) {
    if let Err(e) = check_project(project) {
        return (Vec::new(), vec![e]);
    }
    let mut ids: BTreeSet<String> = BUILTIN.iter().map(|(id, _)| id.to_string()).collect();
    let mut problems = Vec::new();
    match project_recipe_files(data, project) {
        Ok(files) => {
            for path in files {
                match path.file_stem().and_then(|s| s.to_str()) {
                    Some(id) if valid_name(id) => {
                        ids.insert(id.to_string());
                    }
                    _ => problems.push(format!(
                        "{}: a recipe file is named <id>.json, and the id is lowercase letters, digits and dashes",
                        path.display()
                    )),
                }
            }
        }
        Err(e) => problems.push(e),
    }
    let mut out = Vec::new();
    for id in ids {
        match load_recipe(data, project, &id) {
            Ok(Some(recipe)) => out.push(recipe),
            Ok(None) => {}
            Err(e) => problems.push(e),
        }
    }
    (out, problems)
}

/// The recipe `id` as a project resolves it: the project's file when there is one, else the built-in.
///
/// `None` when the id is unknown, or when the project's file for it is invalid. An invalid file is
/// never replaced by the built-in: `recipes` reports the problem.
pub fn recipe(data: &Path, project: &str, id: &str) -> Option<(Recipe, String)> {
    load_recipe(data, project, id).ok().flatten().map(|r| (r.recipe, r.hash))
}

/// The profile file of `name` in `project`, with the checks that keep it inside the project's directory.
pub fn profile_path(data: &Path, project: &str, name: &str) -> Result<PathBuf, String> {
    check_project(project)?;
    check_profile_name(name)?;
    Ok(environments_dir(data, project).join("profiles").join(format!("{name}.json")))
}

/// The profile `name` of `project` with the SHA-256 of its file, or `None` when there is no such file.
///
/// The file must parse as a profile; a broken one is an error that names the file.
pub fn load_profile(data: &Path, project: &str, name: &str) -> Result<Option<(Profile, String)>, String> {
    let path = profile_path(data, project, name)?;
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let profile = serde_json::from_slice(&bytes).map_err(|e| format!("{}: not a valid profile: {e}", path.display()))?;
    Ok(Some((profile, sha256_hex(&bytes))))
}

/// Write the profile `name` of `project` as pretty JSON, atomically. The name is checked first.
pub fn save_profile(data: &Path, project: &str, name: &str, profile: &Profile) -> Result<(), String> {
    let path = profile_path(data, project, name)?;
    let dir = path.parent().ok_or_else(|| format!("{}: no parent directory", path.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut text = serde_json::to_string_pretty(profile).map_err(|e| format!("profile {name}: {e}"))?;
    text.push('\n');
    genie_core::vault::atomic_write(&path, text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))
}

/// Remove the profile `name`. `Ok(false)` when it did not exist.
pub fn delete_profile(data: &Path, project: &str, name: &str) -> Result<bool, String> {
    let path = profile_path(data, project, name)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// The names of the project's profiles, sorted. Files whose names are not profile names are not profiles.
pub fn profiles(data: &Path, project: &str) -> Vec<String> {
    if check_project(project).is_err() {
        return Vec::new();
    }
    let dir = environments_dir(data, project).join("profiles");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|path| path.file_stem().and_then(|s| s.to_str()).map(str::to_string))
        .filter(|name| valid_name(name))
        .collect();
    names.sort();
    names
}

/// Resolve the profile `profile` of `project` against the project's recipes and repositories.
///
/// Fails with every problem: a broken recipe or profile file, a missing profile, or whatever
/// `spec::resolve` finds. Nothing is resolved while any recipe file of the project is invalid.
pub fn resolve_profile(data: &Path, project: &str, profile: &str, repos: &[RepoInfo]) -> Result<Resolved, Vec<String>> {
    let (list, mut problems) = recipes(data, project);
    let loaded = match load_profile(data, project, profile) {
        Ok(Some((loaded, _))) => Some(loaded),
        Ok(None) => {
            problems.push(format!("no profile named {profile:?} in project {project}"));
            None
        }
        Err(e) => {
            problems.push(e);
            None
        }
    };
    let profile = match loaded {
        Some(loaded) if problems.is_empty() => loaded,
        _ => return Err(problems),
    };
    let lookup = |id: &str| list.iter().find(|r| r.recipe.id == id).map(|r| (r.recipe.clone(), r.hash.clone()));
    spec::resolve(&profile, &lookup, repos)
}

fn check_project(project: &str) -> Result<(), String> {
    if valid_slug(project) { Ok(()) } else { Err(format!("project {project:?} is not a project slug")) }
}

fn check_profile_name(name: &str) -> Result<(), String> {
    if valid_name(name) {
        Ok(())
    } else {
        Err(format!("profile name {name:?} must be lowercase letters, digits and dashes, starting with a letter or digit"))
    }
}

fn environments_dir(data: &Path, project: &str) -> PathBuf {
    data.join("environments").join(project)
}

/// The `*.json` files in the project's recipe directory. A missing directory has none.
fn project_recipe_files(data: &Path, project: &str) -> Result<Vec<PathBuf>, String> {
    let dir = environments_dir(data, project).join("recipes");
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "json") && entry.file_type().is_ok_and(|t| t.is_file()) {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// One recipe by id: the project's file if it exists (and must be valid), else the built-in.
fn load_recipe(data: &Path, project: &str, id: &str) -> Result<Option<CatalogRecipe>, String> {
    if !valid_name(id) {
        return Ok(None);
    }
    check_project(project)?;
    let path = environments_dir(data, project).join("recipes").join(format!("{id}.json"));
    match std::fs::read(&path) {
        Ok(bytes) => {
            let recipe = parse_recipe(id, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(Some(CatalogRecipe::new(recipe, &bytes, Source::Project)))
        }
        Err(e) if e.kind() == ErrorKind::NotFound => builtin(id),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn builtin(id: &str) -> Result<Option<CatalogRecipe>, String> {
    let Some((_, text)) = BUILTIN.iter().find(|(name, _)| *name == id) else {
        return Ok(None);
    };
    let recipe = parse_recipe(id, text.as_bytes()).map_err(|e| format!("built-in recipe {id}: {e}"))?;
    Ok(Some(CatalogRecipe::new(recipe, text.as_bytes(), Source::Builtin)))
}

/// Parse a recipe file whose name is `id`: it must be valid JSON for a recipe and declare that id.
fn parse_recipe(id: &str, bytes: &[u8]) -> Result<Recipe, String> {
    let recipe: Recipe = serde_json::from_slice(bytes).map_err(|e| format!("not a valid recipe: {e}"))?;
    if recipe.id != id {
        return Err(format!("declares the id {:?}, but the file is named for {id:?}: the file name must be the id", recipe.id));
    }
    Ok(recipe)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// Every recipe file this module reads or writes is kept under one temporary data directory.
    fn data() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn repos() -> Vec<RepoInfo> {
        vec![RepoInfo { name: "app".into(), mount: "app".into() }]
    }

    /// A profile with one component: `app` uses `recipe` with `params`.
    fn one_component(recipe: &str, params: &[(&str, &str)]) -> Profile {
        let params: BTreeMap<String, String> = params.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let component =
            spec::Component { repo: "app".into(), recipe: recipe.into(), params, prepare: Vec::new(), commands: BTreeMap::new() };
        Profile { components: vec![component], ..Default::default() }
    }

    fn write_project_recipe(data: &Path, project: &str, file: &str, text: &str) -> PathBuf {
        let dir = environments_dir(data, project).join("recipes");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(file);
        std::fs::write(&path, text).unwrap();
        path
    }

    /// The design doc's example profile: base image and packages, three components with their
    /// recipes (one with a custom step), a service with a probe, a process after it, a check, and a
    /// secret granted to one step.
    const EXAMPLE_PROFILE: &str = r#"{
        "base": { "image": "docker.io/library/debian:12", "packages": ["build-essential", "libpq-dev"] },
        "requires": { "arch": ["x86_64"], "memory": "8GiB", "cpus": 4 },
        "components": [
            { "repo": "web", "recipe": "node-npm", "params": { "node": "22" } },
            { "repo": "api", "recipe": "rust-cargo" },
            { "repo": "legacy", "recipe": "custom", "prepare": [{ "id": "bootstrap", "run": "./scripts/bootstrap.sh" }] }
        ],
        "services": {
            "db": {
                "image": "docker.io/library/postgres:16",
                "env": { "POSTGRES_PASSWORD": "genie" },
                "ready": { "command": "pg_isready -U postgres" },
                "data": "workspace"
            }
        },
        "processes": {
            "api": { "repo": "api", "command": "cargo run", "after": ["db"], "ready": { "http": "http://127.0.0.1:8080/health" } }
        },
        "checks": { "integration": "make -C infra e2e" },
        "network": { "prepare": "registries", "work": "none" },
        "secrets": { "npm_token": { "steps": ["web/install"], "file": "/run/secrets/npm" } }
    }"#;

    fn example_profile() -> Profile {
        serde_json::from_str(EXAMPLE_PROFILE).unwrap()
    }

    fn three_repos() -> Vec<RepoInfo> {
        ["web", "api", "legacy"].iter().map(|n| RepoInfo { name: n.to_string(), mount: n.to_string() }).collect()
    }

    #[test]
    fn every_builtin_parses_and_resolves_with_default_params() {
        let dir = data();
        for (id, _) in BUILTIN {
            save_profile(dir.path(), "demo", "default", &one_component(id, &[])).unwrap();
            let resolved = resolve_profile(dir.path(), "demo", "default", &repos())
                .unwrap_or_else(|e| panic!("built-in {id} does not resolve: {e:?}"));
            assert_eq!(resolved.components.len(), 1, "{id}");
            let c = &resolved.components[0];
            assert_eq!(c.mount, "app", "{id}");
            assert_eq!(c.recipe, *id, "{id}");
            assert_eq!(c.revision, 1, "{id}");
            let (_, text) = BUILTIN.iter().find(|(name, _)| name == id).unwrap();
            assert_eq!(c.recipe_hash, sha256_hex(text.as_bytes()), "{id}: the hash of the embedded file");
        }
    }

    #[test]
    fn builtin_recipes_substitute_default_and_explicit_params() {
        let dir = data();
        let resolve = |recipe: &str, params: &[(&str, &str)]| {
            save_profile(dir.path(), "demo", "default", &one_component(recipe, params)).unwrap();
            resolve_profile(dir.path(), "demo", "default", &repos()).unwrap().components.remove(0)
        };

        let node = resolve("node-npm", &[]);
        assert_eq!(node.tools["node"].version, "");
        assert_eq!(node.tools["npm"].probe, "npm --version");
        let install = &node.steps[0];
        assert_eq!(install.qualified, "app/install");
        assert_eq!(install.cwd, "");
        assert_eq!(install.outputs, ["node_modules"]);
        assert_eq!(install.inputs, spec::Inputs::Files(["package.json", "package-lock.json", ".npmrc"].map(String::from).to_vec()));
        assert_eq!(node.commands["build"], "cd . && npm run build --if-present");

        let node = resolve("node-npm", &[("node", "22"), ("dir", "web/")]);
        assert_eq!(node.tools["node"].version, "22");
        let install = &node.steps[0];
        assert_eq!(install.cwd, "web/");
        assert_eq!(install.outputs, ["web/node_modules"]);
        assert_eq!(
            install.inputs,
            spec::Inputs::Files(["web/package.json", "web/package-lock.json", "web/.npmrc"].map(String::from).to_vec())
        );
        assert_eq!(node.commands["build"], "cd web/. && npm run build --if-present");
        assert_eq!(node.commands["test"], "cd web/. && npm test");

        let python = resolve("python-uv", &[]);
        assert_eq!(python.tools["python"].version, "");
        assert_eq!(python.tools["uv"].probe, "uv --version");
        let sync = &python.steps[0];
        assert_eq!(sync.cwd, "");
        assert_eq!(sync.outputs, [".venv"]);
        assert_eq!(sync.verify.as_deref(), Some("uv run --frozen python -c \"import sys\""));
        assert_eq!(python.commands["test"], "cd . && uv run --frozen pytest");

        let python = resolve("python-uv", &[("python", "3.12"), ("dir", "svc/")]);
        assert_eq!(python.tools["python"].version, "3.12");
        let sync = &python.steps[0];
        assert_eq!(sync.cwd, "svc/");
        assert_eq!(sync.outputs, ["svc/.venv"]);
        assert_eq!(
            sync.inputs,
            spec::Inputs::Files(["svc/pyproject.toml", "svc/uv.lock", "svc/.python-version"].map(String::from).to_vec())
        );
        assert_eq!(python.commands["test"], "cd svc/. && uv run --frozen pytest");

        let cargo = resolve("rust-cargo", &[("rust", "1.85")]);
        assert_eq!(cargo.tools["cargo"].version, "1.85");
        assert_eq!(cargo.steps[0].outputs, Vec::<String>::new());
        assert_eq!(cargo.commands["lint"], "cargo clippy --locked");

        let gradle = resolve("jvm-gradle", &[]);
        assert_eq!(gradle.tools["java"].probe, "java -version");
        assert_eq!(gradle.steps[0].run, "./gradlew --no-daemon dependencies");
        assert!(gradle.steps[0].outputs.is_empty());
        assert_eq!(gradle.commands["test"], "./gradlew --no-daemon test");
    }

    #[test]
    fn builtin_caches_have_their_integrity_and_paths() {
        let dir = data();
        let cache = |recipe: &str| {
            save_profile(dir.path(), "demo", "default", &one_component(recipe, &[])).unwrap();
            resolve_profile(dir.path(), "demo", "default", &repos()).unwrap().components.remove(0).caches
        };
        let summary = |caches: Vec<spec::Cache>| -> Vec<(String, String, spec::Integrity)> {
            caches.into_iter().map(|c| (c.name, c.path, c.integrity)).collect()
        };
        use spec::Integrity::{Unverified, Verified};
        assert_eq!(
            summary(cache("rust-cargo")),
            [
                ("cargo-registry".to_string(), "~/.cargo/registry".to_string(), Verified),
                ("cargo-git".to_string(), "~/.cargo/git".to_string(), Unverified),
            ]
        );
        assert_eq!(summary(cache("node-npm")), [("npm".to_string(), "~/.npm".to_string(), Verified)]);
        assert_eq!(summary(cache("python-uv")), [("uv".to_string(), "~/.cache/uv".to_string(), Verified)]);
        assert_eq!(
            summary(cache("jvm-gradle")),
            [
                ("gradle-caches".to_string(), "~/.gradle/caches".to_string(), Unverified),
                ("gradle-wrapper".to_string(), "~/.gradle/wrapper".to_string(), Unverified),
            ]
        );
        assert!(cache("custom").is_empty());
    }

    #[test]
    fn custom_recipe_takes_only_the_profile_steps() {
        let dir = data();
        let mut profile = one_component("custom", &[]);
        profile.components[0].prepare.push(spec::Step {
            id: "bootstrap".into(),
            run: "./scripts/bootstrap.sh".into(),
            cwd: String::new(),
            outputs: Vec::new(),
            inputs: spec::Inputs::default(),
            env: Vec::new(),
            network: None,
            verify: None,
            timeout_secs: None,
        });
        save_profile(dir.path(), "demo", "default", &profile).unwrap();
        let resolved = resolve_profile(dir.path(), "demo", "default", &repos()).unwrap();
        let c = &resolved.components[0];
        assert!(c.tools.is_empty() && c.caches.is_empty() && c.commands.is_empty());
        assert_eq!(c.steps.len(), 1);
        assert_eq!(c.steps[0].qualified, "app/bootstrap");
    }

    #[test]
    fn a_project_recipe_replaces_the_builtin_of_the_same_id() {
        let dir = data();
        let text = r#"{"id": "rust-cargo", "revision": 7, "description": "mine",
            "commands": {"test": "cargo nextest run"}}"#;
        write_project_recipe(dir.path(), "demo", "rust-cargo.json", text);

        let (found, hash) = recipe(dir.path(), "demo", "rust-cargo").unwrap();
        assert_eq!(found.revision, 7);
        assert_eq!(hash, sha256_hex(text.as_bytes()), "the hash of the project's file");

        let (listing, problems) = recipes(dir.path(), "demo");
        assert!(problems.is_empty(), "{problems:?}");
        let sources: Vec<(String, Source)> = listing.iter().map(|r| (r.recipe.id.clone(), r.source)).collect();
        assert_eq!(sources.len(), BUILTIN.len());
        assert!(sources.contains(&("rust-cargo".to_string(), Source::Project)));
        assert!(sources.contains(&("node-npm".to_string(), Source::Builtin)));
        assert_eq!(Source::Project.as_str(), "project");

        save_profile(dir.path(), "demo", "default", &one_component("rust-cargo", &[])).unwrap();
        let resolved = resolve_profile(dir.path(), "demo", "default", &repos()).unwrap();
        assert_eq!(resolved.components[0].revision, 7);
        assert_eq!(resolved.components[0].recipe_hash, hash);
        assert_eq!(resolved.components[0].commands["test"], "cargo nextest run");

        // Another project keeps the built-in.
        let (other, _) = recipe(dir.path(), "other", "rust-cargo").unwrap();
        assert_eq!(other.revision, 1);
    }

    #[test]
    fn a_project_recipe_can_add_a_new_stack() {
        let dir = data();
        write_project_recipe(
            dir.path(),
            "demo",
            "zig.json",
            r#"{"id": "zig", "revision": 1, "tools": {"zig": {"probe": "zig version"}}, "commands": {"build": "zig build"}}"#,
        );
        save_profile(dir.path(), "demo", "default", &one_component("zig", &[])).unwrap();
        let resolved = resolve_profile(dir.path(), "demo", "default", &repos()).unwrap();
        assert_eq!(resolved.components[0].tools["zig"].probe, "zig version");
        let (listing, _) = recipes(dir.path(), "demo");
        assert!(listing.iter().any(|r| r.recipe.id == "zig" && r.source == Source::Project));
    }

    #[test]
    fn a_broken_project_recipe_is_reported_and_never_replaced_by_the_builtin() {
        let dir = data();
        let path = write_project_recipe(dir.path(), "demo", "rust-cargo.json", r#"{"id": "rust-cargo", "revision": 1, "stpes": []}"#);
        write_project_recipe(dir.path(), "demo", "broken.json", "{ not json");

        let (listing, problems) = recipes(dir.path(), "demo");
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains(&path.display().to_string()) && p.contains("stpes")), "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("broken.json") && p.contains("not a valid recipe")), "{problems:?}");
        assert!(!listing.iter().any(|r| r.recipe.id == "broken"), "a recipe with a problem is not listed");
        assert!(recipe(dir.path(), "demo", "rust-cargo").is_none(), "the invalid file is not replaced by the built-in");
        assert!(recipe(dir.path(), "demo", "broken").is_none());
        assert!(recipe(dir.path(), "demo", "node-npm").is_some(), "other built-ins still load");

        // Resolution of any profile of the project fails with the same problems, even one that uses no broken recipe.
        save_profile(dir.path(), "demo", "default", &one_component("node-npm", &[])).unwrap();
        let errors = resolve_profile(dir.path(), "demo", "default", &repos()).unwrap_err();
        assert!(errors.iter().any(|e| e.contains("stpes")), "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("broken.json")), "{errors:?}");
    }

    #[test]
    fn a_recipe_file_must_be_named_for_its_id() {
        let dir = data();
        write_project_recipe(dir.path(), "demo", "fast.json", r#"{"id": "other", "revision": 1}"#);
        write_project_recipe(dir.path(), "demo", "Bad_Name.json", r#"{"id": "bad-name", "revision": 1}"#);
        let (listing, problems) = recipes(dir.path(), "demo");
        assert!(problems.iter().any(|p| p.contains("fast.json") && p.contains("declares the id \"other\"")), "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("Bad_Name.json") && p.contains("<id>.json")), "{problems:?}");
        assert!(!listing.iter().any(|r| r.recipe.id == "other"));
        assert!(recipe(dir.path(), "demo", "fast").is_none());
    }

    #[test]
    fn a_temporary_or_foreign_file_in_the_recipe_directory_is_ignored() {
        let dir = data();
        write_project_recipe(dir.path(), "demo", ".rust-cargo.json.abc.tmp", "{ half written");
        write_project_recipe(dir.path(), "demo", "notes.txt", "not a recipe");
        let (listing, problems) = recipes(dir.path(), "demo");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(listing.len(), BUILTIN.len());
    }

    #[test]
    fn an_unknown_or_unsafe_recipe_or_project_name_gives_nothing() {
        let dir = data();
        assert!(recipe(dir.path(), "demo", "no-such-recipe").is_none());
        assert!(recipe(dir.path(), "demo", "../node-npm").is_none());
        assert!(recipe(dir.path(), "../demo", "node-npm").is_none());
        let (listing, problems) = recipes(dir.path(), "../demo");
        assert!(listing.is_empty());
        assert_eq!(problems.len(), 1, "{problems:?}");
    }

    #[test]
    fn a_profile_round_trips_and_its_hash_follows_the_content() {
        let dir = data();
        let profile = example_profile();
        save_profile(dir.path(), "demo", "default", &profile).unwrap();

        let path = profile_path(dir.path(), "demo", "default").unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.ends_with(b"}\n"), "pretty JSON with a final newline");
        let (loaded, hash) = load_profile(dir.path(), "demo", "default").unwrap().unwrap();
        assert_eq!(loaded, profile);
        assert_eq!(hash, sha256_hex(&bytes));

        let mut changed = profile.clone();
        changed.checks.insert("smoke".into(), "make smoke".into());
        save_profile(dir.path(), "demo", "default", &changed).unwrap();
        let (_, changed_hash) = load_profile(dir.path(), "demo", "default").unwrap().unwrap();
        assert_ne!(changed_hash, hash, "a different profile has a different hash");

        save_profile(dir.path(), "demo", "default", &changed).unwrap();
        assert_eq!(load_profile(dir.path(), "demo", "default").unwrap().unwrap().1, changed_hash, "the same content, the same hash");

        let profiles_dir = path.parent().unwrap();
        let files: Vec<_> = std::fs::read_dir(profiles_dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(files.len(), 1, "no temporary file is left behind: {files:?}");
    }

    #[test]
    fn profiles_are_listed_deleted_and_missing_ones_say_so() {
        let dir = data();
        assert!(profiles(dir.path(), "demo").is_empty());
        assert_eq!(load_profile(dir.path(), "demo", "default").unwrap(), None);
        assert!(!delete_profile(dir.path(), "demo", "default").unwrap());

        save_profile(dir.path(), "demo", "integration", &Profile::default()).unwrap();
        save_profile(dir.path(), "demo", "default", &Profile::default()).unwrap();
        assert_eq!(profiles(dir.path(), "demo"), ["default", "integration"]);

        assert!(delete_profile(dir.path(), "demo", "default").unwrap());
        assert_eq!(load_profile(dir.path(), "demo", "default").unwrap(), None);
        assert!(!delete_profile(dir.path(), "demo", "default").unwrap());
        assert_eq!(profiles(dir.path(), "demo"), ["integration"]);
    }

    #[test]
    fn profile_names_and_project_names_are_checked_before_any_file_is_touched() {
        let dir = data();
        for name in ["Default", "../escape", "", "-x", "a/b"] {
            let err = save_profile(dir.path(), "demo", name, &Profile::default()).unwrap_err();
            assert!(err.contains("profile name"), "{name:?}: {err}");
            assert!(load_profile(dir.path(), "demo", name).is_err(), "{name:?}");
            assert!(delete_profile(dir.path(), "demo", name).is_err(), "{name:?}");
        }
        assert!(save_profile(dir.path(), "../demo", "default", &Profile::default()).unwrap_err().contains("project slug"));
        assert!(profile_path(dir.path(), "Demo", "default").is_err());
        assert!(!dir.path().join("environments").exists(), "nothing was written");
    }

    #[test]
    fn a_corrupt_profile_file_is_an_error_that_names_the_file() {
        let dir = data();
        let path = profile_path(dir.path(), "demo", "default").unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"comonents": []}"#).unwrap();
        let err = load_profile(dir.path(), "demo", "default").unwrap_err();
        assert!(err.contains(&path.display().to_string()) && err.contains("not a valid profile"), "{err}");
        assert!(profiles(dir.path(), "demo").contains(&"default".to_string()), "the file is still listed, so it can be fixed or deleted");
    }

    #[test]
    fn the_design_example_resolves_when_its_repositories_and_recipes_exist() {
        let dir = data();
        save_profile(dir.path(), "demo", "default", &example_profile()).unwrap();
        let resolved = resolve_profile(dir.path(), "demo", "default", &three_repos()).unwrap();

        let names: Vec<&str> = resolved.components.iter().map(|c| c.recipe.as_str()).collect();
        assert_eq!(names, ["node-npm", "rust-cargo", "custom"]);
        let web = &resolved.components[0];
        assert_eq!(web.tools["node"].version, "22");
        assert_eq!(web.steps[0].qualified, "web/install");
        assert_eq!(web.steps[0].secrets, [("npm_token".to_string(), "/run/secrets/npm".to_string())]);
        assert!(resolved.components[1].steps[0].secrets.is_empty());
        let legacy = &resolved.components[2];
        assert_eq!(legacy.steps[0].qualified, "legacy/bootstrap");
        assert_eq!(legacy.steps[0].inputs, spec::Inputs::default(), "a custom step takes the whole repository");

        assert_eq!(resolved.base.image.as_deref(), Some("docker.io/library/debian:12"));
        assert_eq!(resolved.base.packages, ["build-essential", "libpq-dev"]);
        assert_eq!(resolved.requires.memory.as_deref(), Some("8GiB"));
        assert_eq!(resolved.requires.cpus, Some(4.0));
        assert_eq!(resolved.requires.arch, ["x86_64"]);
        assert_eq!(resolved.services["db"].ready.as_ref().and_then(|p| p.command.as_deref()), Some("pg_isready -U postgres"));
        assert_eq!(resolved.processes["api"].after, ["db"]);
        assert_eq!(resolved.checks["integration"], "make -C infra e2e");
        assert_eq!(resolved.network.work, spec::NetworkMode::None);
    }

    #[test]
    fn the_design_example_reports_each_missing_repository_and_recipe_together() {
        let dir = data();
        let mut profile = example_profile();
        profile.components[0].recipe = "node-npn".into();
        save_profile(dir.path(), "demo", "default", &profile).unwrap();

        let errors = resolve_profile(dir.path(), "demo", "default", &[RepoInfo { name: "api".into(), mount: "api".into() }]).unwrap_err();
        assert!(errors.contains(&"component web: the project has no repository named web".to_string()), "{errors:?}");
        assert!(errors.contains(&"component legacy: the project has no repository named legacy".to_string()), "{errors:?}");
        assert!(errors.contains(&"component web: no recipe named node-npn".to_string()), "{errors:?}");
        assert!(errors.len() >= 3, "every problem is reported: {errors:?}");
    }

    #[test]
    fn a_secret_granted_to_a_missing_step_and_an_unknown_profile_are_reported() {
        let dir = data();
        let mut profile = example_profile();
        profile.secrets.get_mut("npm_token").unwrap().steps = vec!["web/instal".into()];
        save_profile(dir.path(), "demo", "default", &profile).unwrap();
        let errors = resolve_profile(dir.path(), "demo", "default", &three_repos()).unwrap_err();
        assert!(errors.iter().any(|e| e.contains("secret npm_token: web/instal is not a step")), "{errors:?}");

        let errors = resolve_profile(dir.path(), "demo", "nope", &three_repos()).unwrap_err();
        assert_eq!(errors, [r#"no profile named "nope" in project demo"#.to_string()]);
    }
}
