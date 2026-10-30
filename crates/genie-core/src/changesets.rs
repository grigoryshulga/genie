//! Documentation changesets and the merges that bring them onto `main`.
//!
//! A *changeset* is one branch of documentation edits (`changes/<id>`) an author opened
//! against a `main` commit. It stays open while it is edited, is published as a commit on
//! `main`, and may then be reviewed by a person. A *merge* records one attempt to bring a
//! changeset's branch up to date with `main`, including the pages that conflicted.

use std::collections::BTreeSet;

use rusqlite::{OptionalExtension, Row, params};
use serde::Serialize;

use crate::db::now;
use crate::error::{GenieError, Result};
use crate::server_db::ServerDb;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum ChangesetStatus {
    Open,
    Merging,
    Published,
    Discarded,
}

impl ChangesetStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangesetStatus::Open => "open",
            ChangesetStatus::Merging => "merging",
            ChangesetStatus::Published => "published",
            ChangesetStatus::Discarded => "discarded",
        }
    }

    pub fn parse(s: &str) -> Result<ChangesetStatus> {
        Ok(match s {
            "open" => ChangesetStatus::Open,
            "merging" => ChangesetStatus::Merging,
            "published" => ChangesetStatus::Published,
            "discarded" => ChangesetStatus::Discarded,
            _ => return Err(GenieError::invalid(format!("unknown changeset status {s}"))),
        })
    }
}

/// What opened a changeset: a person or agent in genie, a remote contribution, or one of
/// the genie-made changesets (a migration, a revert, a restore, a merge of `main`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum ChangesetOrigin {
    Genie,
    Remote,
    Migration,
    Revert,
    Restore,
    Merge,
}

impl ChangesetOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangesetOrigin::Genie => "genie",
            ChangesetOrigin::Remote => "remote",
            ChangesetOrigin::Migration => "migration",
            ChangesetOrigin::Revert => "revert",
            ChangesetOrigin::Restore => "restore",
            ChangesetOrigin::Merge => "merge",
        }
    }

    pub fn parse(s: &str) -> Result<ChangesetOrigin> {
        Ok(match s {
            "genie" => ChangesetOrigin::Genie,
            "remote" => ChangesetOrigin::Remote,
            "migration" => ChangesetOrigin::Migration,
            "revert" => ChangesetOrigin::Revert,
            "restore" => ChangesetOrigin::Restore,
            "merge" => ChangesetOrigin::Merge,
            _ => return Err(GenieError::invalid(format!("unknown changeset origin {s}"))),
        })
    }
}

/// How a reviewer closed a published changeset: accepted as it is, or reverted on `main`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum ReviewDecision {
    Reviewed,
    Reverted,
}

