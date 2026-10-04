//! Application state shared by the HTTP layer and the background workers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use genie_core::server_db::{Project, ServerDb};
use genie_core::vault::Vault;
use genie_core::{GenieError, Tracker};
use tokio::sync::Notify;

use crate::agent_config::AgentConfig;
use crate::config::Config;

/// Error from a blocking database call made on behalf of async code.
#[derive(Debug)]
pub enum AppError {
    Genie(GenieError),
    /// The request itself is wrong (names someone who is not there, a malformed field).
    Bad(String),
    /// The caller may not do this here (an agent outside its task).
    Forbidden(String),
    /// The request is fine but the state of things is in the way (an open request, the project's autonomy).
    Conflict(String),
    Internal(String),
}

impl From<GenieError> for AppError {
    fn from(e: GenieError) -> Self {
        AppError::Genie(e)
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Genie(e) => write!(f, "{e}"),
            AppError::Bad(e) | AppError::Forbidden(e) | AppError::Conflict(e) | AppError::Internal(e) => write!(f, "{e}"),
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;

/// Run a future to its end from synchronous code — a worker of [`App::blocking`], the engine's
/// tick, a test — on a thread of its own, so it never nests in the caller's runtime.
pub fn block_on<F>(f: F) -> F::Output
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    std::thread::scope(|s| {
        s.spawn(|| tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a tokio runtime").block_on(f))
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e))
    })
}

pub struct App {
    pub data: PathBuf,
    pub cfg: Config,
    /// A directory with the built web UI (`--web`); None: the one built into the binary.
    pub web_root: Option<PathBuf>,
    server: Mutex<ServerDb>,
    /// The knowledge vault shared by all projects of this installation.
    pub vault: Mutex<Vault>,
    projects: RwLock<HashMap<String, Arc<Mutex<Tracker>>>>,
    /// Wakes the agent scheduler (new mail, finished turn, new job).
    pub wake_runtime: Notify,
    /// Wakes the automation engine (new events, answered questions, finished jobs).
    pub wake_engine: Notify,
    /// Wakes the delivery dispatcher (new outbox rows).
    pub wake_outbox: Notify,
    /// Path of the running `genie` binary, given to agents so they can call back.
    pub exe: PathBuf,
    /// Live agent sessions (long-running harness processes).
    pub sessions: crate::sessions::Registry,
    /// The scheduler of agent turns (whose turn is running, who waits after failures).
    pub sched: crate::runtime::Sched,
    /// Roles, team templates, skills and MCP connections (reloaded when their files change).
    agents: RwLock<Arc<AgentConfig>>,
    /// The agents' connections through the MCP gateway.
    pub mcp: crate::mcp_gateway::Gateway,
    /// Locks and fetch times of the repository mirrors and workspaces.
    pub git: crate::git::Git,
}

