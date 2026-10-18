//! Environment state in `server.db`: workspaces, their stages, prepared snapshots and their
//! leases, the pins that keep snapshots and bases alive, and a project's environment secrets.
//!
//! The rules that matter for correctness live here. A snapshot is prepared by one owner at a
//! time (its lease, taken over only once it expired), a snapshot pinned by a live workspace is
//! never deleted, and a deleted workspace pins nothing. Each method is one statement or one
//! transaction; the controller decides what to do with the outcome.

use std::collections::BTreeSet;

use chrono::Duration as ChronoDuration;
use rusqlite::types::Type;
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use crate::db::now;
use crate::env::lock::WorkspaceLock;
use crate::env::spec::{valid_name, valid_secret_name};
use crate::error::{GenieError, Result};
use crate::secrets::{self, SecretInfo};
use crate::server_db::{ServerDb, time_in};

/// The pin kind of a snapshot a workspace uses (the ref is the snapshot key).
pub const PIN_SNAPSHOT: &str = "snapshot";
/// The pin kind of a base image a workspace runs on (the ref is the base reference).
pub const PIN_BASE: &str = "base";

/// The runners a workspace can be bound to (`genie::env::runner`).
const RUNNERS: &[&str] = &["podman", "host"];

/// Where a workspace is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WsState {
    Requested,
    SourcesReady,
    EnvironmentReady,
    ServicesReady,
    Ready,
    Stopped,
    Failed,
    Deleting,
    Deleted,
}

impl WsState {
    pub fn as_str(self) -> &'static str {
        match self {
            WsState::Requested => "requested",
            WsState::SourcesReady => "sources_ready",
            WsState::EnvironmentReady => "environment_ready",
            WsState::ServicesReady => "services_ready",
            WsState::Ready => "ready",
            WsState::Stopped => "stopped",
            WsState::Failed => "failed",
            WsState::Deleting => "deleting",
            WsState::Deleted => "deleted",
        }
    }

    pub fn parse(s: &str) -> Result<WsState> {
        Ok(match s {
            "requested" => WsState::Requested,
            "sources_ready" => WsState::SourcesReady,
            "environment_ready" => WsState::EnvironmentReady,
            "services_ready" => WsState::ServicesReady,
            "ready" => WsState::Ready,
            "stopped" => WsState::Stopped,
            "failed" => WsState::Failed,
            "deleting" => WsState::Deleting,
            "deleted" => WsState::Deleted,
            _ => return Err(GenieError::invalid(format!("unknown workspace state {s}"))),
        })
    }
}

/// A task's environment: one workspace per project and name.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvWorkspace {
    /// `<project>--<name>`.
    pub id: String,
    pub project: String,
    pub name: String,
    pub task: Option<String>,
    pub profile: String,
    pub runner: String,
    pub state: WsState,
    pub failed_stage: Option<String>,
    pub error: Option<String>,
    pub lock: Option<WorkspaceLock>,
    pub view_mode: Option<String>,
    pub size_exclusive: Option<i64>,
    pub size_logical: Option<i64>,
    pub created: String,
    pub updated: String,
}

impl EnvWorkspace {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<EnvWorkspace> {
        let lock: Option<String> = r.get("lock")?;
        Ok(EnvWorkspace {
            id: r.get("id")?,
            project: r.get("project")?,
            name: r.get("name")?,
            task: r.get("task")?,
            profile: r.get("profile")?,
            runner: r.get("runner")?,
            state: parsed(r, "state", WsState::parse)?,
            failed_stage: r.get("failed_stage")?,
            error: r.get("error")?,
            lock: lock.map(|text| serde_json::from_str(&text)).transpose().map_err(|e| conversion(r, "lock", e))?,
            view_mode: r.get("view_mode")?,
            size_exclusive: r.get("size_exclusive")?,
            size_logical: r.get("size_logical")?,
            created: r.get("created")?,
            updated: r.get("updated")?,
        })
    }
}

/// The workspace id of a project's workspace called `name`.
pub fn workspace_id(project: &str, name: &str) -> String {
    format!("{project}--{name}")
}

/// How a stage of a workspace stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StageStatus {
    Running,
    Done,
    Failed,
}

impl StageStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            StageStatus::Running => "running",
            StageStatus::Done => "done",
            StageStatus::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Result<StageStatus> {
        Ok(match s {
            "running" => StageStatus::Running,
            "done" => StageStatus::Done,
            "failed" => StageStatus::Failed,
            _ => return Err(GenieError::invalid(format!("unknown stage status {s}"))),
        })
    }
}

/// One stage of a workspace (`sources`, `environment`, `services`) and the input it ran on.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvStage {
    pub workspace: String,
    pub stage: String,
    /// The hash of the inputs the stage ran on; a `done` stage with the same hash is skipped.
    pub input_hash: String,
    pub status: StageStatus,
    pub detail: String,
    /// The process that started the stage.
    pub owner: String,
    pub started: String,
    pub finished: Option<String>,
}

impl EnvStage {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<EnvStage> {
        Ok(EnvStage {
            workspace: r.get("workspace")?,
            stage: r.get("stage")?,
            input_hash: r.get("input_hash")?,
            status: parsed(r, "status", StageStatus::parse)?,
            detail: r.get("detail")?,
            owner: r.get("owner")?,
            started: r.get("started")?,
            finished: r.get("finished")?,
        })
    }
}

/// What `env_stage_begin` decided: skip the stage, or run it now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageStart {
    /// A `done` stage with the same input hash exists.
    Done,
    /// The stage is recorded as running, owned by the caller.
    Begun,
}

/// Where a prepared snapshot is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SnapshotState {
    Preparing,
    Ready,
    Failed,
}

impl SnapshotState {
    pub fn as_str(self) -> &'static str {
        match self {
            SnapshotState::Preparing => "preparing",
            SnapshotState::Ready => "ready",
            SnapshotState::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Result<SnapshotState> {
        Ok(match s {
            "preparing" => SnapshotState::Preparing,
            "ready" => SnapshotState::Ready,
            "failed" => SnapshotState::Failed,
            _ => return Err(GenieError::invalid(format!("unknown snapshot state {s}"))),
        })
    }
}

/// A prepared step's outputs, shared by every workspace whose step has the same key.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvSnapshot {
    pub key: String,
    /// `project:<slug>`.
    pub scope: String,
    /// The step it prepares, for people (`<repository>/<step>`).
    pub label: String,
    pub state: SnapshotState,
    pub lease_owner: Option<String>,
    pub lease_until: Option<String>,
    /// How many times preparation was started for this key.
    pub attempt: i64,
    pub size: Option<i64>,
    pub files: Option<i64>,
    pub error: Option<String>,
    pub created: String,
    pub last_used: String,
}