impl ReviewDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewDecision::Reviewed => "reviewed",
            ReviewDecision::Reverted => "reverted",
        }
    }

    pub fn parse(s: &str) -> Result<ReviewDecision> {
        Ok(match s {
            "reviewed" => ReviewDecision::Reviewed,
            "reverted" => ReviewDecision::Reverted,
            _ => return Err(GenieError::invalid(format!("unknown review decision {s}"))),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum MergeStatus {
    Pending,
    Running,
    Resolved,
    Failed,
}

impl MergeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MergeStatus::Pending => "pending",
            MergeStatus::Running => "running",
            MergeStatus::Resolved => "resolved",
            MergeStatus::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Result<MergeStatus> {
        Ok(match s {
            "pending" => MergeStatus::Pending,
            "running" => MergeStatus::Running,
            "resolved" => MergeStatus::Resolved,
            "failed" => MergeStatus::Failed,
            _ => return Err(GenieError::invalid(format!("unknown merge status {s}"))),
        })
    }
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Changeset {
    pub id: i64,
    /// The git branch, `changes/<id>` unless the caller named it.
    pub branch: String,
    pub project: Option<String>,
    pub task: Option<String>,
    /// Login of the author.
    pub author: String,
    pub author_name: String,
    /// `human`, `agent` or `system`.
    pub author_kind: String,
    pub title: String,
    pub status: ChangesetStatus,
    /// The `main` commit the branch forked from.
    pub base: String,
    /// Page paths the changeset touches, sorted and unique.
    pub paths: Vec<String>,
    pub origin: ChangesetOrigin,
    /// The changeset this one reverts.
    pub reverts: Option<i64>,
    pub created: String,
    pub updated: String,
    pub published_at: Option<String>,
    pub published_commit: Option<String>,
    /// Who published or discarded it.
    pub closed_by: Option<String>,
    pub reminded_at: Option<String>,
    pub review_needed: bool,
    pub reviewed_by: Option<String>,
    pub reviewed_at: Option<String>,
    pub review_decision: Option<ReviewDecision>,
}

impl Changeset {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<Changeset> {
        Ok(Changeset {
            id: r.get("id")?,
            branch: r.get("branch")?,
            project: r.get("project")?,
            task: r.get("task")?,
            author: r.get("author")?,
            author_name: r.get("author_name")?,
            author_kind: r.get("author_kind")?,
            title: r.get("title")?,
            status: decode(r, "status", ChangesetStatus::parse)?,
            base: r.get("base")?,
            paths: decode(r, "paths", parse_list)?,
            origin: decode(r, "origin", ChangesetOrigin::parse)?,
            reverts: r.get("reverts")?,
            created: r.get("created")?,
            updated: r.get("updated")?,
            published_at: r.get("published_at")?,
            published_commit: r.get("published_commit")?,
            closed_by: r.get("closed_by")?,
            reminded_at: r.get("reminded_at")?,
            review_needed: r.get::<_, i64>("review_needed")? != 0,
            reviewed_by: r.get("reviewed_by")?,
            reviewed_at: r.get("reviewed_at")?,
            review_decision: decode_opt(r, "review_decision", ReviewDecision::parse)?,
        })
    }
}

/// One attempt to bring a changeset's branch up to `main`. `conflicts` lists the pages that
/// could not be merged automatically.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocMerge {
    pub id: i64,
    pub changeset: i64,
    pub status: MergeStatus,
    /// The `main` commit merged against.
    pub main: String,
    /// The changeset branch tip that was merged.
    pub tip: String,
    pub conflicts: Vec<String>,
    pub attempts: i64,
    pub error: Option<String>,
    pub resolved_by: Option<String>,
    /// The commit that resolved the merge.
    pub resolution: Option<String>,
    /// Whether the resolution contradicts one side of the change.
    pub contradiction: bool,
    pub note: String,
    /// The doc-merger job working on this merge.
    pub job: Option<i64>,
    pub created: String,
    pub updated: String,
}

impl DocMerge {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<DocMerge> {
        Ok(DocMerge {
            id: r.get("id")?,
            changeset: r.get("changeset")?,
            status: decode(r, "status", MergeStatus::parse)?,
            main: r.get("main")?,
            tip: r.get("tip")?,
            conflicts: decode(r, "conflicts", parse_list)?,
            attempts: r.get("attempts")?,
            error: r.get("error")?,
            resolved_by: r.get("resolved_by")?,
            resolution: r.get("resolution")?,
            contradiction: r.get::<_, i64>("contradiction")? != 0,
            note: r.get("note")?,
            job: r.get("job")?,
            created: r.get("created")?,
            updated: r.get("updated")?,
        })
    }
}

/// The fields a new changeset is opened with. An empty `branch` is named `changes/<id>`
/// once the id is allocated.
pub struct NewChangeset<'a> {
    pub branch: &'a str,
    pub project: Option<&'a str>,
    pub task: Option<&'a str>,
    pub author: &'a str,
    pub author_name: &'a str,
    pub author_kind: &'a str,
    pub title: &'a str,
    pub base: &'a str,
    pub origin: ChangesetOrigin,
    pub reverts: Option<i64>,
}

/// Which changesets to list. An empty list or `None` leaves that condition open; `limit` 0 means 100.
#[derive(Default)]
pub struct ChangesetFilter {
    /// Keep changesets that touch a page under one of these roots (the first path segment).
    pub project_roots: Option<Vec<String>>,
    pub statuses: Vec<ChangesetStatus>,
    pub task: Option<String>,
    pub author: Option<String>,
    /// Published, flagged for review and not yet reviewed.
    pub unreviewed: bool,
    /// Keep changesets that touch exactly this page.
    pub path: Option<String>,
    pub limit: i64,
}

/// Decodes a text column with `parse`, reporting a bad value as a conversion failure of that column.
fn decode<T>(r: &Row<'_>, column: &str, parse: fn(&str) -> Result<T>) -> rusqlite::Result<T> {
    let text: String = r.get(column)?;
    parse(&text).map_err(|e| conversion(r, column, e))
}

fn decode_opt<T>(r: &Row<'_>, column: &str, parse: fn(&str) -> Result<T>) -> rusqlite::Result<Option<T>> {
    let text: Option<String> = r.get(column)?;
    text.map(|t| parse(&t).map_err(|e| conversion(r, column, e))).transpose()
}

fn conversion(r: &Row<'_>, column: &str, e: GenieError) -> rusqlite::Error {
    match r.as_ref().column_index(column) {
        Ok(index) => rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(e)),
        Err(err) => err,
    }
}

