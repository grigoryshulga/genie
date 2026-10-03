//! Agents in a sandbox (bubblewrap). An agent process sees the machine read-only;
//! it writes only in its working directory and that directory's git repository,
//! its session files, pi's directory, its own `/tmp` and the usual tool caches.
//! The server's data directory (the databases, `config.json` with the channel
//! secrets, the vault), the other projects, the tracker files and secret
//! directories of the home (`~/.ssh`, cloud credentials…) are not there at all,
//! and the agent has its own `/proc`: the server's process and its environment
//! are out of sight. The network stays shared: agents reach their models and
//! the genie API.
//!
//! Without bubblewrap (`runtime.sandbox`: `auto` on a machine where it does not
//! work, or `off`) agents run as before, and only the soft limits of the guard
//! extension keep them within their role.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::config::SandboxConfig;

/// What an agent may do with a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Hidden,
    ReadOnly,
    Writable,
}

/// Home directories agents may write when they exist: pi's own (unless it is
/// elsewhere) and the caches of the usual toolchains.
const HOME_WRITABLE: &[&str] =
    &[".pi", ".cache", ".npm", ".cargo", ".rustup", "go", ".m2", ".gradle", ".yarn", ".bun", ".local/share/pnpm"];
/// Home paths agents never see: keys, cloud and cluster credentials, tokens.
const HOME_HIDDEN: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".config/gcloud",
    ".kube",
    ".docker",
    ".config/gh",
    ".password-store",
    ".local/share/keyrings",
    ".netrc",
    ".git-credentials",
];

/// Sockets that hand out the machine: the container engines' control sockets reach root through a mounted host path.
const HIDDEN_SOCKETS: &[&str] = &[
    "/run/docker.sock",
    "/run/podman/podman.sock",
    "/run/containerd/containerd.sock",
    "/run/crio/crio.sock",
    "/run/buildkit/buildkitd.sock",
];

/// What the server itself runs outside the sandbox when it uses git in a repository the agent writes to
/// (`git worktree add`, `git status`, checkouts): a hook, or a config key such as `core.fsmonitor`, written by
/// the agent would run with the server's rights. They stay read-only; commits, refs and objects remain writable.
pub const GIT_DIR_READONLY: &[&str] = &["hooks", "config", "config.worktree", "info/attributes"];

/// The mounts of one agent's sandbox.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    mounts: BTreeMap<PathBuf, Access>,
    /// Bound at `/tmp`.
    tmp: Option<PathBuf>,
    cwd: PathBuf,
}

impl Plan {
    pub fn new(cwd: &Path) -> Plan {
        Plan { cwd: real(cwd).unwrap_or_else(|| cwd.to_path_buf()), ..Default::default() }
    }

    /// Set what the agent may do with `path` (if it exists); a later call wins.
    pub fn set(&mut self, path: &Path, access: Access) {
        if let Some(p) = real(path) {
            self.mounts.insert(p, access);
        }
    }

    /// The agent's own `/tmp`.
    pub fn tmp(&mut self, dir: &Path) {
        self.tmp = real(dir);
    }

    pub fn access(&self, path: &Path) -> Option<Access> {
        self.mounts.get(path).copied()
    }

    /// bwrap's arguments before the command. Mounts go from the root down, so a
    /// deeper path overrides the one above it (a working directory inside a
    /// hidden tree, a tracker inside a writable repository).
    pub fn args(&self) -> Vec<String> {
        let s = |p: &Path| p.to_string_lossy().into_owned();
        let mut out: Vec<String> =
            ["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--unshare-pid", "--unshare-ipc", "--die-with-parent"]
                .iter()
                .map(|x| x.to_string())
                .collect();
        if let Some(t) = &self.tmp {
            out.extend(["--bind".into(), s(t), "/tmp".into()]);
        }
        let mut mounts: Vec<(&PathBuf, &Access)> = self.mounts.iter().collect();
        mounts.sort_by_key(|(p, _)| p.components().count());
        for (path, access) in mounts {
            match access {
                Access::Writable => out.extend(["--bind".into(), s(path), s(path)]),
                Access::ReadOnly => out.extend(["--ro-bind".into(), s(path), s(path)]),
                Access::Hidden if path.is_dir() => out.extend(["--tmpfs".into(), s(path)]),
                // A file, a socket or any other special file: an empty file takes its place (a connection to the socket is refused).
                Access::Hidden => out.extend(["--ro-bind".into(), "/dev/null".into(), s(path)]),
            }
        }
        for var in ["DBUS_SESSION_BUS_ADDRESS", "SSH_AUTH_SOCK", "GPG_AGENT_INFO"] {
            out.extend(["--unsetenv".into(), var.into()]);
        }
        out.extend(["--chdir".into(), s(&self.cwd), "--".into()]);
        out
    }

    /// The command to run instead of `program args`.
    pub fn wrap(&self, program: &str, args: &[String]) -> (String, Vec<String>) {
        let mut out = self.args();
        out.push(program.to_string());
        out.extend(args.iter().cloned());
        ("bwrap".to_string(), out)
    }
}

/// The path as the kernel sees it (symlinks resolved), when it exists.
fn real(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

/// `~/x` against the home directory.
pub fn expand(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if path == "~" => home.to_path_buf(),
        None => PathBuf::from(path),
    }
}

