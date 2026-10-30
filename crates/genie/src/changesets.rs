//! Documentation changesets: edits on branches, publication into main, merges, rollback.
//!
//! INTERFACE STUB for parallel work (GNE-110): the signatures are the contract; the
//! implementation lands from the service task and replaces every body below.

use genie_core::changesets::{Changeset, DocMerge};
use genie_core::doc_ops::DocOp;
use genie_core::vault::AuthorKind;
use genie_core::vault_git::CommitInfo;
use serde_json::Value;

use crate::state::{App, AppResult};

/// Who edits, publishes or reviews.
#[derive(Debug, Clone)]
pub struct Editor {
    pub name: String,
    pub login: String,
    pub kind: AuthorKind,
    pub task: Option<String>,
    pub project: String,
    pub admin: bool,
    /// The agent's role (`doc-merger` may resolve merges).
    pub role: Option<String>,
}

pub struct Edited {
    pub changeset: Changeset,
    pub commit: String,
    pub content: String,
}

pub enum Published {
    Published { changeset: Box<Changeset>, commit: String },
    Merging { changeset: Box<Changeset>, merge: Box<DocMerge> },
}

const STUB: &str = "documentation changesets: not implemented yet";

fn stub<T>() -> AppResult<T> {
    Err(genie_core::GenieError::invalid(STUB).into())
}

pub fn edit(_app: &App, _ed: &Editor, _path: &str, _ops: &[DocOp], _note: &str, _changeset: Option<i64>) -> AppResult<Edited> {
    stub()
}
pub fn remove_page(_app: &App, _ed: &Editor, _path: &str, _note: &str, _changeset: Option<i64>) -> AppResult<Edited> {
    stub()
}
pub fn move_page(_app: &App, _ed: &Editor, _from: &str, _to: &str, _note: &str, _changeset: Option<i64>) -> AppResult<Edited> {
    stub()
}
pub fn publish(_app: &App, _id: i64, _by: &Editor) -> AppResult<Published> {
    stub()
}
pub fn discard(_app: &App, _id: i64, _by: &Editor) -> AppResult<Changeset> {
    stub()
}
pub fn review(_app: &App, _id: i64, _by: &Editor) -> AppResult<Changeset> {
    stub()
}
pub fn revert(_app: &App, _id: i64, _by: &Editor) -> AppResult<Published> {
    stub()
}
pub fn restore(_app: &App, _ed: &Editor, _path: &str, _at: &str) -> AppResult<Published> {
    stub()
}
pub fn history(_app: &App, _project: &str, _path: &str, _limit: usize) -> AppResult<Vec<CommitInfo>> {
    stub()
}
pub fn diff(_app: &App, _project: &str, _path: &str, _from: &str, _to: &str) -> AppResult<String> {
    stub()
}
pub fn changeset_detail(_app: &App, _project: &str, _id: i64) -> AppResult<Value> {
    stub()
}
#[allow(clippy::too_many_arguments)]
pub fn read_for(
    _app: &App,
    _ed: &Editor,
    _path: &str,
    _changeset: Option<&str>,
    _at: Option<&str>,
    _heading: Option<&str>,
    _max_chars: Option<usize>,
) -> AppResult<Value> {
    stub()
}
pub fn merge_detail(_app: &App, _project: &str, _id: i64) -> AppResult<Value> {
    stub()
}
#[allow(clippy::too_many_arguments)]
pub fn resolve_merge(
    _app: &App,
    _id: i64,
    _by: &Editor,
    _pages: &[(String, Option<String>)],
    _done: bool,
    _fail: bool,
    _contradiction: bool,
    _note: &str,
) -> AppResult<Value> {
    stub()
}
pub fn sweep(_app: &App) -> AppResult<()> {
    Ok(())
}
/// One-time migration of open proposals and `.conflict` copies (sync/migration task).
pub fn migrate_legacy(_app: &App) -> AppResult<()> {
    Ok(())
}