fn parse_list(s: &str) -> Result<Vec<String>> {
    Ok(serde_json::from_str(s)?)
}

impl ServerDb {
    /// Opens a changeset. The id is allocated inside the transaction, so an empty branch is
    /// replaced by `changes/<id>` before the transaction commits.
    pub fn create_changeset(&self, new: &NewChangeset<'_>) -> Result<Changeset> {
        self.tx(|| {
            let at = now();
            self.conn().execute(
                "INSERT INTO doc_changesets(branch, project, task, author, author_name, author_kind, title, status, base, origin, reverts, created, updated)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'open', ?8, ?9, ?10, ?11, ?11)",
                params![
                    new.branch,
                    new.project,
                    new.task,
                    new.author,
                    new.author_name,
                    new.author_kind,
                    new.title,
                    new.base,
                    new.origin.as_str(),
                    new.reverts,
                    at
                ],
            )?;
            let id = self.conn().last_insert_rowid();
            if new.branch.is_empty() {
                self.conn().execute("UPDATE doc_changesets SET branch = ?1 WHERE id = ?2", params![format!("changes/{id}"), id])?;
            }
            self.changeset(id)
        })
    }

    pub fn changeset(&self, id: i64) -> Result<Changeset> {
        self.conn()
            .query_row("SELECT * FROM doc_changesets WHERE id = ?1", [id], Changeset::from_row)
            .optional()?
            .ok_or_else(|| GenieError::not_found(format!("changeset {id} not found")))
    }

    pub fn changeset_by_branch(&self, branch: &str) -> Result<Option<Changeset>> {
        Ok(self.conn().query_row("SELECT * FROM doc_changesets WHERE branch = ?1", [branch], Changeset::from_row).optional()?)
    }