/// The defaults every sandbox gets, then the configuration's own paths (which win).
pub fn defaults(plan: &mut Plan, cfg: &SandboxConfig, home: &Path, pi_dir: Option<&Path>) {
    for p in HOME_WRITABLE {
        plan.set(&home.join(p), Access::Writable);
    }
    if let Some(d) = pi_dir {
        plan.set(d, Access::Writable);
    }
    for p in HOME_HIDDEN {
        plan.set(&home.join(p), Access::Hidden);
    }
    for p in HIDDEN_SOCKETS {
        plan.set(Path::new(p), Access::Hidden);
    }
    // The desktop session: its bus, keyring and agent sockets.
    if let Ok(run) = std::env::var("XDG_RUNTIME_DIR") {
        plan.set(Path::new(&run), Access::Hidden);
    }
    for p in &cfg.hidden {
        plan.set(&expand(p, home), Access::Hidden);
    }
    for p in &cfg.writable {
        plan.set(&expand(p, home), Access::Writable);
    }
}

/// Whether bubblewrap can make a sandbox on this machine (checked once).
pub fn works() -> bool {
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(|| {
        std::process::Command::new("bwrap")
            .args(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--unshare-pid", "--die-with-parent", "--", "true"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// Whether agents run in the sandbox; an error when the configuration requires
/// one this machine cannot make.
pub fn enabled(cfg: &SandboxConfig) -> Result<bool, String> {
    match cfg.mode.as_str() {
        "off" => Ok(false),
        "auto" => Ok(works()),
        "bwrap" if works() => Ok(true),
        "bwrap" => Err("runtime.sandbox is \"bwrap\", but bubblewrap does not work on this machine: install it (apt install bubblewrap) and allow unprivileged user namespaces".into()),
        other => Err(format!("runtime.sandbox: unknown mode {other:?} (auto, bwrap or off)")),
    }
}

/// How the sandbox stands, in words for people (`genie agents check`, the web).
pub fn status(cfg: &SandboxConfig) -> (bool, String) {
    match enabled(cfg) {
        Ok(true) => (true, "agents run in a bubblewrap sandbox".into()),
        Ok(false) if cfg.mode == "off" => (false, "the sandbox is off (runtime.sandbox): agents see everything the server user sees".into()),
        Ok(false) => (
            false,
            "bubblewrap does not work on this machine: agents run without a sandbox and see everything the server user sees (apt install bubblewrap)".into(),
        ),
        Err(e) => (false, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(path: &Path) -> PathBuf {
        std::fs::create_dir_all(path).unwrap();
        path.canonicalize().unwrap()
    }

    #[test]
    fn deeper_paths_override_the_ones_above_them() {
        let t = tempfile::tempdir().unwrap();
        let data = dir(&t.path().join("data"));
        let cwd = dir(&data.join("workspaces/shop/x"));
        let repo = dir(&t.path().join("repo"));
        let tracker = dir(&repo.join(".genie"));
        let mut plan = Plan::new(&cwd);
        plan.set(&data, Access::Hidden);
        plan.set(&cwd, Access::Writable);
        plan.set(&repo, Access::Writable);
        plan.set(&tracker, Access::Hidden);
        plan.set(&t.path().join("missing"), Access::Hidden);
        let args = plan.args();
        let at = |flag: &str, p: &Path| args.windows(2).position(|w| w[0] == flag && w[1] == p.to_string_lossy()).unwrap();
        assert!(at("--tmpfs", &data) < at("--bind", &cwd), "the working directory shows through the hidden data");
        assert!(at("--bind", &repo) < at("--tmpfs", &tracker), "the tracker is hidden inside the writable repository");
        assert!(!args.iter().any(|a| a.contains("missing")), "a path that does not exist is left out");
        assert_eq!(&args[..3], ["--ro-bind", "/", "/"]);
        assert!(args.ends_with(&["--chdir".to_string(), cwd.to_string_lossy().into_owned(), "--".to_string()]));
    }

    #[test]
    fn a_hidden_file_is_covered_with_an_empty_one() {
        let t = tempfile::tempdir().unwrap();
        let home = dir(t.path());
        std::fs::write(home.join(".netrc"), "machine x password y").unwrap();
        let mut plan = Plan::new(&home);
        defaults(&mut plan, &SandboxConfig::default(), &home, None);
        let args = plan.args();
        let netrc = home.join(".netrc").to_string_lossy().into_owned();
        assert!(args.windows(3).any(|w| w[0] == "--ro-bind" && w[1] == "/dev/null" && w[2] == netrc));
    }

    #[test]
    fn the_configuration_wins_over_the_defaults() {
        let t = tempfile::tempdir().unwrap();
        let home = dir(t.path());
        let cache = dir(&home.join(".cache"));
        let m2 = dir(&home.join(".m2"));
        let own = dir(&home.join("secrets"));
        let cfg =
            SandboxConfig { mode: "auto".into(), writable: vec!["~/.docker".into()], hidden: vec!["~/.cache".into(), "~/secrets".into()] };
        let docker = dir(&home.join(".docker"));
        let mut plan = Plan::new(&home);
        defaults(&mut plan, &cfg, &home, None);
        assert_eq!(plan.access(&cache), Some(Access::Hidden));
        assert_eq!(plan.access(&m2), Some(Access::Writable));
        assert_eq!(plan.access(&own), Some(Access::Hidden));
        assert_eq!(plan.access(&docker), Some(Access::Writable));
    }

    #[test]
    fn an_unknown_mode_is_an_error() {
        let cfg = |mode: &str| SandboxConfig { mode: mode.into(), ..Default::default() };
        assert_eq!(enabled(&cfg("off")), Ok(false));
        assert!(enabled(&cfg("docker")).is_err());
    }
}
