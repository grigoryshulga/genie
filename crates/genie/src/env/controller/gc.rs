//! Housekeeping of the environment store: reconciling the workspaces with the runtime after a
//! restart, and garbage collection of snapshots, staging directories, caches and bases.
//!
//! Garbage collection never removes what a live workspace pins. Its selection is pure, so the
//! policy (least recently used first, oldest caches first) is tested without a store.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use genie_core::env::db::{PIN_BASE, PIN_SNAPSHOT, SnapshotState, WsState};
use genie_core::env::spec::parse_size;

use super::{finish_delete, store_of};
use crate::env::store::{self, dir_size};
use crate::state::{App, AppError, AppResult};

/// A staging directory younger than this may belong to an attempt still running.
const STAGING_GRACE: Duration = Duration::from_secs(3600);

/// The reason recorded on a workspace whose runtime did not survive a restart.
const LOST: &str = "lost after a restart; run `genie env up` again";

/// What `reconcile` found and changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    /// Stages another process was running, now failed as interrupted.
    pub stages_interrupted: usize,
    /// Snapshots another process was preparing, now failed as interrupted.
    pub snapshots_interrupted: usize,
    /// Runtime objects of workspaces that no longer exist, removed.
    pub resources_removed: usize,
    /// Workspaces whose runtime is gone, now stopped.
    pub workspaces_stopped: Vec<String>,
    /// Deletions a restart interrupted, now finished.
    pub deletions_finished: Vec<String>,
    /// Anything that could not be done; reconciliation goes on past it.
    pub problems: Vec<String>,
}

/// Reconcile the database with the runtime, once at start: interrupted work is failed, staging
/// left by it is removed, and a workspace whose processes are gone is stopped. Problems are
/// reported, never fatal: the server must start even when the runtime is unavailable.
pub fn reconcile(app: &App) -> AppResult<ReconcileReport> {
    let owner = app.envs.owner();
    let mut report = ReconcileReport { stages_interrupted: app.with_server(|db| db.env_stages_interrupt(owner))?, ..Default::default() };
    let interrupted = app.with_server(|db| db.env_snapshots_interrupt(owner))?;
    report.snapshots_interrupted = interrupted.len();
    let store = store_of(app)?;
    let runner = match app.envs.runner(app) {
        Ok(runner) => Some(runner),
        Err(e) => {
            report.problems.push(format!("the environment runner is not available: {e}"));
            None
        }
    };
    let remove = |p: &Path| match &runner {
        Some(runner) => runner.remove_tree(p),
        None => store::remove_tree(p),
    };
    for (key, attempt) in &interrupted {
        if let Err(e) = store.discard_staging(key, *attempt, &remove) {
            report.problems.push(e);
        }
    }
    let Some(runner) = runner else { return Ok(report) };

    let resources = match runner.list() {
        Ok(resources) => resources,
        Err(e) => {
            report.problems.push(format!("cannot list the runtime's objects: {e}"));
            return Ok(report);
        }
    };
    let workspaces = app.with_server(|db| db.env_workspaces(None))?;
    let exists = |id: &str| workspaces.iter().find(|w| w.id == id).filter(|w| w.state != WsState::Deleted);
    let mut orphans: Vec<String> = resources.iter().filter(|r| exists(&r.workspace).is_none()).map(|r| r.workspace.clone()).collect();
    orphans.sort();
    orphans.dedup();
    for id in &orphans {
        match runner.remove(id) {
            Ok(()) => report.resources_removed += 1,
            Err(e) => report.problems.push(format!("cannot remove the runtime objects of {id}: {e}")),
        }
    }
    let running = |id: &str| resources.iter().any(|r| r.workspace == id && r.running && (r.kind == "workspace" || r.kind == "main"));
    for ws in &workspaces {
        match ws.state {
            WsState::Deleting => match finish_delete(app, runner.as_ref(), &store, ws) {
                Ok(_) => report.deletions_finished.push(ws.id.clone()),
                Err(e) => report.problems.push(format!("{}: cannot finish its deletion: {e}", ws.id)),
            },
            WsState::Requested | WsState::SourcesReady | WsState::EnvironmentReady | WsState::ServicesReady | WsState::Ready => {
                if !running(&ws.id) {
                    match app.with_server(|db| db.env_set_state(&ws.id, WsState::Stopped, None, Some(LOST))) {
                        Ok(_) => report.workspaces_stopped.push(ws.id.clone()),
                        Err(e) => report.problems.push(format!("{}: cannot record that it stopped: {e}", ws.id)),
                    }
                }
            }
            WsState::Stopped | WsState::Failed | WsState::Deleted => {}
        }
    }
    Ok(report)
}