    /// Changesets newest first, filtered as `f` describes.
    pub fn changesets(&self, f: &ChangesetFilter) -> Result<Vec<Changeset>> {
        let roots = serde_json::to_string(f.project_roots.as_deref().unwrap_or_default())?;
        let statuses: Vec<&str> = f.statuses.iter().map(|s| s.as_str()).collect();
        let statuses = serde_json::to_string(&statuses)?;
        let limit = if f.limit > 0 { f.limit } else { 100 };
        let mut stmt = self.conn().prepare(
            "SELECT * FROM doc_changesets c
             WHERE (?1 = 0 OR EXISTS (SELECT 1 FROM json_each(c.paths) p
                 WHERE substr(p.value, 1, instr(p.value, '/') - 1) IN (SELECT value FROM json_each(?2))))
             AND (?3 = 0 OR c.status IN (SELECT value FROM json_each(?4)))
             AND (?5 IS NULL OR c.task = ?5)
             AND (?6 IS NULL OR c.author = ?6)
             AND (?7 = 0 OR (c.status = 'published' AND c.review_needed = 1 AND c.review_decision IS NULL))
             AND (?8 IS NULL OR EXISTS (SELECT 1 FROM json_each(c.paths) p WHERE p.value = ?8))
             ORDER BY c.id DESC LIMIT ?9",
        )?;
        let rows = stmt.query_map(
            params![
                f.project_roots.is_some(),
                roots,
                !f.statuses.is_empty(),
                statuses,
                f.task.as_deref(),
                f.author.as_deref(),
                f.unreviewed,
                f.path.as_deref(),
                limit
            ],
            Changeset::from_row,
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The author's open or merging changeset for `task` (`None` means one without a task),
    /// optionally one that touches `path`. Newest first.
    pub fn open_changeset_for(&self, author: &str, task: Option<&str>, path: Option<&str>) -> Result<Option<Changeset>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT * FROM doc_changesets c
                 WHERE c.author = ?1 AND c.task IS ?2 AND c.status IN ('open', 'merging')
                 AND (?3 IS NULL OR EXISTS (SELECT 1 FROM json_each(c.paths) p WHERE p.value = ?3))
                 ORDER BY c.id DESC LIMIT 1",
                params![author, task, path],
                Changeset::from_row,
            )
            .optional()?)
    }

    /// Adds page paths to an open changeset. The paths stay sorted and unique.
    pub fn touch_changeset(&self, id: i64, paths: &[String]) -> Result<Changeset> {
        self.tx(|| {
            let current = self.changeset(id)?;
            if current.status != ChangesetStatus::Open {
                return Err(GenieError::invalid(format!(
                    "changeset {id} is {}; only an open changeset takes new paths",
                    current.status.as_str()
                )));
            }
            let merged: BTreeSet<String> = current.paths.into_iter().chain(paths.iter().cloned()).collect();
            self.conn().execute(
                "UPDATE doc_changesets SET paths = ?1, updated = ?2 WHERE id = ?3",
                params![serde_json::to_string(&merged)?, now(), id],
            )?;
            self.changeset(id)
        })
    }

    /// Moves a changeset between `open` and `merging`, or discards it. Publishing goes through
    /// `mark_published`, and a discarded or published changeset is closed for good.
    pub fn set_changeset_status(&self, id: i64, status: ChangesetStatus, by: Option<&str>) -> Result<Changeset> {
        self.tx(|| {
            let current = self.changeset(id)?;
            if status == ChangesetStatus::Published {
                return Err(GenieError::invalid(format!("changeset {id} is published with mark_published")));
            }
            if matches!(current.status, ChangesetStatus::Published | ChangesetStatus::Discarded) {
                return Err(GenieError::invalid(format!("changeset {id} is already {}", current.status.as_str())));
            }
            self.conn().execute(
                "UPDATE doc_changesets SET status = ?1, closed_by = CASE WHEN ?1 = 'discarded' THEN ?2 ELSE closed_by END, updated = ?3
                 WHERE id = ?4",
                params![status.as_str(), by, now(), id],
            )?;
            self.changeset(id)
        })
    }

    /// Records that the changeset landed on `main` as `commit`.
    pub fn mark_published(&self, id: i64, commit: &str, review_needed: bool, by: &str) -> Result<Changeset> {
        self.tx(|| {
            let current = self.changeset(id)?;
            if !matches!(current.status, ChangesetStatus::Open | ChangesetStatus::Merging) {
                return Err(GenieError::invalid(format!("changeset {id} is {}; only an open one is published", current.status.as_str())));
            }
            let at = now();
            self.conn().execute(
                "UPDATE doc_changesets SET status = 'published', published_at = ?1, published_commit = ?2, review_needed = ?3,
                 closed_by = ?4, updated = ?1 WHERE id = ?5",
                params![at, commit, review_needed, by, id],
            )?;
            self.changeset(id)
        })
    }

    /// Records a reviewer's decision on a published changeset.
    pub fn mark_reviewed(&self, id: i64, by: &str, decision: ReviewDecision) -> Result<Changeset> {
        self.tx(|| {
            let current = self.changeset(id)?;
            if current.status != ChangesetStatus::Published {
                return Err(GenieError::invalid(format!(
                    "changeset {id} is {}; only a published one is reviewed",
                    current.status.as_str()
                )));
            }
            let at = now();
            self.conn().execute(
                "UPDATE doc_changesets SET reviewed_by = ?1, reviewed_at = ?2, review_decision = ?3, updated = ?2 WHERE id = ?4",
                params![by, at, decision.as_str(), id],
            )?;
            self.changeset(id)
        })
    }

    pub fn mark_reminded(&self, id: i64) -> Result<()> {
        let n = self.conn().execute("UPDATE doc_changesets SET reminded_at = ?1 WHERE id = ?2", params![now(), id])?;
        if n == 0 {
            return Err(GenieError::not_found(format!("changeset {id} not found")));
        }
        Ok(())
    }

    /// Open changesets untouched since `before` that have not been reminded since their last update.
    /// Both timestamps use the RFC 3339 format of `now()`, so comparing them as text orders them.
    pub fn idle_changesets(&self, before: &str) -> Result<Vec<Changeset>> {
        let mut stmt = self.conn().prepare(
            "SELECT * FROM doc_changesets
             WHERE status = 'open' AND updated < ?1 AND (reminded_at IS NULL OR reminded_at < updated)
             ORDER BY updated, id",
        )?;
        Ok(stmt.query_map([before], Changeset::from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    /// Starts a merge of `tip` into `main` for a changeset, recording the conflicting pages.
    pub fn create_merge(&self, changeset: i64, main: &str, tip: &str, conflicts: &[String]) -> Result<DocMerge> {
        self.tx(|| {
            self.changeset(changeset)?;
            let at = now();
            self.conn().execute(
                "INSERT INTO doc_merges(changeset, status, main, tip, conflicts, created, updated)
                 VALUES (?1, 'pending', ?2, ?3, ?4, ?5, ?5)",
                params![changeset, main, tip, serde_json::to_string(conflicts)?, at],
            )?;
            self.doc_merge(self.conn().last_insert_rowid())
        })
    }

    pub fn doc_merge(&self, id: i64) -> Result<DocMerge> {
        self.conn()
            .query_row("SELECT * FROM doc_merges WHERE id = ?1", [id], DocMerge::from_row)
            .optional()?
            .ok_or_else(|| GenieError::not_found(format!("merge {id} not found")))
    }

    /// The latest merge of a changeset that still needs work: pending, running or failed.
    pub fn active_merge(&self, changeset: i64) -> Result<Option<DocMerge>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT * FROM doc_merges WHERE changeset = ?1 AND status IN ('pending', 'running', 'failed') ORDER BY id DESC LIMIT 1",
                [changeset],
                DocMerge::from_row,
            )
            .optional()?)
    }

    /// Merges with one of `statuses` (any status when empty), newest first. `limit` 0 means 100.
    pub fn merges(&self, statuses: &[MergeStatus], limit: i64) -> Result<Vec<DocMerge>> {
        let statuses: Vec<&str> = statuses.iter().map(|s| s.as_str()).collect();
        let limit = if limit > 0 { limit } else { 100 };
        let mut stmt = self.conn().prepare(
            "SELECT * FROM doc_merges WHERE (?1 = 0 OR status IN (SELECT value FROM json_each(?2)))
             ORDER BY id DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![!statuses.is_empty(), serde_json::to_string(&statuses)?, limit], DocMerge::from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Moves an unfinished merge to `status`. Each attempt that starts counts when `attempts_inc` is set.
    /// Resolution goes through `resolve_merge`.
    pub fn update_merge(&self, id: i64, status: MergeStatus, error: Option<&str>, attempts_inc: bool) -> Result<DocMerge> {
        self.tx(|| {
            let current = self.doc_merge(id)?;
            if current.status == MergeStatus::Resolved {
                return Err(GenieError::invalid(format!("merge {id} is resolved")));
            }
            if status == MergeStatus::Resolved {
                return Err(GenieError::invalid(format!("merge {id} is resolved with resolve_merge")));
            }
            self.conn().execute(
                "UPDATE doc_merges SET status = ?1, error = ?2, attempts = attempts + ?3, updated = ?4 WHERE id = ?5",
                params![status.as_str(), error, i64::from(attempts_inc), now(), id],
            )?;
            self.doc_merge(id)
        })
    }

    /// Records the agent job that works on a merge.
    pub fn set_merge_job(&self, id: i64, job: i64) -> Result<DocMerge> {
        let n = self.conn().execute("UPDATE doc_merges SET job = ?1, updated = ?2 WHERE id = ?3", params![job, now(), id])?;
        if n == 0 {
            return Err(GenieError::not_found(format!("merge {id} not found")));
        }
        self.doc_merge(id)
    }

    /// Closes a merge with the commit that resolved it.
    pub fn resolve_merge(&self, id: i64, by: &str, resolution: &str, contradiction: bool, note: &str) -> Result<DocMerge> {
        self.tx(|| {
            let current = self.doc_merge(id)?;
            if current.status == MergeStatus::Resolved {
                return Err(GenieError::invalid(format!("merge {id} is already resolved")));
            }
            self.conn().execute(
                "UPDATE doc_merges SET status = 'resolved', resolved_by = ?1, resolution = ?2, contradiction = ?3, note = ?4, updated = ?5
                 WHERE id = ?6",
                params![by, resolution, i64::from(contradiction), note, now(), id],
            )?;
            self.doc_merge(id)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_db::SERVER_SCHEMA_VERSION;

    fn db() -> (tempfile::TempDir, ServerDb) {
        let dir = tempfile::tempdir().unwrap();
        let db = ServerDb::open(&dir.path().join("server.db")).unwrap();
        (dir, db)
    }

    fn new<'a>(branch: &'a str, author: &'a str, task: Option<&'a str>) -> NewChangeset<'a> {
        NewChangeset {
            branch,
            project: Some("shop"),
            task,
            author,
            author_name: "Anna",
            author_kind: "human",
            title: "Fix the intro",
            base: "abc123",
            origin: ChangesetOrigin::Genie,
            reverts: None,
        }
    }

    /// Sets timestamps directly so the idle tests do not depend on the wall clock.
    fn set_times(db: &ServerDb, id: i64, updated: &str, reminded: Option<&str>) {
        db.conn()
            .execute("UPDATE doc_changesets SET updated = ?1, reminded_at = ?2 WHERE id = ?3", params![updated, reminded, id])
            .unwrap();
    }

    fn ids(list: &[Changeset]) -> Vec<i64> {
        list.iter().map(|c| c.id).collect()
    }

    #[test]
    fn an_empty_branch_is_named_after_the_allocated_id() {
        let (_d, db) = db();
        let c = db.create_changeset(&new("", "anna", None)).unwrap();
        assert_eq!(c.branch, format!("changes/{}", c.id));
        assert_eq!(c.status, ChangesetStatus::Open);
        assert!(c.paths.is_empty());
        assert_eq!(db.changeset_by_branch(&c.branch).unwrap().map(|x| x.id), Some(c.id));
        let named = db.create_changeset(&new("feature/intro", "anna", None)).unwrap();
        assert_eq!(named.branch, "feature/intro");
    }

    #[test]
    fn a_duplicate_branch_is_rejected() {
        let (_d, db) = db();
        db.create_changeset(&new("changes/x", "anna", None)).unwrap();
        assert!(db.create_changeset(&new("changes/x", "bob", None)).is_err());
        assert!(db.changeset_by_branch("changes/missing").unwrap().is_none());
        assert!(db.changeset(999).is_err());
    }

    #[test]
    fn touching_a_changeset_adds_sorted_unique_paths() {
        let (_d, db) = db();
        let c = db.create_changeset(&new("", "anna", None)).unwrap();
        let c = db.touch_changeset(c.id, &["shop/b.md".into(), "shop/a.md".into()]).unwrap();
        assert_eq!(c.paths, vec!["shop/a.md", "shop/b.md"]);
        let c = db.touch_changeset(c.id, &["shop/a.md".into(), "shop/c.md".into()]).unwrap();
        assert_eq!(c.paths, vec!["shop/a.md", "shop/b.md", "shop/c.md"]);
    }

    #[test]
    fn changesets_are_filtered_newest_first() {
        let (_d, db) = db();
        let a = db.create_changeset(&new("", "anna", Some("G-1"))).unwrap();
        db.touch_changeset(a.id, &["shop/intro.md".into()]).unwrap();
        let b = db.create_changeset(&new("", "bob", Some("G-2"))).unwrap();
        db.touch_changeset(b.id, &["web/home.md".into()]).unwrap();
        let c = db.create_changeset(&new("", "anna", None)).unwrap();
        db.touch_changeset(c.id, &["shop/guide/setup.md".into()]).unwrap();
        db.mark_published(c.id, "sha-c", true, "anna").unwrap();

        let all = |f: ChangesetFilter| db.changesets(&f).unwrap();
        assert_eq!(ids(&all(ChangesetFilter::default())), vec![c.id, b.id, a.id]);
        assert_eq!(ids(&all(ChangesetFilter { project_roots: Some(vec!["shop".into()]), ..Default::default() })), vec![c.id, a.id]);
        assert_eq!(ids(&all(ChangesetFilter { project_roots: Some(vec!["web".into()]), ..Default::default() })), vec![b.id]);
        assert!(all(ChangesetFilter { project_roots: Some(vec![]), ..Default::default() }).is_empty());
        assert_eq!(ids(&all(ChangesetFilter { statuses: vec![ChangesetStatus::Open], ..Default::default() })), vec![b.id, a.id]);
        assert_eq!(ids(&all(ChangesetFilter { task: Some("G-1".into()), ..Default::default() })), vec![a.id]);
        assert_eq!(ids(&all(ChangesetFilter { author: Some("anna".into()), ..Default::default() })), vec![c.id, a.id]);
        assert_eq!(ids(&all(ChangesetFilter { path: Some("shop/intro.md".into()), ..Default::default() })), vec![a.id]);
        assert_eq!(ids(&all(ChangesetFilter { path: Some("shop/intro".into()), ..Default::default() })), Vec::<i64>::new());
        assert_eq!(ids(&all(ChangesetFilter { limit: 1, ..Default::default() })), vec![c.id]);

        let unreviewed = ChangesetFilter { unreviewed: true, ..Default::default() };
        assert_eq!(ids(&all(unreviewed)), vec![c.id]);
        db.mark_reviewed(c.id, "bob", ReviewDecision::Reviewed).unwrap();
        assert!(all(ChangesetFilter { unreviewed: true, ..Default::default() }).is_empty());
    }

    #[test]
    fn the_open_changeset_of_an_author_and_task_is_found() {
        let (_d, db) = db();
        let a = db.create_changeset(&new("", "anna", Some("G-1"))).unwrap();
        db.touch_changeset(a.id, &["shop/intro.md".into()]).unwrap();
        let d = db.create_changeset(&new("", "anna", None)).unwrap();
        db.touch_changeset(d.id, &["shop/other.md".into()]).unwrap();

        let found = |author: &str, task: Option<&str>, path: Option<&str>| db.open_changeset_for(author, task, path).unwrap().map(|c| c.id);
        assert_eq!(found("anna", Some("G-1"), None), Some(a.id));
        assert_eq!(found("anna", None, None), Some(d.id));
        assert_eq!(found("anna", None, Some("shop/other.md")), Some(d.id));
        assert_eq!(found("anna", None, Some("shop/intro.md")), None, "the task-less changeset does not touch that page");
        assert_eq!(found("bob", Some("G-1"), None), None);

        // Newest first when an author has two open changesets for one task.
        let newer = db.create_changeset(&new("", "anna", Some("G-1"))).unwrap();
        assert_eq!(found("anna", Some("G-1"), None), Some(newer.id));

        // A published changeset is no longer open for its author.
        db.mark_published(d.id, "sha-d", false, "anna").unwrap();
        assert_eq!(found("anna", None, None), None);
    }

    #[test]
    fn publishing_and_reviewing_follow_the_lifecycle() {
        let (_d, db) = db();
        let c = db.create_changeset(&new("", "anna", None)).unwrap();
        let err = db.mark_reviewed(c.id, "bob", ReviewDecision::Reviewed).unwrap_err().to_string();
        assert!(err.contains("only a published one is reviewed"), "{err}");

        let published = db.mark_published(c.id, "deadbeef", true, "anna").unwrap();
        assert_eq!(published.status, ChangesetStatus::Published);
        assert_eq!(published.published_commit.as_deref(), Some("deadbeef"));
        assert!(published.published_at.is_some());
        assert_eq!(published.closed_by.as_deref(), Some("anna"));
        assert!(published.review_needed);
        assert!(db.mark_published(c.id, "again", false, "anna").is_err(), "a published changeset is not published twice");
        assert!(db.set_changeset_status(c.id, ChangesetStatus::Discarded, Some("anna")).is_err());

        let reviewed = db.mark_reviewed(c.id, "bob", ReviewDecision::Reverted).unwrap();
        assert_eq!(reviewed.review_decision, Some(ReviewDecision::Reverted));
        assert_eq!(reviewed.reviewed_by.as_deref(), Some("bob"));
        assert!(reviewed.reviewed_at.is_some());
    }

    #[test]
    fn a_discarded_changeset_is_closed_and_cannot_take_paths() {
        let (_d, db) = db();
        let c = db.create_changeset(&new("", "anna", None)).unwrap();
        assert!(db.set_changeset_status(c.id, ChangesetStatus::Published, None).is_err());
        let discarded = db.set_changeset_status(c.id, ChangesetStatus::Discarded, Some("bob")).unwrap();
        assert_eq!(discarded.status, ChangesetStatus::Discarded);
        assert_eq!(discarded.closed_by.as_deref(), Some("bob"));
        assert!(db.set_changeset_status(c.id, ChangesetStatus::Open, None).is_err());
        assert!(db.touch_changeset(c.id, &["shop/a.md".into()]).is_err());
    }

    #[test]
    fn idle_changesets_respect_the_reminder() {
        let (_d, db) = db();
        let i1 = db.create_changeset(&new("", "anna", None)).unwrap();
        let i2 = db.create_changeset(&new("", "bob", None)).unwrap();
        let published = db.create_changeset(&new("", "carol", None)).unwrap();
        db.mark_published(published.id, "sha", false, "carol").unwrap();
        for c in [&i1, &i2, &published] {
            set_times(&db, c.id, "2026-01-01T00:00:00.000Z", None);
        }
        let idle = |before: &str| ids(&db.idle_changesets(before).unwrap());
        assert_eq!(idle("2026-02-01T00:00:00.000Z"), vec![i1.id, i2.id]);

        db.mark_reminded(i1.id).unwrap();
        assert_eq!(idle("2026-02-01T00:00:00.000Z"), vec![i2.id], "reminded since its last update");

        // Updated after the reminder, so it is idle again and due another reminder.
        set_times(&db, i1.id, "2026-03-01T00:00:00.000Z", Some("2026-02-15T00:00:00.000Z"));
        assert_eq!(idle("2026-04-01T00:00:00.000Z"), vec![i2.id, i1.id], "oldest update first");
        assert!(db.mark_reminded(999).is_err());
    }

    #[test]
    fn a_merge_is_created_updated_and_resolved() {
        let (_d, db) = db();
        let c = db.create_changeset(&new("", "anna", None)).unwrap();
        assert!(db.create_merge(999, "main", "tip", &[]).is_err(), "the changeset must exist");
        assert!(db.active_merge(c.id).unwrap().is_none());

        let m = db.create_merge(c.id, "main1", "tip1", &["shop/intro.md".into()]).unwrap();
        assert_eq!(m.status, MergeStatus::Pending);
        assert_eq!(
            (m.attempts, m.conflicts.as_slice(), m.contradiction, m.note.as_str()),
            (0, &["shop/intro.md".to_string()][..], false, "")
        );
        assert_eq!(db.active_merge(c.id).unwrap().map(|x| x.id), Some(m.id));

        let running = db.update_merge(m.id, MergeStatus::Running, None, true).unwrap();
        assert_eq!((running.status, running.attempts), (MergeStatus::Running, 1));
        let failed = db.update_merge(m.id, MergeStatus::Failed, Some("boom"), true).unwrap();
        assert_eq!((failed.status, failed.attempts, failed.error.as_deref()), (MergeStatus::Failed, 2, Some("boom")));
        assert_eq!(db.active_merge(c.id).unwrap().map(|x| x.status), Some(MergeStatus::Failed), "a failed merge still needs work");
        let retried = db.update_merge(m.id, MergeStatus::Running, None, false).unwrap();
        assert_eq!((retried.attempts, retried.error), (2, None), "a retry without a new attempt keeps the count");

        let resolved = db.resolve_merge(m.id, "anna", "sha9", true, "kept both sides").unwrap();
        assert_eq!(resolved.status, MergeStatus::Resolved);
        assert_eq!(resolved.resolved_by.as_deref(), Some("anna"));
        assert_eq!(resolved.resolution.as_deref(), Some("sha9"));
        assert!(resolved.contradiction);
        assert_eq!(resolved.note, "kept both sides");
        assert!(db.update_merge(m.id, MergeStatus::Running, None, true).is_err(), "a resolved merge takes no updates");
        assert!(db.resolve_merge(m.id, "bob", "sha10", false, "").is_err());
        assert!(db.active_merge(c.id).unwrap().is_none());

        let second = db.create_merge(c.id, "main2", "tip2", &[]).unwrap();
        assert!(db.update_merge(second.id, MergeStatus::Resolved, None, false).is_err(), "resolution goes through resolve_merge");
        assert_eq!(db.active_merge(c.id).unwrap().map(|x| x.id), Some(second.id));
        assert_eq!(db.merges(&[MergeStatus::Pending], 0).unwrap().iter().map(|x| x.id).collect::<Vec<_>>(), vec![second.id]);
        assert_eq!(db.merges(&[], 0).unwrap().iter().map(|x| x.id).collect::<Vec<_>>(), vec![second.id, m.id]);
    }

    #[test]
    fn a_version_eleven_server_db_gains_the_changeset_tables() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.db");
        {
            let db = ServerDb::open(&path).unwrap();
            db.create_project("shop", "Shop", "tracker", None, None).unwrap();
            db.conn().execute_batch("DROP TABLE doc_merges; DROP TABLE doc_changesets;").unwrap();
            db.conn().execute("UPDATE meta SET value = '11' WHERE key = 'schema'", []).unwrap();
        }
        let report = crate::migrate::check_server(&path).unwrap();
        assert!(report.rehearsed, "the update is rehearsed on a copy: {report:?}");
        assert_eq!((report.from, report.to), (11, SERVER_SCHEMA_VERSION));

        let db = ServerDb::open(&path).unwrap();
        assert_eq!(db.project("shop").unwrap().name, "Shop", "the existing rows are kept");
        let version: String = db.conn().query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| r.get(0)).unwrap();
        assert_eq!(version, SERVER_SCHEMA_VERSION.to_string());
        let tables: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('doc_changesets', 'doc_merges')", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(tables, 2);
        let c = db.create_changeset(&new("", "anna", None)).unwrap();
        assert_eq!(db.changeset(c.id).unwrap().branch, format!("changes/{}", c.id));
    }
}