impl EnvSnapshot {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<EnvSnapshot> {
        Ok(EnvSnapshot {
            key: r.get("key")?,
            scope: r.get("scope")?,
            label: r.get("label")?,
            state: parsed(r, "state", SnapshotState::parse)?,
            lease_owner: r.get("lease_owner")?,
            lease_until: r.get("lease_until")?,
            attempt: r.get("attempt")?,
            size: r.get("size")?,
            files: r.get("files")?,
            error: r.get("error")?,
            created: r.get("created")?,
            last_used: r.get("last_used")?,
        })
    }
}

/// What `env_snapshot_acquire` decided about a snapshot key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Acquire {
    /// The snapshot is ready; use it.
    Ready,
    /// The caller holds the lease and must prepare it. `attempt` counts the starts for this key.
    Acquired { attempt: i64 },
    /// Another owner holds an unexpired lease until `until`; wait and ask again.
    Busy { owner: String, until: String },
}

/// Parse a text column with `parse`, turning a bad value into a conversion error of the row.
fn parsed<T>(r: &Row<'_>, column: &str, parse: fn(&str) -> Result<T>) -> rusqlite::Result<T> {
    let text: String = r.get(column)?;
    parse(&text).map_err(|e| conversion(r, column, e))
}

/// A value of `column` that does not parse: the error names the column's index, as rusqlite does.
fn conversion(r: &Row<'_>, column: &str, e: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    let index = r.as_ref().column_index(column).unwrap_or(0);
    rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(e))
}

/// The additional authenticated data of a secret: a sealed value cannot be moved to another name.
fn env_secret_aad(project: &str, name: &str) -> String {
    format!("env-secret:{project}/{name}")
}

/// Whether a lease ending at `until` (RFC 3339, UTC, millisecond precision) has not passed yet.
/// All timestamps are written by `db::now` or `time_in`, so their text orders like their time.
fn live(until: Option<&str>, at: &str) -> bool {
    until.is_some_and(|u| u > at)
}

impl ServerDb {
    /// The workspace called `name` of `project`, created in state `requested` on first use.
    ///
    /// Idempotent for a live workspace. A deleted one is reset to `requested` with the given
    /// profile, runner and task, its stages and pins dropped. A live workspace bound to another
    /// profile or runner is refused: its environment would not match what was asked for.
    pub fn env_workspace_ensure(&self, project: &str, name: &str, task: Option<&str>, profile: &str, runner: &str) -> Result<EnvWorkspace> {
        self.project(project)?;
        if !valid_name(name) {
            return Err(GenieError::invalid(format!("workspace name {name}: lowercase letters, digits and dashes, at most 63 characters")));
        }
        if !valid_name(profile) {
            return Err(GenieError::invalid(format!("profile name {profile}: lowercase letters, digits and dashes")));
        }
        if !RUNNERS.contains(&runner) {
            return Err(GenieError::invalid(format!("unknown runner {runner}: use podman or host")));
        }
        let id = workspace_id(project, name);
        self.tx(|| {
            match self.env_workspace(&id)? {
                None => {
                    let at = now();
                    self.conn().execute(
                        "INSERT INTO env_workspaces(id, project, name, task, profile, runner, state, created, updated)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'requested', ?7, ?7)",
                        params![id, project, name, task, profile, runner, at],
                    )?;
                }
                Some(ws) if ws.state == WsState::Deleted => {
                    self.conn().execute("DELETE FROM env_stages WHERE workspace = ?1", [&id])?;
                    self.conn().execute("DELETE FROM env_pins WHERE workspace = ?1", [&id])?;
                    let at = now();
                    self.conn().execute(
                        "UPDATE env_workspaces SET task = ?2, profile = ?3, runner = ?4, state = 'requested', failed_stage = NULL,
                            error = NULL, lock = NULL, view_mode = NULL, size_exclusive = NULL, size_logical = NULL,
                            created = ?5, updated = ?5
                         WHERE id = ?1",
                        params![id, task, profile, runner, at],
                    )?;
                }
                Some(ws) if ws.profile != profile || ws.runner != runner => {
                    return Err(GenieError::invalid(format!(
                        "workspace {name} already exists with profile {} on runner {}; use another --name, or delete it first to change its profile or runner",
                        ws.profile, ws.runner
                    )));
                }
                Some(_) => {}
            }
            self.require_workspace(&id)
        })
    }

    /// The workspace with this id, if any (deleted ones included).
    pub fn env_workspace(&self, id: &str) -> Result<Option<EnvWorkspace>> {
        Ok(self.conn().query_row("SELECT * FROM env_workspaces WHERE id = ?1", [id], EnvWorkspace::from_row).optional()?)
    }

    fn require_workspace(&self, id: &str) -> Result<EnvWorkspace> {
        self.env_workspace(id)?.ok_or_else(|| GenieError::not_found(format!("workspace {id} not found")))
    }