/// What a garbage collection removed, or would remove in a dry run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcReport {
    pub dry_run: bool,
    /// Snapshot keys removed (or to remove).
    pub snapshots: Vec<String>,
    /// Staging directories of finished or abandoned attempts.
    pub staging: Vec<String>,
    /// Cache directories removed, oldest first.
    pub caches: Vec<String>,
    /// Bases the runner removed; a dry run cannot know them, so it lists none.
    pub bases: Vec<String>,
    /// Bytes of snapshots and caches removed (or that would be).
    pub freed: u64,
}

/// A snapshot as garbage collection sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SnapshotUse {
    pub key: String,
    pub last_used: String,
    pub size: u64,
    pub ready: bool,
    pub pinned: bool,
}

/// A cache directory as garbage collection sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CacheDir {
    pub path: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
}

/// The snapshots to remove so that the ready ones fit in `max` bytes: least recently used first.
/// Pinned snapshots are never chosen, even when that leaves the store over its limit.
pub(crate) fn select_snapshots(uses: &[SnapshotUse], max: u64) -> Vec<String> {
    let mut total: u64 = uses.iter().filter(|u| u.ready).map(|u| u.size).sum();
    let mut ordered: Vec<&SnapshotUse> = uses.iter().collect();
    ordered.sort_by(|a, b| a.last_used.cmp(&b.last_used).then_with(|| a.key.cmp(&b.key)));
    let mut victims = Vec::new();
    for u in ordered {
        if total <= max {
            break;
        }
        if u.ready && !u.pinned {
            total -= u.size;
            victims.push(u.key.clone());
        }
    }
    victims
}

/// The cache directories to remove so that all of them fit in `max` bytes: the oldest first.
pub(crate) fn select_caches(caches: &[CacheDir], max: u64) -> Vec<PathBuf> {
    let mut total: u64 = caches.iter().map(|c| c.size).sum();
    let mut ordered: Vec<&CacheDir> = caches.iter().collect();
    ordered.sort_by(|a, b| a.modified.cmp(&b.modified).then_with(|| a.path.cmp(&b.path)));
    let mut victims = Vec::new();
    for c in ordered {
        if total <= max {
            break;
        }
        total -= c.size;
        victims.push(c.path.clone());
    }
    victims
}

/// Garbage collection: snapshots over `storage.snapshotsMax` (least recently used first, never
/// pinned ones), staging left by attempts that ended an hour or more ago, caches over
/// `storage.cachesMax` (oldest first), and the bases no workspace runs on. A dry run changes nothing.
pub fn gc(app: &App, dry_run: bool) -> AppResult<GcReport> {
    let runner = app.envs.runner(app)?;
    let store = store_of(app)?;
    let storage = &app.cfg.environments.storage;
    let snapshots_max = parse_size(&storage.snapshots_max).map_err(AppError::Internal)?;
    let caches_max = parse_size(&storage.caches_max).map_err(AppError::Internal)?;
    let pinned = app.with_server(|db| db.env_pinned(PIN_SNAPSHOT))?;
    let pinned_bases = app.with_server(|db| db.env_pinned(PIN_BASE))?;
    let rows = app.with_server(|db| db.env_snapshots())?;
    let remove = |p: &Path| runner.remove_tree(p);
    let mut report = GcReport { dry_run, ..Default::default() };

    let uses: Vec<SnapshotUse> = rows
        .iter()
        .map(|s| SnapshotUse {
            key: s.key.clone(),
            last_used: s.last_used.clone(),
            size: s.size.unwrap_or(0).max(0) as u64,
            ready: s.state == SnapshotState::Ready,
            pinned: pinned.contains(&s.key),
        })
        .collect();
    for key in select_snapshots(&uses, snapshots_max) {
        let size = uses.iter().find(|u| u.key == key).map_or(0, |u| u.size);
        if !dry_run {
            // The row goes first: a snapshot that is gone from the database cannot be taken up again.
            if !app.with_server(|db| db.env_snapshot_delete(&key))? {
                continue;
            }
            store.remove_snapshot(&key, &remove).map_err(AppError::Internal)?;
        }
        report.freed += size;
        report.snapshots.push(key);
    }

    let staging = store.staging_leftovers().map_err(AppError::Internal)?;
    for path in staging {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let key = name.rsplit_once('.').map(|(key, _)| key);
        let in_use = key.is_some_and(|key| rows.iter().any(|s| s.key == key && s.state == SnapshotState::Preparing));
        if in_use || !is_older_than(&path, STAGING_GRACE) {
            continue;
        }
        if !dry_run {
            remove(&path).map_err(AppError::Internal)?;
        }
        report.staging.push(name);
    }

    let caches = cache_dirs(&store);
    for victim in select_caches(&caches, caches_max) {
        let size = caches.iter().find(|c| c.path == victim).map_or(0, |c| c.size);
        if !dry_run {
            remove(&victim).map_err(AppError::Internal)?;
        }
        report.freed += size;
        report.caches.push(victim.display().to_string());
    }

    if !dry_run {
        report.bases = runner.prune_bases(&pinned_bases).map_err(AppError::Internal)?;
    }
    Ok(report)
}