impl App {
    pub fn open(data: &Path, cfg: Config, web_root: impl Into<Option<PathBuf>>) -> AppResult<Arc<App>> {
        std::fs::create_dir_all(data).map_err(|e| AppError::Internal(format!("{}: {e}", data.display())))?;
        let server = ServerDb::open(&data.join("server.db"))?;
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("genie"));
        let mut vault = Vault::open(&cfg.vault_path(data), &data.join("vault-index.db"), cfg.vault.commit.unwrap_or(true))?;
        for p in server.projects()? {
            vault.ensure_space(&p.slug, p.repo.as_deref().map(Path::new))?;
        }
        let agents = AgentConfig::load(data, &cfg, None);
        Ok(Arc::new(App {
            data: data.to_path_buf(),
            cfg,
            web_root: web_root.into(),
            server: Mutex::new(server),
            vault: Mutex::new(vault),
            projects: RwLock::new(HashMap::new()),
            wake_runtime: Notify::new(),
            wake_engine: Notify::new(),
            wake_outbox: Notify::new(),
            exe,
            sessions: Default::default(),
            sched: Default::default(),
            agents: RwLock::new(Arc::new(agents)),
            mcp: Default::default(),
            git: Default::default(),
        }))
    }

    /// Print the errors of the agent configuration (when the server starts).
    pub fn print_agent_errors(&self) {
        for p in self.agents().errors() {
            eprintln!(
                "genie: agent configuration: {}{}: {}",
                p.item,
                p.path.as_deref().map(|x| format!(" ({x})")).unwrap_or_default(),
                p.message
            );
        }
    }

    /// The current agent configuration (a snapshot: cheap to clone, never torn).
    pub fn agents(&self) -> Arc<AgentConfig> {
        self.agents.read().map(|a| a.clone()).unwrap_or_else(|e| e.into_inner().clone())
    }

    /// Re-read the agent configuration from its files; broken items keep their last
    /// valid version. Running agents get their new guard rules at once.
    pub fn reload_agents(&self) -> Arc<AgentConfig> {
        let previous = self.agents();
        let fresh = Arc::new(AgentConfig::load(&self.data, &self.cfg, Some(&previous)));
        match self.agents.write() {
            Ok(mut a) => *a = fresh.clone(),
            Err(e) => *e.into_inner() = fresh.clone(),
        }
        crate::sessions::refresh_policies(self);
        fresh
    }

    /// Synchronous access to the server database (call from blocking contexts only).
    pub fn with_server<T>(&self, f: impl FnOnce(&ServerDb) -> Result<T, GenieError>) -> AppResult<T> {
        let db = self.server.lock().map_err(|_| AppError::Internal("server db lock poisoned".into()))?;
        Ok(f(&db)?)
    }

    /// A project's tracker, opened on first use.
    fn project_tracker(&self, slug: &str) -> AppResult<Arc<Mutex<Tracker>>> {
        if let Some(p) = self.projects.read().map_err(|_| AppError::Internal("registry poisoned".into()))?.get(slug) {
            return Ok(p.clone());
        }
        let project = self.with_server(|db| db.project(slug))?;
        let tracker = Arc::new(Mutex::new(Tracker::open(&project.tracker_dir)?));
        // Opening a tracker may have migrated it: log it, like the server's start-up does.
        crate::cli::report_migrations(&self.data);
        let mut map = self.projects.write().map_err(|_| AppError::Internal("registry poisoned".into()))?;
        Ok(map.entry(slug.to_string()).or_insert(tracker).clone())
    }

    /// Synchronous access to a project's tracker (call from blocking contexts only).
    pub fn with_tracker<T>(&self, slug: &str, f: impl FnOnce(&Tracker) -> Result<T, GenieError>) -> AppResult<T> {
        let tracker = self.project_tracker(slug)?;
        let t = tracker.lock().map_err(|_| AppError::Internal("tracker lock poisoned".into()))?;
        Ok(f(&t)?)
    }

    /// Run blocking work (SQLite, git, files) off the async runtime.
    pub async fn blocking<T: Send + 'static>(self: &Arc<Self>, f: impl FnOnce(&App) -> AppResult<T> + Send + 'static) -> AppResult<T> {
        let app = self.clone();
        tokio::task::spawn_blocking(move || f(&app)).await.map_err(|e| AppError::Internal(e.to_string()))?
    }

    /// Create a project: a tracker directory under `<data>/projects/<slug>`, or an
    /// existing tracker (e.g. a repository's `.genie/`) registered in place.
    pub fn create_project(
        &self,
        slug: &str,
        name: &str,
        repo: Option<&str>,
        tracker_dir: Option<&str>,
        prefix: Option<&str>,
    ) -> AppResult<Project> {
        // Checked before anything is created on disk: the slug names a directory.
        let slug = slug.trim().to_lowercase();
        let slug = slug.as_str();
        if !genie_core::server_db::valid_slug(slug) {
            return Err(GenieError::invalid("project slug must be lowercase latin letters, digits and dashes").into());
        }
        if self.with_server(|db| db.project_opt(slug))?.is_some() {
            return Err(GenieError::invalid(format!("project {slug} already exists")).into());
        }
        if let Some(r) = repo
            && !Path::new(r).is_dir()
        {
            return Err(GenieError::invalid(format!("repository {r}: no such directory on the server")).into());
        }
        let dir = match tracker_dir {
            Some(d) => PathBuf::from(d),
            None => self.data.join("projects").join(slug),
        };
        let existing = dir.join(genie_core::tracker::DB_FILE).exists();
        let tracker =
            if existing { Tracker::open(&dir)? } else { Tracker::init(&dir, prefix, Some(if name.is_empty() { slug } else { name }))? };
        drop(tracker);
        let dir = dir.canonicalize().map_err(|e| AppError::Internal(format!("{}: {e}", dir.display())))?;
        let repo = repo
            .map(|r| Path::new(r).canonicalize().map(|p| p.to_string_lossy().into_owned()))
            .transpose()
            .map_err(|e| AppError::Internal(format!("repo: {e}")))?;
        let project = self.with_server(|db| db.create_project(slug, name, &dir.to_string_lossy(), repo.as_deref(), None))?;
        self.with_vault(|v| v.ensure_space(&project.slug, project.repo.as_deref().map(Path::new)))?;
        Ok(project)
    }

    /// Synchronous access to the vault (call from blocking contexts only).
    pub fn with_vault<T>(&self, f: impl FnOnce(&mut Vault) -> Result<T, GenieError>) -> AppResult<T> {
        let mut v = self.vault.lock().map_err(|_| AppError::Internal("vault lock poisoned".into()))?;
        Ok(f(&mut v)?)
    }
}