    /// The workspaces of `project`, or of every project when none is given. Deleted workspaces
    /// are listed too: the caller filters on `state`.
    pub fn env_workspaces(&self, project: Option<&str>) -> Result<Vec<EnvWorkspace>> {
        let rows = match project {
            Some(p) => {
                let mut stmt = self.conn().prepare_cached("SELECT * FROM env_workspaces WHERE project = ?1 ORDER BY name")?;
                stmt.query_map([p], EnvWorkspace::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?
            }
            None => {
                let mut stmt = self.conn().prepare_cached("SELECT * FROM env_workspaces ORDER BY project, name")?;
                stmt.query_map([], EnvWorkspace::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?
            }
        };
        Ok(rows)
    }

    /// Move a workspace to `state`, recording the stage and error of a failure (`None` clears
    /// them). Leaving `deleted` is refused: `env_workspace_ensure` is the way back. Entering
    /// `deleted` drops the workspace's pins, so a deleted workspace keeps nothing alive.
    pub fn env_set_state(&self, id: &str, state: WsState, failed_stage: Option<&str>, error: Option<&str>) -> Result<EnvWorkspace> {
        self.tx(|| {
            let current = self.require_workspace(id)?;
            if current.state == WsState::Deleted && state != WsState::Deleted {
                return Err(GenieError::invalid(format!("workspace {id} is deleted; up it again to reuse its name")));
            }
            self.conn().execute(
                "UPDATE env_workspaces SET state = ?2, failed_stage = ?3, error = ?4, updated = ?5 WHERE id = ?1",
                params![id, state.as_str(), failed_stage, error, now()],
            )?;
            if state == WsState::Deleted {
                self.conn().execute("DELETE FROM env_pins WHERE workspace = ?1", [id])?;
            }
            self.require_workspace(id)
        })
    }

    /// Record the lock a workspace was built from, and its view mode.
    pub fn env_set_lock(&self, id: &str, lock: &WorkspaceLock, view_mode: &str) -> Result<()> {
        let json = serde_json::to_string(lock)?;
        let changed = self.conn().execute(
            "UPDATE env_workspaces SET lock = ?2, view_mode = ?3, updated = ?4 WHERE id = ?1",
            params![id, json, view_mode, now()],
        )?;
        if changed == 0 {
            return Err(GenieError::not_found(format!("workspace {id} not found")));
        }
        Ok(())
    }

    /// Record the disk use of a workspace: its own bytes, and its bytes with the snapshots it uses.
    pub fn env_set_sizes(&self, id: &str, exclusive: i64, logical: i64) -> Result<()> {
        let changed = self.conn().execute(
            "UPDATE env_workspaces SET size_exclusive = ?2, size_logical = ?3, updated = ?4 WHERE id = ?1",
            params![id, exclusive, logical, now()],
        )?;
        if changed == 0 {
            return Err(GenieError::not_found(format!("workspace {id} not found")));
        }
        Ok(())
    }

    /// Start a stage on `input_hash`, owned by `owner`. A `done` stage with the same hash is
    /// reported as `Done` and left alone; any other stage is (re)started as running.
    pub fn env_stage_begin(&self, workspace: &str, stage: &str, input_hash: &str, owner: &str) -> Result<StageStart> {
        self.tx(|| {
            self.require_workspace(workspace)?;
            let done = self
                .conn()
                .query_row(
                    "SELECT 1 FROM env_stages WHERE workspace = ?1 AND stage = ?2 AND input_hash = ?3 AND status = 'done'",
                    params![workspace, stage, input_hash],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if done {
                return Ok(StageStart::Done);
            }
            self.conn().execute(
                "INSERT INTO env_stages(workspace, stage, input_hash, status, detail, owner, started, finished)
                 VALUES (?1, ?2, ?3, 'running', '', ?4, ?5, NULL)
                 ON CONFLICT(workspace, stage) DO UPDATE SET input_hash = excluded.input_hash, status = 'running', detail = '',
                    owner = excluded.owner, started = excluded.started, finished = NULL",
                params![workspace, stage, input_hash, owner, now()],
            )?;
            Ok(StageStart::Begun)
        })
    }

    /// Finish a running stage: `done` when `ok`, otherwise `failed` with `detail` (the last lines of
    /// the log, say). A stage that is not running any more (interrupted) cannot be finished.
    pub fn env_stage_finish(&self, workspace: &str, stage: &str, ok: bool, detail: &str) -> Result<()> {
        let status = if ok { StageStatus::Done } else { StageStatus::Failed };
        let changed = self.conn().execute(
            "UPDATE env_stages SET status = ?3, detail = ?4, finished = ?5
             WHERE workspace = ?1 AND stage = ?2 AND status = 'running'",
            params![workspace, stage, status.as_str(), detail, now()],
        )?;
        if changed == 0 {
            return Err(GenieError::invalid(format!("stage {stage} of {workspace} is not running, so it cannot finish")));
        }
        Ok(())
    }

    /// The stages of a workspace, in the order they started.
    pub fn env_stages(&self, workspace: &str) -> Result<Vec<EnvStage>> {
        let mut stmt = self.conn().prepare_cached("SELECT * FROM env_stages WHERE workspace = ?1 ORDER BY started, stage")?;
        Ok(stmt.query_map([workspace], EnvStage::from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    /// Fail the running stages that belong to other owners: those processes are gone after a
    /// restart. Returns how many were failed.
    pub fn env_stages_interrupt(&self, owner_other_than: &str) -> Result<usize> {
        Ok(self.conn().execute(
            "UPDATE env_stages SET status = 'failed', detail = 'interrupted by a restart', finished = ?2
             WHERE status = 'running' AND owner <> ?1",
            params![owner_other_than, now()],
        )?)
    }

    /// Take the lease of a snapshot for `owner`, one transaction. A new key is `Acquired` at
    /// attempt 1. A ready snapshot is `Ready`. A live lease of another owner is `Busy`. A
    /// failed snapshot, or one whose lease expired, is taken over as `Acquired` at the next
    /// attempt.
    pub fn env_snapshot_acquire(&self, key: &str, scope: &str, label: &str, owner: &str, ttl_secs: u64) -> Result<Acquire> {
        self.tx(|| {
            let at = now();
            let until = time_in(ChronoDuration::seconds(ttl_secs as i64));
            let Some(s) = self.env_snapshot(key)? else {
                self.conn().execute(
                    "INSERT INTO env_snapshots(key, scope, label, state, lease_owner, lease_until, attempt, created, last_used)
                     VALUES (?1, ?2, ?3, 'preparing', ?4, ?5, 1, ?6, ?6)",
                    params![key, scope, label, owner, until, at],
                )?;
                return Ok(Acquire::Acquired { attempt: 1 });
            };
            match s.state {
                SnapshotState::Ready => Ok(Acquire::Ready),
                SnapshotState::Preparing if live(s.lease_until.as_deref(), &at) => {
                    Ok(Acquire::Busy { owner: s.lease_owner.unwrap_or_default(), until: s.lease_until.unwrap_or_default() })
                }
                SnapshotState::Preparing | SnapshotState::Failed => {
                    let attempt = s.attempt + 1;
                    self.conn().execute(
                        "UPDATE env_snapshots SET state = 'preparing', lease_owner = ?2, lease_until = ?3, attempt = ?4, error = NULL
                         WHERE key = ?1",
                        params![key, owner, until, attempt],
                    )?;
                    Ok(Acquire::Acquired { attempt })
                }
            }
        })
    }

    /// Extend the lease of `key` held by `owner`. `false` when the lease is no longer theirs
    /// (another owner took it over, or the snapshot was published or failed).
    pub fn env_snapshot_renew(&self, key: &str, owner: &str, ttl_secs: u64) -> Result<bool> {
        let until = time_in(ChronoDuration::seconds(ttl_secs as i64));
        let changed = self.conn().execute(
            "UPDATE env_snapshots SET lease_until = ?3 WHERE key = ?1 AND state = 'preparing' AND lease_owner = ?2",
            params![key, owner, until],
        )?;
        Ok(changed == 1)
    }

    /// Mark a snapshot ready with its size and file count. Only the lease holder may publish.
    pub fn env_snapshot_publish(&self, key: &str, owner: &str, size: i64, files: i64) -> Result<()> {
        let changed = self.conn().execute(
            "UPDATE env_snapshots SET state = 'ready', size = ?3, files = ?4, lease_owner = NULL, lease_until = NULL,
                error = NULL, last_used = ?5
             WHERE key = ?1 AND state = 'preparing' AND lease_owner = ?2",
            params![key, owner, size, files, now()],
        )?;
        if changed == 0 {
            return Err(not_leased(key));
        }
        Ok(())
    }

    /// Mark a snapshot failed with `error`. Only the lease holder may fail it.
    pub fn env_snapshot_fail(&self, key: &str, owner: &str, error: &str) -> Result<()> {
        let changed = self.conn().execute(
            "UPDATE env_snapshots SET state = 'failed', error = ?3, lease_owner = NULL, lease_until = NULL
             WHERE key = ?1 AND state = 'preparing' AND lease_owner = ?2",
            params![key, owner, error],
        )?;
        if changed == 0 {
            return Err(not_leased(key));
        }
        Ok(())
    }

    /// Record a use of a snapshot (its last use orders the garbage collection).
    pub fn env_snapshot_touch(&self, key: &str) -> Result<()> {
        let changed = self.conn().execute("UPDATE env_snapshots SET last_used = ?2 WHERE key = ?1", params![key, now()])?;
        if changed == 0 {
            return Err(GenieError::not_found(format!("snapshot {key} not found")));
        }
        Ok(())
    }

    pub fn env_snapshot(&self, key: &str) -> Result<Option<EnvSnapshot>> {
        Ok(self.conn().query_row("SELECT * FROM env_snapshots WHERE key = ?1", [key], EnvSnapshot::from_row).optional()?)
    }

    /// Every snapshot, least recently used first.
    pub fn env_snapshots(&self) -> Result<Vec<EnvSnapshot>> {
        let mut stmt = self.conn().prepare_cached("SELECT * FROM env_snapshots ORDER BY last_used, key")?;
        Ok(stmt.query_map([], EnvSnapshot::from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    /// Fail the snapshots being prepared by other owners (their processes are gone after a
    /// restart). Returns each failed key with the attempt that was interrupted.
    pub fn env_snapshots_interrupt(&self, owner_other_than: &str) -> Result<Vec<(String, i64)>> {
        self.tx(|| {
            let rows = {
                let mut stmt = self
                    .conn()
                    .prepare("SELECT key, attempt FROM env_snapshots WHERE state = 'preparing' AND lease_owner IS NOT ?1 ORDER BY key")?;
                stmt.query_map([owner_other_than], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?
            };
            self.conn().execute(
                "UPDATE env_snapshots SET state = 'failed', error = 'interrupted by a restart', lease_owner = NULL, lease_until = NULL
                 WHERE state = 'preparing' AND lease_owner IS NOT ?1",
                [owner_other_than],
            )?;
            Ok(rows)
        })
    }

    /// Delete a snapshot's row. Refused (`false`) while a live workspace pins it, and while it is
    /// being prepared. The caller removes the directory afterwards.
    pub fn env_snapshot_delete(&self, key: &str) -> Result<bool> {
        let changed = self.conn().execute(
            "DELETE FROM env_snapshots WHERE key = ?1 AND state <> 'preparing'
               AND NOT EXISTS (SELECT 1 FROM env_pins p JOIN env_workspaces w ON w.id = p.workspace
                               WHERE p.kind = 'snapshot' AND p.ref = ?1 AND w.state <> 'deleted')",
            [key],
        )?;
        Ok(changed > 0)
    }

    /// Replace the pins of a workspace: what its environment uses and must keep.
    pub fn env_pins_set(&self, workspace: &str, pins: &[(String, String)]) -> Result<()> {
        self.tx(|| {
            if self.require_workspace(workspace)?.state == WsState::Deleted {
                return Err(GenieError::invalid(format!("workspace {workspace} is deleted and cannot pin anything")));
            }
            self.conn().execute("DELETE FROM env_pins WHERE workspace = ?1", [workspace])?;
            for (kind, r#ref) in pins {
                self.conn()
                    .execute("INSERT OR IGNORE INTO env_pins(workspace, kind, ref) VALUES (?1, ?2, ?3)", params![workspace, kind, r#ref])?;
            }
            Ok(())
        })
    }

    /// The refs of `kind` pinned by workspaces that are not deleted.
    pub fn env_pinned(&self, kind: &str) -> Result<BTreeSet<String>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT DISTINCT p.ref FROM env_pins p JOIN env_workspaces w ON w.id = p.workspace
             WHERE p.kind = ?1 AND w.state <> 'deleted'",
        )?;
        Ok(stmt.query_map([kind], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
    }

    /// Set a project's environment secret (a blank value removes it). Its value is sealed like
    /// the other secrets and is only ever read back by the server, for steps granted it.
    pub fn env_secret_set(&self, project: &str, name: &str, value: &str) -> Result<()> {
        let value = value.trim();
        if value.is_empty() {
            return self.env_secret_delete(project, name);
        }
        if !valid_secret_name(name) {
            return Err(GenieError::invalid(format!(
                "secret name {name}: letters, digits, underscores and dashes, not starting with a dash, at most 64 characters"
            )));
        }
        secrets::check_value(value)?;
        self.project(project)?;
        let key = secrets::load_key(&self.secrets_key, true)?.expect("created");
        let sealed = secrets::seal(&key, &env_secret_aad(project, name), value)?;
        self.conn().execute(
            "INSERT INTO env_secrets(project, name, sealed, hint, updated) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(project, name) DO UPDATE SET sealed = excluded.sealed, hint = excluded.hint, updated = excluded.updated",
            params![project, name, sealed, secrets::hint(value), now()],
        )?;
        Ok(())
    }

    pub fn env_secret_delete(&self, project: &str, name: &str) -> Result<()> {
        self.conn().execute("DELETE FROM env_secrets WHERE project = ?1 AND name = ?2", params![project, name])?;
        Ok(())
    }

    /// A project's environment secret in the clear; `None` when unset or unreadable.
    pub fn env_secret(&self, project: &str, name: &str) -> Result<Option<String>> {
        let sealed: Option<String> = self
            .conn()
            .query_row("SELECT sealed FROM env_secrets WHERE project = ?1 AND name = ?2", params![project, name], |r| r.get(0))
            .optional()?;
        let Some(sealed) = sealed else { return Ok(None) };
        let Some(key) = secrets::load_key(&self.secrets_key, false)? else { return Ok(None) };
        Ok(secrets::unseal(&key, &env_secret_aad(project, name), &sealed))
    }

    /// What can be shown about a project's environment secrets, by name. Never the values.
    pub fn env_secret_names(&self, project: &str) -> Result<Vec<SecretInfo>> {
        let rows: Vec<(String, String, String, String)> = {
            let mut stmt =
                self.conn().prepare_cached("SELECT name, sealed, hint, updated FROM env_secrets WHERE project = ?1 ORDER BY name")?;
            stmt.query_map([project], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?
        };
        let key = secrets::load_key(&self.secrets_key, false)?;
        Ok(rows
            .into_iter()
            .map(|(name, sealed, hint, updated)| {
                let readable = key.as_ref().is_some_and(|k| secrets::unseal(k, &env_secret_aad(project, &name), &sealed).is_some());
                SecretInfo { name, hint, updated, unreadable: !readable }
            })
            .collect())
    }
}

/// The error of a snapshot write by an owner that no longer holds its lease.
fn not_leased(key: &str) -> GenieError {
    GenieError::invalid(format!(
        "snapshot {key} is no longer leased to this process: its lease expired and another process took it over, or it was already published or failed"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::lock::{BaseLock, RepoLock};

    fn db() -> (tempfile::TempDir, ServerDb) {
        let dir = tempfile::tempdir().unwrap();
        let db = ServerDb::open(&dir.path().join("server.db")).unwrap();
        db.create_project("shop", "Shop", "/tmp/shop-tracker", None, None).unwrap();
        (dir, db)
    }

    /// A workspace `shop--api` bound to the default profile on podman.
    fn workspace(db: &ServerDb) -> EnvWorkspace {
        db.env_workspace_ensure("shop", "api", Some("shop-1"), "default", "podman").unwrap()
    }

    fn lock() -> WorkspaceLock {
        WorkspaceLock {
            profile: "default".into(),
            profile_hash: "ph".into(),
            runner: "podman".into(),
            runner_version: "5.0.0".into(),
            base: BaseLock { reference: "genie-base:0123456789abcdef".into(), digest: "sha256:abc".into() },
            repos: vec![RepoLock { name: "api".into(), mount: ".".into(), sha: "f00d".into(), branch: None }],
            tools: Default::default(),
            snapshots: Default::default(),
            services: Default::default(),
            view_mode: "overlay".into(),
            created: "2026-10-17T10:00:00.000Z".into(),
        }
    }

    #[test]
    fn a_workspace_id_joins_the_project_and_the_name() {
        assert_eq!(workspace_id("shop", "task-shop-1"), "shop--task-shop-1");
    }

    #[test]
    fn a_workspace_starts_requested_and_ensuring_it_again_returns_it() {
        let (_dir, db) = db();
        let ws = workspace(&db);
        assert_eq!((ws.id.as_str(), ws.state, ws.task.as_deref()), ("shop--api", WsState::Requested, Some("shop-1")));
        assert!(ws.lock.is_none() && ws.error.is_none());

        let again = db.env_workspace_ensure("shop", "api", None, "default", "podman").unwrap();
        assert_eq!(again, ws, "a live workspace is returned as it is");
        assert_eq!(db.env_workspaces(Some("shop")).unwrap().len(), 1);
    }

    #[test]
    fn a_live_workspace_on_another_profile_or_runner_is_refused_with_the_way_out() {
        let (_dir, db) = db();
        workspace(&db);
        let err = db.env_workspace_ensure("shop", "api", None, "heavy", "podman").unwrap_err().to_string();
        assert!(err.contains("already exists with profile default on runner podman"), "{err}");
        assert!(err.contains("delete it first"), "{err}");
        let err = db.env_workspace_ensure("shop", "api", None, "default", "host").unwrap_err().to_string();
        assert!(err.contains("runner podman"), "{err}");
    }

    #[test]
    fn bad_names_unknown_runners_and_unknown_projects_are_refused() {
        let (_dir, db) = db();
        assert!(matches!(db.env_workspace_ensure("shop", "Api_X", None, "default", "podman"), Err(GenieError::Invalid(_))));
        assert!(matches!(db.env_workspace_ensure("shop", "api", None, "Bad Profile", "podman"), Err(GenieError::Invalid(_))));
        assert!(matches!(db.env_workspace_ensure("shop", "api", None, "default", "docker"), Err(GenieError::Invalid(_))));
        assert!(matches!(db.env_workspace_ensure("nope", "api", None, "default", "podman"), Err(GenieError::NotFound(_))));
        assert!(db.env_workspace("shop--api").unwrap().is_none(), "nothing is written for a refused request");
    }

    #[test]
    fn ensuring_a_deleted_workspace_resets_it_and_clears_its_stages_and_pins() {
        let (_dir, db) = db();
        let ws = workspace(&db);
        db.env_stage_begin(&ws.id, "environment", "h1", "owner-a").unwrap();
        db.env_stage_finish(&ws.id, "environment", true, "").unwrap();
        db.env_set_lock(&ws.id, &lock(), "overlay").unwrap();
        db.env_set_sizes(&ws.id, 10, 20).unwrap();
        db.env_pins_set(&ws.id, &[(PIN_SNAPSHOT.into(), "k1".into())]).unwrap();
        db.env_set_state(&ws.id, WsState::Failed, Some("services"), Some("boom")).unwrap();
        db.env_set_state(&ws.id, WsState::Deleted, None, None).unwrap();

        let back = db.env_workspace_ensure("shop", "api", Some("shop-2"), "heavy", "host").unwrap();
        assert_eq!(back.state, WsState::Requested);
        assert_eq!((back.profile.as_str(), back.runner.as_str(), back.task.as_deref()), ("heavy", "host", Some("shop-2")));
        assert!(back.lock.is_none() && back.error.is_none() && back.failed_stage.is_none());
        assert!(back.view_mode.is_none() && back.size_exclusive.is_none() && back.size_logical.is_none());
        assert!(db.env_stages(&ws.id).unwrap().is_empty(), "the stages of the old life are gone");
        assert!(db.env_pinned(PIN_SNAPSHOT).unwrap().is_empty());
    }

    #[test]
    fn a_workspace_moves_through_states_and_keeps_the_error_of_a_failure() {
        let (_dir, db) = db();
        let ws = workspace(&db);
        let failed = db.env_set_state(&ws.id, WsState::Failed, Some("environment"), Some("step web/install exited 2")).unwrap();
        assert_eq!((failed.state, failed.failed_stage.as_deref()), (WsState::Failed, Some("environment")));
        assert_eq!(failed.error.as_deref(), Some("step web/install exited 2"));

        let ready = db.env_set_state(&ws.id, WsState::Ready, None, None).unwrap();
        assert!(ready.failed_stage.is_none() && ready.error.is_none(), "a state without a failure clears it");

        let missing = db.env_set_state("shop--ghost", WsState::Ready, None, None).unwrap_err();
        assert!(matches!(missing, GenieError::NotFound(_)), "{missing}");
    }

    #[test]
    fn a_deleted_workspace_leaves_deleted_only_by_up() {
        let (_dir, db) = db();
        let ws = workspace(&db);
        db.env_set_state(&ws.id, WsState::Deleted, None, None).unwrap();
        let err = db.env_set_state(&ws.id, WsState::Ready, None, None).unwrap_err().to_string();
        assert!(err.contains("up it again"), "{err}");
    }

    #[test]
    fn the_lock_and_the_sizes_are_stored_and_read_back() {
        let (_dir, db) = db();
        let ws = workspace(&db);
        db.env_set_lock(&ws.id, &lock(), "copy").unwrap();
        db.env_set_sizes(&ws.id, 1024, 4096).unwrap();
        let got = db.env_workspace(&ws.id).unwrap().unwrap();
        assert_eq!(got.lock, Some(lock()));
        assert_eq!(got.view_mode.as_deref(), Some("copy"));
        assert_eq!((got.size_exclusive, got.size_logical), (Some(1024), Some(4096)));
        assert!(db.env_set_sizes("shop--ghost", 1, 1).is_err());
        assert!(db.env_set_lock("shop--ghost", &lock(), "copy").is_err());
    }

    #[test]
    fn a_stage_runs_once_per_input_and_is_skipped_when_done() {
        let (_dir, db) = db();
        let ws = workspace(&db);
        assert_eq!(db.env_stage_begin(&ws.id, "environment", "h1", "owner-a").unwrap(), StageStart::Begun);
        db.env_stage_finish(&ws.id, "environment", true, "").unwrap();
        assert_eq!(db.env_stage_begin(&ws.id, "environment", "h1", "owner-b").unwrap(), StageStart::Done);
        assert_eq!(db.env_stage_begin(&ws.id, "environment", "h2", "owner-b").unwrap(), StageStart::Begun, "new inputs run again");

        let stages = db.env_stages(&ws.id).unwrap();
        assert_eq!(stages.len(), 1);
        assert_eq!((stages[0].status, stages[0].input_hash.as_str(), stages[0].owner.as_str()), (StageStatus::Running, "h2", "owner-b"));
        assert!(stages[0].finished.is_none());
    }

    #[test]
    fn a_failed_stage_records_its_detail_and_runs_again() {
        let (_dir, db) = db();
        let ws = workspace(&db);
        db.env_stage_begin(&ws.id, "sources", "h1", "owner-a").unwrap();
        db.env_stage_finish(&ws.id, "sources", false, "clone failed: no such remote").unwrap();
        let stage = &db.env_stages(&ws.id).unwrap()[0];
        assert_eq!((stage.status, stage.detail.as_str()), (StageStatus::Failed, "clone failed: no such remote"));
        assert!(stage.finished.is_some());
        assert_eq!(db.env_stage_begin(&ws.id, "sources", "h1", "owner-a").unwrap(), StageStart::Begun, "a failure is not reused");
    }

    #[test]
    fn a_finished_stage_cannot_finish_again_and_an_unknown_one_is_refused() {
        let (_dir, db) = db();
        let ws = workspace(&db);
        db.env_stage_begin(&ws.id, "environment", "h1", "owner-a").unwrap();
        db.env_stage_finish(&ws.id, "environment", true, "").unwrap();
        assert!(matches!(db.env_stage_finish(&ws.id, "environment", true, ""), Err(GenieError::Invalid(_))));
        assert!(db.env_stage_finish(&ws.id, "services", true, "").is_err());
        assert!(db.env_stage_begin("shop--ghost", "environment", "h1", "owner-a").is_err());
    }

    #[test]
    fn running_stages_of_other_owners_are_interrupted_and_their_own_are_kept() {
        let (_dir, db) = db();
        let ws = workspace(&db);
        db.env_stage_begin(&ws.id, "sources", "h1", "old-process").unwrap();
        db.env_stage_begin(&ws.id, "environment", "h2", "this-process").unwrap();
        db.env_stage_begin(&ws.id, "services", "h3", "old-process").unwrap();
        db.env_stage_finish(&ws.id, "services", true, "").unwrap();

        assert_eq!(db.env_stages_interrupt("this-process").unwrap(), 1, "only the old process's running stage");
        let stages = db.env_stages(&ws.id).unwrap();
        let by = |name: &str| stages.iter().find(|s| s.stage == name).unwrap().clone();
        assert_eq!(by("sources").status, StageStatus::Failed);
        assert_eq!(by("sources").detail, "interrupted by a restart");
        assert_eq!(by("environment").status, StageStatus::Running, "our own stage is untouched");
        assert_eq!(by("services").status, StageStatus::Done, "a finished stage stays finished");
    }

    #[test]
    fn a_snapshot_is_acquired_once_and_a_second_owner_waits_for_the_lease() {
        let (_dir, db) = db();
        let first = db.env_snapshot_acquire("k1", "project:shop", "api/fetch", "owner-a", 600).unwrap();
        assert_eq!(first, Acquire::Acquired { attempt: 1 });

        match db.env_snapshot_acquire("k1", "project:shop", "api/fetch", "owner-b", 600).unwrap() {
            Acquire::Busy { owner, until } => {
                assert_eq!(owner, "owner-a");
                assert!(until > now(), "the lease runs into the future: {until}");
            }
            other => panic!("expected busy, got {other:?}"),
        }
        let snap = db.env_snapshot("k1").unwrap().unwrap();
        assert_eq!((snap.state, snap.attempt, snap.lease_owner.as_deref()), (SnapshotState::Preparing, 1, Some("owner-a")));
    }

    #[test]
    fn an_expired_lease_is_taken_over_at_the_next_attempt_and_the_old_owner_is_refused() {
        let (_dir, db) = db();
        assert_eq!(db.env_snapshot_acquire("k1", "project:shop", "api/fetch", "owner-a", 0).unwrap(), Acquire::Acquired { attempt: 1 });
        // A zero-second lease has already expired.
        assert_eq!(db.env_snapshot_acquire("k1", "project:shop", "api/fetch", "owner-b", 600).unwrap(), Acquire::Acquired { attempt: 2 });

        assert!(!db.env_snapshot_renew("k1", "owner-a", 600).unwrap(), "the old owner lost the lease");
        assert!(db.env_snapshot_publish("k1", "owner-a", 1, 1).is_err(), "and cannot publish");
        assert!(db.env_snapshot_fail("k1", "owner-a", "late").is_err(), "or fail");
        assert!(db.env_snapshot_renew("k1", "owner-b", 600).unwrap(), "the new owner keeps it");
    }

    #[test]
    fn publishing_makes_a_snapshot_ready_for_everyone_and_releases_the_lease() {
        let (_dir, db) = db();
        db.env_snapshot_acquire("k1", "project:shop", "api/fetch", "owner-a", 600).unwrap();
        db.env_snapshot_publish("k1", "owner-a", 4096, 12).unwrap();
        let snap = db.env_snapshot("k1").unwrap().unwrap();
        assert_eq!((snap.state, snap.size, snap.files), (SnapshotState::Ready, Some(4096), Some(12)));
        assert!(snap.lease_owner.is_none() && snap.lease_until.is_none());
        assert_eq!(db.env_snapshot_acquire("k1", "project:shop", "api/fetch", "owner-b", 600).unwrap(), Acquire::Ready);
        assert!(!db.env_snapshot_renew("k1", "owner-a", 600).unwrap(), "a published snapshot has no lease to renew");
    }

    #[test]
    fn a_failed_snapshot_keeps_its_error_and_is_prepared_again_at_the_next_attempt() {
        let (_dir, db) = db();
        db.env_snapshot_acquire("k1", "project:shop", "api/fetch", "owner-a", 600).unwrap();
        db.env_snapshot_fail("k1", "owner-a", "step api/fetch exited 2").unwrap();
        let snap = db.env_snapshot("k1").unwrap().unwrap();
        assert_eq!((snap.state, snap.error.as_deref()), (SnapshotState::Failed, Some("step api/fetch exited 2")));
        assert_eq!(db.env_snapshot_acquire("k1", "project:shop", "api/fetch", "owner-b", 600).unwrap(), Acquire::Acquired { attempt: 2 });
        assert!(db.env_snapshot("k1").unwrap().unwrap().error.is_none(), "the new attempt clears the old error");
    }

    #[test]
    fn a_snapshot_renewal_keeps_others_out() {
        let (_dir, db) = db();
        db.env_snapshot_acquire("k1", "project:shop", "api/fetch", "owner-a", 0).unwrap();
        assert!(db.env_snapshot_renew("k1", "owner-a", 600).unwrap());
        assert!(matches!(db.env_snapshot_acquire("k1", "project:shop", "api/fetch", "owner-b", 600).unwrap(), Acquire::Busy { .. }));
        assert!(!db.env_snapshot_renew("k1", "owner-b", 600).unwrap(), "only the holder renews");
        assert!(!db.env_snapshot_renew("missing", "owner-a", 600).unwrap());
    }

    #[test]
    fn snapshots_of_other_owners_are_interrupted_and_ready_ones_are_kept() {
        let (_dir, db) = db();
        db.env_snapshot_acquire("k-old", "project:shop", "api/fetch", "old-process", 600).unwrap();
        db.env_snapshot_acquire("k-mine", "project:shop", "api/build", "this-process", 600).unwrap();
        db.env_snapshot_acquire("k-done", "project:shop", "api/test", "old-process", 600).unwrap();
        db.env_snapshot_publish("k-done", "old-process", 1, 1).unwrap();

        assert_eq!(db.env_snapshots_interrupt("this-process").unwrap(), vec![("k-old".to_string(), 1)]);
        assert_eq!(db.env_snapshot("k-old").unwrap().unwrap().state, SnapshotState::Failed);
        assert_eq!(db.env_snapshot("k-mine").unwrap().unwrap().state, SnapshotState::Preparing);
        assert_eq!(db.env_snapshot("k-done").unwrap().unwrap().state, SnapshotState::Ready);
    }

    #[test]
    fn touching_a_snapshot_records_its_use_and_a_missing_one_is_not_found() {
        let (_dir, db) = db();
        db.env_snapshot_acquire("k1", "project:shop", "api/fetch", "owner-a", 600).unwrap();
        db.env_snapshot_publish("k1", "owner-a", 1, 1).unwrap();
        db.env_snapshot_touch("k1").unwrap();
        assert!(db.env_snapshot("k1").unwrap().unwrap().last_used >= db.env_snapshot("k1").unwrap().unwrap().created);
        assert!(matches!(db.env_snapshot_touch("ghost"), Err(GenieError::NotFound(_))));
        assert!(db.env_snapshot("ghost").unwrap().is_none());
        assert_eq!(db.env_snapshots().unwrap().len(), 1);
    }

    #[test]
    fn a_pinned_snapshot_is_not_deleted_until_its_workspace_is_deleted() {
        let (_dir, db) = db();
        let ws = workspace(&db);
        db.env_snapshot_acquire("k1", "project:shop", "api/fetch", "owner-a", 600).unwrap();
        db.env_snapshot_publish("k1", "owner-a", 1, 1).unwrap();
        db.env_pins_set(&ws.id, &[(PIN_SNAPSHOT.into(), "k1".into())]).unwrap();

        assert!(!db.env_snapshot_delete("k1").unwrap(), "a live workspace pins it");
        assert!(db.env_snapshot("k1").unwrap().is_some());

        db.env_set_state(&ws.id, WsState::Stopped, None, None).unwrap();
        assert!(!db.env_snapshot_delete("k1").unwrap(), "a stopped workspace still pins it");

        db.env_set_state(&ws.id, WsState::Deleted, None, None).unwrap();
        assert!(db.env_pinned(PIN_SNAPSHOT).unwrap().is_empty());
        assert!(db.env_snapshot_delete("k1").unwrap());
        assert!(db.env_snapshot("k1").unwrap().is_none());
        assert!(!db.env_snapshot_delete("k1").unwrap(), "deleting twice finds nothing");
    }

    #[test]
    fn a_snapshot_being_prepared_is_not_deleted() {
        let (_dir, db) = db();
        db.env_snapshot_acquire("k1", "project:shop", "api/fetch", "owner-a", 600).unwrap();
        assert!(!db.env_snapshot_delete("k1").unwrap());
        db.env_snapshot_fail("k1", "owner-a", "gone").unwrap();
        assert!(db.env_snapshot_delete("k1").unwrap(), "a failed one can go");
    }

    #[test]
    fn pins_are_replaced_per_workspace_and_filtered_by_kind() {
        let (_dir, db) = db();
        let a = workspace(&db);
        let b = db.env_workspace_ensure("shop", "web", None, "default", "podman").unwrap();
        db.env_pins_set(&a.id, &[(PIN_SNAPSHOT.into(), "k1".into()), (PIN_BASE.into(), "genie-base:aaaa".into())]).unwrap();
        db.env_pins_set(&b.id, &[(PIN_SNAPSHOT.into(), "k1".into()), (PIN_SNAPSHOT.into(), "k2".into())]).unwrap();
        assert_eq!(db.env_pinned(PIN_SNAPSHOT).unwrap(), BTreeSet::from(["k1".to_string(), "k2".to_string()]));
        assert_eq!(db.env_pinned(PIN_BASE).unwrap(), BTreeSet::from(["genie-base:aaaa".to_string()]));

        db.env_pins_set(&a.id, &[]).unwrap();
        assert_eq!(db.env_pinned(PIN_BASE).unwrap(), BTreeSet::new(), "a replacement drops the old pins");
        assert_eq!(db.env_pinned(PIN_SNAPSHOT).unwrap(), BTreeSet::from(["k1".to_string(), "k2".to_string()]));
        assert!(db.env_pins_set("shop--ghost", &[]).is_err());
    }

    #[test]
    fn a_deleted_workspace_pins_nothing_and_cannot_pin_again() {
        let (_dir, db) = db();
        let ws = workspace(&db);
        db.env_pins_set(&ws.id, &[(PIN_BASE.into(), "genie-base:aaaa".into())]).unwrap();
        db.env_set_state(&ws.id, WsState::Deleted, None, None).unwrap();
        assert!(db.env_pinned(PIN_BASE).unwrap().is_empty());
        let err = db.env_pins_set(&ws.id, &[(PIN_BASE.into(), "x".into())]).unwrap_err().to_string();
        assert!(err.contains("deleted"), "{err}");
    }

    #[test]
    fn a_workspace_goes_with_its_project_and_its_pins() {
        let (_dir, db) = db();
        let ws = workspace(&db);
        db.env_pins_set(&ws.id, &[(PIN_BASE.into(), "genie-base:aaaa".into())]).unwrap();
        db.conn().execute("DELETE FROM projects WHERE slug = 'shop'", []).unwrap();
        assert!(db.env_workspace(&ws.id).unwrap().is_none());
        let pins: i64 = db.conn().query_row("SELECT COUNT(*) FROM env_pins", [], |r| r.get(0)).unwrap();
        assert_eq!(pins, 0);
    }

    #[test]
    fn a_secret_round_trips_sealed_and_shows_only_its_hint() {
        let (_dir, db) = db();
        db.env_secret_set("shop", "npm_token", "  npm_live_0123456789abcdef  ").unwrap();
        assert_eq!(db.env_secret("shop", "npm_token").unwrap().as_deref(), Some("npm_live_0123456789abcdef"));

        let raw: String = db.conn().query_row("SELECT sealed FROM env_secrets WHERE name = 'npm_token'", [], |r| r.get(0)).unwrap();
        assert!(!raw.contains("npm_live"), "the value is not stored in the clear");

        let names = db.env_secret_names("shop").unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!((names[0].name.as_str(), names[0].hint.as_str(), names[0].unreadable), ("npm_token", "…cdef", false));
        assert!(db.env_secret("shop", "missing").unwrap().is_none());
    }

    #[test]
    fn a_secret_is_bound_to_its_project_and_name_and_a_blank_value_removes_it() {
        let (_dir, db) = db();
        db.create_project("other", "Other", "/tmp/other-tracker", None, None).unwrap();
        db.env_secret_set("shop", "deploy_key", "0123456789abcdef").unwrap();
        assert!(db.env_secret("other", "deploy_key").unwrap().is_none(), "another project does not see it");

        // A sealed value moved to another name cannot be opened under that name.
        db.env_secret_set("shop", "other_key", "fedcba9876543210").unwrap();
        db.conn()
            .execute(
                "UPDATE env_secrets SET sealed = (SELECT sealed FROM env_secrets WHERE name = 'deploy_key') WHERE name = 'other_key'",
                [],
            )
            .unwrap();
        assert!(db.env_secret("shop", "other_key").unwrap().is_none());
        assert_eq!(db.env_secret_names("shop").unwrap().iter().filter(|s| s.unreadable).count(), 1);

        db.env_secret_set("shop", "deploy_key", "   ").unwrap();
        assert!(db.env_secret("shop", "deploy_key").unwrap().is_none(), "a blank value removes the secret");
    }

    #[test]
    fn a_secret_is_refused_when_its_name_or_value_is_not_allowed() {
        let (_dir, db) = db();
        assert!(matches!(db.env_secret_set("shop", "-bad", "0123456789abcdef"), Err(GenieError::Invalid(_))));
        assert!(matches!(db.env_secret_set("shop", "ok", "line\nbreak"), Err(GenieError::Invalid(_))));
        assert!(db.env_secret_set("nope", "ok", "0123456789abcdef").is_err());
    }
}