/// Every cache directory of the store (`caches/project-<slug>/<name>`) with its size and age.
fn cache_dirs(store: &store::Store) -> Vec<CacheDir> {
    let mut out = Vec::new();
    let Ok(projects) = std::fs::read_dir(store.root.join("caches")) else { return out };
    for project in projects.flatten() {
        let Ok(names) = std::fs::read_dir(project.path()) else { continue };
        for cache in names.flatten() {
            let path = cache.path();
            let modified = std::fs::metadata(&path).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            out.push(CacheDir { size: dir_size(&path).0, path, modified });
        }
    }
    out
}

/// Whether a path was last changed at least `age` ago.
fn is_older_than(path: &Path, age: Duration) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|elapsed| elapsed >= age)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(key: &str, last_used: &str, size: u64, pinned: bool) -> SnapshotUse {
        SnapshotUse { key: key.into(), last_used: last_used.into(), size, ready: true, pinned }
    }

    #[test]
    fn the_least_recently_used_snapshots_go_until_the_store_fits_and_pinned_ones_stay() {
        let uses =
            vec![snap("old", "2026-01-01", 40, false), snap("pinned", "2026-01-02", 40, true), snap("recent", "2026-03-01", 40, false)];
        // 120 bytes over a limit of 100: the oldest unpinned goes, and that is enough.
        assert_eq!(select_snapshots(&uses, 100), vec!["old".to_string()]);
        // Over a limit of 50: the pinned one stays, the recent one goes too.
        assert_eq!(select_snapshots(&uses, 50), vec!["old".to_string(), "recent".to_string()]);
        // Within the limit nothing goes.
        assert!(select_snapshots(&uses, 120).is_empty());
    }

    #[test]
    fn a_snapshot_that_is_not_ready_is_never_a_victim_and_does_not_count() {
        let mut failed = snap("failed", "2026-01-01", 500, false);
        failed.ready = false;
        let uses = vec![failed, snap("ready", "2026-02-01", 10, false)];
        assert!(select_snapshots(&uses, 10).is_empty(), "the failed row holds no bytes");
    }

    #[test]
    fn the_oldest_caches_go_until_they_fit() {
        let at = |secs: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
        let caches = vec![
            CacheDir { path: "/c/new".into(), size: 30, modified: at(300) },
            CacheDir { path: "/c/old".into(), size: 50, modified: at(100) },
            CacheDir { path: "/c/mid".into(), size: 40, modified: at(200) },
        ];
        assert_eq!(select_caches(&caches, 80), vec![PathBuf::from("/c/old")]);
        assert_eq!(select_caches(&caches, 30), vec![PathBuf::from("/c/old"), PathBuf::from("/c/mid")]);
        assert!(select_caches(&caches, 120).is_empty());
    }
}
