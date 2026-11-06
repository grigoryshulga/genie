//! Git plumbing for the documentation vault's changesets.
//!
//! A vault is a non-bare repository whose checked-out branch is the published
//! documentation, plus `refs/heads/changes/<id>` branches that hold proposed
//! documentation changes and are never checked out. Writes therefore go through
//! plumbing commands only: `commit-tree` with a private `GIT_INDEX_FILE`,
//! compare-and-swap `update-ref`, and `merge-file` for individual paths. The
//! working tree and the real index stay untouched unless a caller explicitly
//! asks for a checkout (`fast_forward`) or to commit local edits
//! (`absorb_worktree`).
//!
//! Git runs through `std::process::Command` with prompts disabled, a stable `C`
//! locale and unquoted paths. Failures surface as the core's internal `Io`
//! error carrying git's stderr; caller mistakes (unsafe paths, malformed refs,
//! stale compare-and-swap expectations) as `Invalid`.

use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

use crate::error::{GenieError, Result};

/// The zero object id `update-ref` expects when a ref must not exist yet.
const ZERO_OID: &str = "0000000000000000000000000000000000000000";

/// Distinguishes the scratch files of concurrent callers in one process.
static SCRATCH: AtomicU64 = AtomicU64::new(0);

/// A handle on a vault repository. All commands run against `root`.
#[derive(Debug, Clone)]
pub struct Git {
    root: PathBuf,
}

/// What the caller knows about a commit it asks git to create.
#[derive(Debug, Clone, Default)]
pub struct CommitMeta {
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    /// Git trailers appended to the message as `Key: value` lines after a blank line (e.g. `Genie-Changeset: 12`).
    pub trailers: Vec<(String, String)>,
}

/// A parsed commit of the vault history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommitInfo {
    pub sha: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    /// RFC 3339 author date.
    pub date: String,
    pub subject: String,
    pub body: String,
    pub trailers: Vec<(String, String)>,
}

/// Result of merging one path three ways.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileMerge {
    /// Merged content; None = the file is deleted in the result.
    Clean(Option<Vec<u8>>),
    /// Both sides changed the same lines (or one deleted, the other edited). `marked`: content with standard conflict markers (empty when one side deleted).
    Conflict { base: Option<Vec<u8>>, ours: Option<Vec<u8>>, theirs: Option<Vec<u8>>, marked: Vec<u8> },
}

impl Git {
    /// Open an existing repository; invalid error when `root` has no `.git`.
    pub fn open(root: &Path) -> Result<Git> {
        if !root.join(".git").exists() {
            return Err(GenieError::invalid(format!("{} is not a git repository", root.display())));
        }
        Ok(Git { root: root.to_path_buf() })
    }

    /// Short name of the checked-out branch (e.g. "main" or "master").
    pub fn head_branch(&self) -> Result<String> {
        let out = self.exec_raw(&["symbolic-ref", "--short", "HEAD"])?;
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !out.status.success() || name.is_empty() {
            return Err(GenieError::invalid("the vault has no checked-out branch"));
        }
        Ok(name)
    }

    /// Full SHA of a revision, None when it does not exist (`rev-parse --verify -q <rev>^{commit}`).
    pub fn resolve(&self, rev: &str) -> Result<Option<String>> {
        check_rev(rev)?;
        let spec = format!("{rev}^{{commit}}");
        let out = self.exec_raw(&["rev-parse", "--verify", "-q", &spec])?;
        if !out.status.success() {
            return Ok(None);
        }
        let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
        Ok((!sha.is_empty()).then_some(sha))
    }

    /// File content at a revision; None when the path does not exist there.
    pub fn show(&self, rev: &str, path: &str) -> Result<Option<Vec<u8>>> {
        check_rev(rev)?;
        check_path(path)?;
        let spec = format!("{rev}:{path}");
        if !self.exec_raw(&["cat-file", "-e", &spec])?.status.success() {
            return Ok(None);
        }
        let out = self.exec(&["cat-file", "blob", &spec], &[], None)?;
        Ok(Some(out.stdout))
    }

    /// Paths that differ between two revisions (`diff --name-only -z --no-renames`), sorted.
    pub fn changed_paths(&self, from: &str, to: &str) -> Result<Vec<String>> {
        check_rev(from)?;
        check_rev(to)?;
        let out = self.exec(&["diff", "--name-only", "-z", "--no-renames", from, to, "--"], &[], None)?;
        let mut paths: Vec<String> =
            out.stdout.split(|b| *b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect();
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    pub fn merge_base(&self, a: &str, b: &str) -> Result<Option<String>> {
        check_rev(a)?;
        check_rev(b)?;
        let out = self.exec_raw(&["merge-base", a, b])?;
        if out.status.success() {
            let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
            return Ok((!sha.is_empty()).then_some(sha));
        }
        // `git merge-base` exits 1 when the histories share no commit.
        if out.status.code() == Some(1) {
            return Ok(None);
        }
        Err(failure(&["merge-base", a, b], &out.stderr))
    }

    /// Create a commit WITHOUT moving any ref: tree = tree of `base_rev` with `changes` applied (Some(bytes) = write file, None = delete file), parents as given (may be empty only for a root commit). Use a temporary index file (`GIT_INDEX_FILE` in a tempdir/unique file under .git), `read-tree`, `hash-object -w --stdin`, `update-index --add --cacheinfo 100644,<blob>,<path>` / `update-index --force-remove`, `write-tree`, `commit-tree`. Author AND committer from `meta` (GIT_AUTHOR_NAME/EMAIL, GIT_COMMITTER_NAME/EMAIL). Never touches the real index or working tree.
    pub fn commit_tree(
        &self,
        base_rev: &str,
        parents: &[&str],
        changes: &[(String, Option<Vec<u8>>)],
        meta: &CommitMeta,
    ) -> Result<String> {
        check_rev(base_rev)?;
        for parent in parents {
            check_rev(parent)?;
        }
        for (path, _) in changes {
            check_path(path)?;
        }
        // A private index keeps the real one, and the working tree, untouched.
        let index = self.scratch("index");
        let result = self.commit_tree_inner(&index, base_rev, parents, changes, meta);
        let _ = std::fs::remove_file(&index);
        result
    }

    fn commit_tree_inner(
        &self,
        index: &Path,
        base_rev: &str,
        parents: &[&str],
        changes: &[(String, Option<Vec<u8>>)],
        meta: &CommitMeta,
    ) -> Result<String> {
        self.indexed(index, &["read-tree", base_rev])?;
        for (path, change) in changes {
            match change {
                Some(bytes) => {
                    let out = self.exec(&["hash-object", "-w", "--stdin"], &[], Some(bytes))?;
                    let blob = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if blob.is_empty() {
                        return Err(unexpected("hash-object returned no blob id"));
                    }
                    let entry = format!("100644,{blob},{path}");
                    self.indexed(index, &["update-index", "--add", "--cacheinfo", &entry])?;
                }
                None => {
                    self.indexed(index, &["update-index", "--force-remove", "--", path])?;
                }
            }
        }
        let tree = String::from_utf8_lossy(&self.indexed(index, &["write-tree"])?.stdout).trim().to_string();
        let mut args = vec!["commit-tree", tree.as_str()];
        for parent in parents {
            args.push("-p");
            args.push(parent);
        }
        args.push("-F");
        args.push("-");
        let message = commit_message(meta);
        let out = self.exec(&args, &identity(meta), Some(message.as_bytes()))?;
        let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if sha.is_empty() {
            return Err(unexpected("commit-tree returned no commit id"));
        }
        Ok(sha)
    }

    /// Move a ref with compare-and-swap: `expected_old` Some(sha) = must currently be that; None = must not exist yet (use the zero OID 40×'0'). Refuse (invalid error) to move the checked-out branch's ref this way.
    pub fn update_ref(&self, refname: &str, new: &str, expected_old: Option<&str>) -> Result<()> {
        check_ref(refname)?;
        check_rev(new)?;
        self.refuse_checked_out(refname)?;
        self.expect_ref(refname, expected_old)?;
        // Passing the old value as well keeps the update atomic when another writer races us.
        self.exec(&["update-ref", refname, new, expected_old.unwrap_or(ZERO_OID)], &[], None)?;
        Ok(())
    }

    /// Delete a ref pinned to its expected SHA; the checked-out branch is refused like `update_ref`.
    pub fn delete_ref(&self, refname: &str, expected_old: &str) -> Result<()> {
        check_ref(refname)?;
        check_rev(expected_old)?;
        self.refuse_checked_out(refname)?;
        self.expect_ref(refname, Some(expected_old))?;
        self.exec(&["update-ref", "-d", refname, expected_old], &[], None)?;
        Ok(())
    }

    /// Refs under a prefix with their SHAs (`for-each-ref --format=%(refname) %(objectname) <prefix>`).
    pub fn list_refs(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        let out = self.exec(&["for-each-ref", "--format=%(refname) %(objectname)", prefix], &[], None)?;
        let mut refs = Vec::new();
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let Some((name, sha)) = line.split_once(' ') else { continue };
            refs.push((name.to_string(), sha.trim().to_string()));
        }
        refs.sort();
        Ok(refs)
    }

    /// Fast-forward the checked-out branch, its index and working tree to `new` (`merge --ff-only -q <new>`). Error if not a fast-forward. Returns the new HEAD.
    pub fn fast_forward(&self, new: &str) -> Result<String> {
        check_rev(new)?;
        self.exec(&["merge", "--ff-only", "-q", new], &[], None)?;
        self.head_sha()
    }

    /// Commit uncommitted changes of the working tree (tracked modifications, deletions, and untracked files not ignored) on the checked-out branch with `meta`. Returns None when clean. Uses the real index (`add -A`, then commit with author/committer env from meta).
    pub fn absorb_worktree(&self, meta: &CommitMeta) -> Result<Option<String>> {
        if self.exec(&["status", "--porcelain", "-z"], &[], None)?.stdout.is_empty() {
            return Ok(None);
        }
        self.exec(&["add", "-A"], &[], None)?;
        let message = commit_message(meta);
        self.exec(&["commit", "-q", "-F", "-"], &identity(meta), Some(message.as_bytes()))?;
        Ok(Some(self.head_sha()?))
    }

    /// Three-way merge of individual files: for each path, base/ours/theirs are the file at those revisions (absent = None).
    pub fn merge3(&self, base: &str, ours: &str, theirs: &str, paths: &[String]) -> Result<Vec<(String, FileMerge)>> {
        check_rev(base)?;
        check_rev(ours)?;
        check_rev(theirs)?;
        for path in paths {
            check_path(path)?;
        }
        let dir = self.scratch("merge");
        let result = self.merge3_inner(base, ours, theirs, paths, &dir);
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    fn merge3_inner(&self, base: &str, ours: &str, theirs: &str, paths: &[String], dir: &Path) -> Result<Vec<(String, FileMerge)>> {
        let mut merged = Vec::with_capacity(paths.len());
        for path in paths {
            let at_base = self.show(base, path)?;
            let at_ours = self.show(ours, path)?;
            let at_theirs = self.show(theirs, path)?;
            let file = if at_theirs == at_base {
                FileMerge::Clean(at_ours)
            } else if at_ours == at_base || at_ours == at_theirs {
                FileMerge::Clean(at_theirs)
            } else {
                match (&at_ours, &at_theirs) {
                    // One side deleted the file and the other edited it: nothing to merge line by line.
                    (Some(_), None) | (None, Some(_)) => {
                        FileMerge::Conflict { base: at_base, ours: at_ours, theirs: at_theirs, marked: Vec::new() }
                    }
                    (Some(ours_bytes), Some(theirs_bytes)) => self.merge_file(dir, at_base.as_deref(), ours_bytes, theirs_bytes)?,
                    (None, None) => FileMerge::Clean(None),
                }
            };
            merged.push((path.clone(), file));
        }
        Ok(merged)
    }

    /// Let git merge one file three ways and keep its conflict markers when it cannot.
    fn merge_file(&self, dir: &Path, base: Option<&[u8]>, ours: &[u8], theirs: &[u8]) -> Result<FileMerge> {
        std::fs::create_dir_all(dir)?;
        let ours_path = dir.join("ours");
        let base_path = dir.join("base");
        let theirs_path = dir.join("theirs");
        std::fs::write(&ours_path, ours)?;
        std::fs::write(&base_path, base.unwrap_or_default())?;
        std::fs::write(&theirs_path, theirs)?;
        let (ours_arg, base_arg, theirs_arg) = (
            ours_path.to_string_lossy().into_owned(),
            base_path.to_string_lossy().into_owned(),
            theirs_path.to_string_lossy().into_owned(),
        );
        let args = ["merge-file", "-p", "-L", "ours", "-L", "base", "-L", "theirs", &ours_arg, &base_arg, &theirs_arg];
        let out = self.exec_raw(&args)?;
        if out.status.success() {
            return Ok(FileMerge::Clean(Some(out.stdout)));
        }
        // Exit 1..=127 is the number of conflicts git had to mark up.
        match out.status.code() {
            Some(code) if (1..=127).contains(&code) => Ok(FileMerge::Conflict {
                base: base.map(<[u8]>::to_vec),
                ours: Some(ours.to_vec()),
                theirs: Some(theirs.to_vec()),
                marked: out.stdout,
            }),
            _ => Err(failure(&args, &out.stderr)),
        }
    }

    /// History, newest first: `log` of `rev` (optionally `--follow -- <path>`), up to `limit`. Parse with a `-z`/NUL-separated custom `--format` including %H %P %an %ae %aI %s %b and `%(trailers:only,unfold)` parsed into pairs.
    pub fn log(&self, rev: &str, path: Option<&str>, limit: usize) -> Result<Vec<CommitInfo>> {
        self.log_with(rev, path, limit, false)
    }

    /// Page history that keeps merge commits (`--full-history` instead of `--follow`): a publication is a merge commit, and `--follow` simplifies it away.
    pub fn log_full(&self, rev: &str, path: Option<&str>, limit: usize) -> Result<Vec<CommitInfo>> {
        self.log_with(rev, path, limit, true)
    }

    fn log_with(&self, rev: &str, path: Option<&str>, limit: usize, full_history: bool) -> Result<Vec<CommitInfo>> {
        check_rev(rev)?;
        if let Some(path) = path {
            check_path(path)?;
        }
        // The body is the only field that may contain newlines, so it goes last; every other
        // field is NUL-separated and records are NUL-terminated by `-z`.
        let format = "--format=%H%x00%P%x00%an%x00%ae%x00%aI%x00%s%x00%(trailers:only,unfold)%x00%b";
        let count = format!("--max-count={limit}");
        let mut args: Vec<&str> = vec!["log", "-z", format, &count];
        if full_history {
            args.push("--full-history");
        } else if path.is_some() {
            args.push("--follow");
        }
        args.push(rev);
        if let Some(path) = path {
            args.push("--");
            args.push(path);
        }
        let raw = self.exec(&args, &[], None)?.stdout;
        let text = String::from_utf8_lossy(&raw);
        let mut fields: Vec<&str> = text.split('\0').collect();
        // `-z` terminates every record, so the last split piece is empty.
        if fields.last() == Some(&"") {
            fields.pop();
        }
        if !fields.len().is_multiple_of(8) {
            return Err(unexpected("unexpected git log output"));
        }
        let mut commits = Vec::with_capacity(fields.len() / 8);
        for chunk in fields.chunks(8) {
            let parents = chunk[1].split_whitespace().map(str::to_string).collect();
            commits.push(CommitInfo {
                sha: chunk[0].to_string(),
                parents,
                author_name: chunk[2].to_string(),
                author_email: chunk[3].to_string(),
                date: chunk[4].to_string(),
                subject: chunk[5].to_string(),
                trailers: parse_trailers(chunk[6]),
                body: chunk[7].trim_end_matches('\n').to_string(),
            });
        }
        Ok(commits)
    }

    /// Commits reachable from `to` but not `from` (`rev-list --reverse from..to`), oldest first.
    pub fn commits_between(&self, from: &str, to: &str) -> Result<Vec<String>> {
        check_rev(from)?;
        check_rev(to)?;
        let range = format!("{from}..{to}");
        let out = self.exec(&["rev-list", "--reverse", &range], &[], None)?;
        Ok(String::from_utf8_lossy(&out.stdout).lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_string).collect())
    }

    /// Unified diff between two revisions, optionally limited to paths (`diff --no-color --no-renames -U3`).
    pub fn diff(&self, from: &str, to: &str, paths: &[String]) -> Result<String> {
        check_rev(from)?;
        check_rev(to)?;
        for path in paths {
            check_path(path)?;
        }
        let mut args = vec!["diff", "--no-color", "--no-renames", "-U3", from, to];
        if !paths.is_empty() {
            args.push("--");
            args.extend(paths.iter().map(String::as_str));
        }
        Ok(String::from_utf8_lossy(&self.exec(&args, &[], None)?.stdout).into_owned())
    }

    // --- plumbing helpers ----------------------------------------------------

    /// A configured git command: anchored at the root, no prompts, stable locale, unquoted paths.
    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(&self.root).arg("-c").arg("core.quotepath=off").args(args).env("GIT_TERMINAL_PROMPT", "0").env("LC_ALL", "C");
        cmd
    }

    /// Run git and fail on a non-zero exit with its stderr.
    fn exec(&self, args: &[&str], env: &[(&str, String)], input: Option<&[u8]>) -> Result<Output> {
        let mut cmd = self.command(args);
        for (key, value) in env {
            cmd.env(key, value);
        }
        match input {
            None => {
                let out = cmd.stdin(Stdio::null()).output();
                finish(args, out)
            }
            Some(bytes) => {
                let mut child =
                    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| spawn_error(args, e))?;
                if let Some(mut stdin) = child.stdin.take() {
                    // A git that already failed may close the pipe; its exit status reports why.
                    if let Err(e) = stdin.write_all(bytes)
                        && e.kind() != ErrorKind::BrokenPipe
                    {
                        return Err(GenieError::Io(std::io::Error::other(format!("git {}: {e}", args.join(" ")))));
                    }
                }
                finish(args, child.wait_with_output())
            }
        }
    }

    /// Run git without judging its exit status (probing commands report absence that way).
    fn exec_raw(&self, args: &[&str]) -> Result<Output> {
        self.command(args).stdin(Stdio::null()).output().map_err(|e| spawn_error(args, e))
    }

    /// Run an index operation against a private index file.
    fn indexed(&self, index: &Path, args: &[&str]) -> Result<Output> {
        self.exec(args, &[("GIT_INDEX_FILE", index.to_string_lossy().into_owned())], None)
    }

    /// A unique scratch path next to the repository's git directory.
    fn scratch(&self, stem: &str) -> PathBuf {
        let base = if self.root.join(".git").is_dir() { self.root.join(".git") } else { std::env::temp_dir() };
        base.join(format!("genie-{stem}-{}-{}", std::process::id(), SCRATCH.fetch_add(1, Ordering::Relaxed)))
    }

    /// Full SHA of the current HEAD.
    fn head_sha(&self) -> Result<String> {
        let sha = String::from_utf8_lossy(&self.exec(&["rev-parse", "HEAD"], &[], None)?.stdout).trim().to_string();
        if sha.is_empty() {
            return Err(unexpected("rev-parse HEAD returned nothing"));
        }
        Ok(sha)
    }

    /// The ref of the checked-out branch, if HEAD is on a branch.
    fn checked_out_ref(&self) -> Result<Option<String>> {
        let out = self.exec_raw(&["symbolic-ref", "-q", "HEAD"])?;
        if !out.status.success() {
            return Ok(None);
        }
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        Ok((!name.is_empty()).then_some(name))
    }

    /// Refuse to rewrite the branch the working tree stands on without a checkout.
    fn refuse_checked_out(&self, refname: &str) -> Result<()> {
        if self.checked_out_ref()?.as_deref() == Some(refname) {
            return Err(GenieError::invalid(format!("refusing to change the checked-out branch {refname} without a checkout")));
        }
        Ok(())
    }

    /// Compare a ref's current value with the caller's expectation before the atomic update.
    fn expect_ref(&self, refname: &str, expected_old: Option<&str>) -> Result<()> {
        let current = self.exec_raw(&["rev-parse", "--verify", "-q", refname])?;
        let current = current.status.success().then(|| String::from_utf8_lossy(&current.stdout).trim().to_string());
        if current.as_deref() != expected_old {
            return Err(GenieError::invalid(format!(
                "{refname} is at {} but expected {}",
                current.as_deref().unwrap_or("<missing>"),
                expected_old.unwrap_or("<missing>")
            )));
        }
        Ok(())
    }
}

/// Compose the commit message: the caller's text, then trailers after a blank line.
fn commit_message(meta: &CommitMeta) -> String {
    let mut message = meta.message.trim_end().to_string();
    if !meta.trailers.is_empty() {
        message.push_str("\n\n");
        let trailers: Vec<String> = meta.trailers.iter().map(|(key, value)| format!("{key}: {value}")).collect();
        message.push_str(&trailers.join("\n"));
        message.push('\n');
    }
    message
}

/// Author and committer identity for commits git creates on the caller's behalf.
fn identity(meta: &CommitMeta) -> Vec<(&'static str, String)> {
    vec![
        ("GIT_AUTHOR_NAME", meta.author_name.clone()),
        ("GIT_AUTHOR_EMAIL", meta.author_email.clone()),
        ("GIT_COMMITTER_NAME", meta.author_name.clone()),
        ("GIT_COMMITTER_EMAIL", meta.author_email.clone()),
    ]
}

/// Parse `%(trailers:only,unfold)` output into key/value pairs.
fn parse_trailers(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            Some((key.trim().to_string(), value.strip_prefix(' ').unwrap_or(value).trim_end().to_string()))
        })
        .collect()
}

/// Reject revisions that would read as options.
fn check_rev(rev: &str) -> Result<()> {
    if rev.is_empty() || rev.starts_with('-') || rev.chars().any(char::is_control) {
        return Err(GenieError::invalid(format!("invalid revision {rev:?}")));
    }
    Ok(())
}

/// Reject paths that could escape the vault before git sees them.
fn check_path(path: &str) -> Result<()> {
    let relative = Path::new(path);
    let unsafe_component = relative
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_)) || c.as_os_str().is_empty());
    if path.is_empty() || relative.is_absolute() || unsafe_component {
        return Err(GenieError::invalid(format!("invalid vault path {path:?}")));
    }
    Ok(())
}

/// Reject full ref names that git would not accept, so mistakes fail as `Invalid`.
fn check_ref(refname: &str) -> Result<()> {
    let forbidden = refname.ends_with('/')
        || refname.contains("//")
        || refname.contains("..")
        || refname.chars().any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c));
    if !refname.starts_with("refs/") || refname.len() <= "refs/".len() || forbidden {
        return Err(GenieError::invalid(format!("invalid ref name {refname:?}")));
    }
    Ok(())
}

fn spawn_error(args: &[&str], e: std::io::Error) -> GenieError {
    GenieError::Io(std::io::Error::other(format!("git {}: {e}", args.join(" "))))
}

/// git's stderr, trimmed, as the core's internal failure.
fn failure(args: &[&str], stderr: &[u8]) -> GenieError {
    let text = String::from_utf8_lossy(stderr);
    let text = text.trim();
    let reason = if text.is_empty() { "failed" } else { text };
    GenieError::Io(std::io::Error::other(format!("git {}: {reason}", args.join(" "))))
}

fn finish(args: &[&str], out: std::io::Result<Output>) -> Result<Output> {
    let out = out.map_err(|e| spawn_error(args, e))?;
    if out.status.success() { Ok(out) } else { Err(failure(args, &out.stderr)) }
}

fn unexpected(what: &str) -> GenieError {
    GenieError::Io(std::io::Error::other(format!("git: {what}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn repo() -> (tempfile::TempDir, Git) {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), &["init", "-q", "-b", "main"]);
        let git = Git::open(dir.path()).unwrap();
        (dir, git)
    }

    /// Run git in a test repository with an identity, failing the test on error.
    fn run(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn meta(message: &str) -> CommitMeta {
        CommitMeta {
            message: message.into(),
            author_name: "Ada Lovelace".into(),
            author_email: "ada@example.com".into(),
            trailers: Vec::new(),
        }
    }

    fn change(path: &str, bytes: &[u8]) -> (String, Option<Vec<u8>>) {
        (path.to_string(), Some(bytes.to_vec()))
    }

    /// A root commit on `main` with `docs/home.md`, returning its SHA.
    fn seed(dir: &Path) -> String {
        fs::create_dir_all(dir.join("docs")).unwrap();
        fs::write(dir.join("docs/home.md"), "home\n").unwrap();
        run(dir, &["add", "-A"]);
        run(dir, &["commit", "-q", "-m", "init"]);
        run(dir, &["rev-parse", "HEAD"])
    }

    fn scratch_files(dir: &Path) -> Vec<String> {
        fs::read_dir(dir.join(".git"))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("genie-"))
            .collect()
    }

    #[test]
    fn open_requires_a_repository() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(Git::open(dir.path()).unwrap_err(), GenieError::Invalid(_)));
    }

    #[test]
    fn head_branch_is_the_checked_out_name() {
        let (dir, git) = repo();
        seed(dir.path());
        assert_eq!(git.head_branch().unwrap(), "main");
    }

    #[test]
    fn commit_tree_leaves_checkout_alone() {
        let (dir, git) = repo();
        let head = seed(dir.path());
        let commit = git.commit_tree(&head, &[&head], &[change("docs/a.md", b"alpha\n")], &meta("docs: add a")).unwrap();

        assert_ne!(commit, head);
        assert_eq!(git.resolve(&commit).unwrap().as_deref(), Some(commit.as_str()));
        assert!(!dir.path().join("docs/a.md").exists(), "the working tree must not gain the file");
        assert_eq!(run(dir.path(), &["rev-parse", "HEAD"]), head);
        assert_eq!(run(dir.path(), &["status", "--porcelain"]), "");
        assert_eq!(git.show(&commit, "docs/a.md").unwrap(), Some(b"alpha\n".to_vec()));
        assert!(scratch_files(dir.path()).is_empty(), "the private index must be removed");

        git.update_ref("refs/heads/changes/1", &commit, None).unwrap();
        assert_eq!(git.resolve("refs/heads/changes/1").unwrap(), Some(commit.clone()));
        assert_eq!(run(dir.path(), &["rev-parse", "HEAD"]), head);
        assert_eq!(run(dir.path(), &["ls-files"]), "docs/home.md");
    }

    #[test]
    fn trailers_round_trip_through_log() {
        let (dir, git) = repo();
        let head = seed(dir.path());
        let mut meta = meta("docs: publish a");
        meta.trailers = vec![("Genie-Changeset".into(), "12".into()), ("Reviewed-By".into(), "Bob".into())];
        let commit = git.commit_tree(&head, &[&head], &[], &meta).unwrap();

        let log = git.log(&commit, None, 5).unwrap();
        assert_eq!(log.len(), 2, "the root commit is history too");
        assert_eq!(log[1].sha, head);
        let entry = &log[0];
        assert_eq!(entry.sha, commit);
        assert_eq!(entry.parents, vec![head]);
        assert_eq!(entry.subject, "docs: publish a");
        assert_eq!(entry.author_name, "Ada Lovelace");
        assert_eq!(entry.author_email, "ada@example.com");
        assert!(entry.date.starts_with("20") && entry.date.contains('T'), "RFC 3339 date, got {}", entry.date);
        assert_eq!(entry.trailers, vec![("Genie-Changeset".to_string(), "12".to_string()), ("Reviewed-By".to_string(), "Bob".to_string())]);
        assert!(entry.body.contains("Genie-Changeset: 12"), "body keeps the trailers, got {:?}", entry.body);
    }

    #[test]
    fn update_ref_rejects_stale_or_duplicate_expectations() {
        let (dir, git) = repo();
        let head = seed(dir.path());
        let other = git.commit_tree(&head, &[&head], &[], &meta("docs: other")).unwrap();
        git.update_ref("refs/heads/changes/2", &other, None).unwrap();

        let stale = git.update_ref("refs/heads/changes/2", &head, Some(&head)).unwrap_err();
        assert!(matches!(stale, GenieError::Invalid(_)), "{stale:?}");
        assert_eq!(git.resolve("refs/heads/changes/2").unwrap(), Some(other.clone()));

        let duplicate = git.update_ref("refs/heads/changes/2", &head, None).unwrap_err();
        assert!(matches!(duplicate, GenieError::Invalid(_)), "{duplicate:?}");

        git.update_ref("refs/heads/changes/2", &head, Some(&other)).unwrap();
        assert_eq!(git.resolve("refs/heads/changes/2").unwrap(), Some(head));
    }

    #[test]
    fn update_ref_refuses_the_checked_out_branch() {
        let (dir, git) = repo();
        let head = seed(dir.path());
        let other = git.commit_tree(&head, &[&head], &[], &meta("docs: other")).unwrap();
        let err = git.update_ref("refs/heads/main", &other, Some(&head)).unwrap_err();
        assert!(matches!(err, GenieError::Invalid(_)), "{err:?}");
        assert_eq!(run(dir.path(), &["rev-parse", "main"]), head);
    }

    #[test]
    fn fast_forward_moves_files_onto_disk() {
        let (dir, git) = repo();
        let head = seed(dir.path());
        let next = git
            .commit_tree(&head, &[&head], &[change("docs/home.md", b"home v2\n"), change("docs/new.md", b"new\n")], &meta("docs: publish"))
            .unwrap();
        git.update_ref("refs/heads/changes/3", &next, None).unwrap();

        assert_eq!(git.fast_forward(&next).unwrap(), next);
        assert_eq!(run(dir.path(), &["rev-parse", "HEAD"]), next);
        assert_eq!(fs::read_to_string(dir.path().join("docs/home.md")).unwrap(), "home v2\n");
        assert_eq!(fs::read_to_string(dir.path().join("docs/new.md")).unwrap(), "new\n");
    }

    #[test]
    fn fast_forward_refuses_divergence() {
        let (dir, git) = repo();
        let head = seed(dir.path());
        let side = git.commit_tree(&head, &[&head], &[change("docs/side.md", b"side\n")], &meta("docs: side")).unwrap();
        let ahead = git.commit_tree(&head, &[&head], &[change("docs/home.md", b"ahead\n")], &meta("docs: ahead")).unwrap();
        git.fast_forward(&ahead).unwrap();

        let err = git.fast_forward(&side).unwrap_err();
        assert!(matches!(err, GenieError::Io(_)), "{err:?}");
        assert_eq!(run(dir.path(), &["rev-parse", "HEAD"]), ahead);
        assert!(!dir.path().join("docs/side.md").exists());
    }

    #[test]
    fn absorb_worktree_commits_edits_and_new_files() {
        let (dir, git) = repo();
        seed(dir.path());
        fs::write(dir.path().join("docs/home.md"), "home edited\n").unwrap();
        fs::write(dir.path().join("docs/notes.md"), "notes\n").unwrap();

        let commit = git.absorb_worktree(&meta("docs: save local work")).unwrap().expect("a commit for the dirty tree");
        assert_eq!(run(dir.path(), &["rev-parse", "HEAD"]), commit);
        assert_eq!(run(dir.path(), &["status", "--porcelain"]), "");
        assert_eq!(git.show(&commit, "docs/notes.md").unwrap(), Some(b"notes\n".to_vec()));
        assert_eq!(git.show(&commit, "docs/home.md").unwrap(), Some(b"home edited\n".to_vec()));
        let log = git.log(&commit, None, 1).unwrap();
        assert_eq!(log[0].subject, "docs: save local work");
        assert_eq!(log[0].author_email, "ada@example.com");

        assert_eq!(git.absorb_worktree(&meta("docs: nothing to do")).unwrap(), None);
    }

    /// base, ours and theirs covering every merge3 rule; returns (repo, git, base, ours, theirs).
    fn merge_fixture() -> (tempfile::TempDir, Git, String, String, String) {
        let (dir, git) = repo();
        let head = seed(dir.path());
        let base = git
            .commit_tree(
                &head,
                &[&head],
                &[
                    change("docs/x.md", b"line1\nline2\nline3\n"),
                    change("docs/y.md", b"gone\n"),
                    change("docs/w.md", b"both delete\n"),
                    change("docs/t.md", b"t\n"),
                    change("docs/c.md", b"same\n"),
                ],
                &meta("docs: base"),
            )
            .unwrap();
        let ours = git
            .commit_tree(
                &base,
                &[&base],
                &[
                    change("docs/x.md", b"OURS\nline2\nline3\n"),
                    change("docs/y.md", b"gone ours\n"),
                    ("docs/w.md".to_string(), None),
                    change("docs/c.md", b"OURS\n"),
                ],
                &meta("docs: ours"),
            )
            .unwrap();
        let theirs = git
            .commit_tree(
                &base,
                &[&base],
                &[
                    change("docs/x.md", b"line1\nline2\nTHEIRS\n"),
                    ("docs/y.md".to_string(), None),
                    ("docs/w.md".to_string(), None),
                    change("docs/t.md", b"t theirs\n"),
                    change("docs/c.md", b"THEIRS\n"),
                    change("docs/z.md", b"new\n"),
                ],
                &meta("docs: theirs"),
            )
            .unwrap();
        (dir, git, base, ours, theirs)
    }

    #[test]
    fn merge3_follows_the_three_way_rules() {
        let (_dir, git, base, ours, theirs) = merge_fixture();
        let paths: Vec<String> =
            ["docs/x.md", "docs/y.md", "docs/z.md", "docs/w.md", "docs/t.md", "docs/c.md", "docs/home.md"].map(String::from).to_vec();
        let merged = git.merge3(&base, &ours, &theirs, &paths).unwrap();
        assert_eq!(merged.len(), paths.len());

        // Both sides changed different lines: git merges without conflicts.
        assert_eq!(merged[0], ("docs/x.md".to_string(), FileMerge::Clean(Some(b"OURS\nline2\nTHEIRS\n".to_vec()))));
        // One side deleted, the other edited.
        assert_eq!(
            merged[1],
            (
                "docs/y.md".to_string(),
                FileMerge::Conflict {
                    base: Some(b"gone\n".to_vec()),
                    ours: Some(b"gone ours\n".to_vec()),
                    theirs: None,
                    marked: Vec::new(),
                }
            )
        );
        // New file on theirs only.
        assert_eq!(merged[2], ("docs/z.md".to_string(), FileMerge::Clean(Some(b"new\n".to_vec()))));
        // Both deleted.
        assert_eq!(merged[3], ("docs/w.md".to_string(), FileMerge::Clean(None)));
        // Only theirs changed.
        assert_eq!(merged[4], ("docs/t.md".to_string(), FileMerge::Clean(Some(b"t theirs\n".to_vec()))));
        // Both changed the same line.
        let (conflict_path, conflict) = &merged[5];
        assert_eq!(conflict_path, "docs/c.md");
        match conflict {
            FileMerge::Conflict { base, ours, theirs, marked } => {
                assert_eq!(base.as_deref(), Some(&b"same\n"[..]));
                assert_eq!(ours.as_deref(), Some(&b"OURS\n"[..]));
                assert_eq!(theirs.as_deref(), Some(&b"THEIRS\n"[..]));
                let text = String::from_utf8_lossy(marked);
                assert!(text.contains("<<<<<<< ours"), "{text}");
                assert!(text.contains("======="), "{text}");
                assert!(text.contains(">>>>>>> theirs"), "{text}");
                assert!(text.contains("OURS") && text.contains("THEIRS"), "{text}");
            }
            other => panic!("expected a conflict on docs/c.md, got {other:?}"),
        }
        // Untouched on both sides.
        assert_eq!(merged[6], ("docs/home.md".to_string(), FileMerge::Clean(Some(b"home\n".to_vec()))));
    }

    #[test]
    fn merge3_cleans_up_its_scratch_directory() {
        let (dir, git, base, ours, theirs) = merge_fixture();
        git.merge3(&base, &ours, &theirs, &["docs/c.md".to_string()]).unwrap();
        assert!(scratch_files(dir.path()).is_empty(), "{:?}", scratch_files(dir.path()));
    }

    #[test]
    fn log_follows_a_rename() {
        let (dir, git) = repo();
        let head = seed(dir.path());
        let renamed = git
            .commit_tree(
                &head,
                &[&head],
                &[("docs/home.md".to_string(), None), change("docs/index.md", b"home\n")],
                &meta("docs: rename home"),
            )
            .unwrap();

        let history = git.log(&renamed, Some("docs/index.md"), 10).unwrap();
        assert_eq!(history.len(), 2, "{history:?}");
        assert_eq!(history[0].subject, "docs: rename home");
        assert_eq!(history[1].sha, head, "the rename history reaches back to the original path");
    }

    #[test]
    fn changed_paths_and_commits_between() {
        let (dir, git) = repo();
        let head = seed(dir.path());
        let first = git.commit_tree(&head, &[&head], &[change("docs/a.md", b"a\n")], &meta("docs: a")).unwrap();
        let second =
            git.commit_tree(&first, &[&first], &[change("docs/a.md", b"a v2\n"), change("docs/b.md", b"b\n")], &meta("docs: b")).unwrap();

        assert_eq!(git.changed_paths(&head, &second).unwrap(), vec!["docs/a.md".to_string(), "docs/b.md".to_string()]);
        assert_eq!(git.commits_between(&head, &second).unwrap(), vec![first.clone(), second.clone()]);
        assert!(git.commits_between(&second, &head).unwrap().is_empty());
        assert_eq!(git.merge_base(&first, &second).unwrap(), Some(first.clone()));
        assert!(git.diff(&head, &second, &["docs/b.md".to_string()]).unwrap().contains("+b"));
        assert!(dir.path().exists());
    }

    #[test]
    fn show_of_a_missing_path_is_none() {
        let (dir, git) = repo();
        let head = seed(dir.path());
        assert_eq!(git.show(&head, "docs/missing.md").unwrap(), None);
        assert_eq!(git.resolve("refs/heads/changes/absent").unwrap(), None);
        assert!(dir.path().join("docs").exists());
    }

    #[test]
    fn paths_may_not_escape_the_vault() {
        let (dir, git) = repo();
        let head = seed(dir.path());
        let meta = meta("docs: escape");

        let show = git.show(&head, "../outside.md").unwrap_err();
        assert!(matches!(show, GenieError::Invalid(_)), "{show:?}");
        let absolute = git.show(&head, "/etc/passwd").unwrap_err();
        assert!(matches!(absolute, GenieError::Invalid(_)), "{absolute:?}");
        let commit = git.commit_tree(&head, &[&head], &[change("../outside.md", b"x\n")], &meta).unwrap_err();
        assert!(matches!(commit, GenieError::Invalid(_)), "{commit:?}");
        let merge = git.merge3(&head, &head, &head, &["../../etc/passwd".to_string()]).unwrap_err();
        assert!(matches!(merge, GenieError::Invalid(_)), "{merge:?}");
        assert!(scratch_files(dir.path()).is_empty(), "a rejected commit leaves no private index behind");
    }

    #[test]
    fn delete_ref_requires_the_expected_sha() {
        let (dir, git) = repo();
        let head = seed(dir.path());
        let other = git.commit_tree(&head, &[&head], &[], &meta("docs: other")).unwrap();
        git.update_ref("refs/heads/changes/4", &other, None).unwrap();

        let stale = git.delete_ref("refs/heads/changes/4", &head).unwrap_err();
        assert!(matches!(stale, GenieError::Invalid(_)), "{stale:?}");
        // Only changeset refs are listed under their prefix.
        assert_eq!(git.list_refs("refs/heads/changes/").unwrap(), vec![("refs/heads/changes/4".to_string(), other.clone())]);

        git.delete_ref("refs/heads/changes/4", &other).unwrap();
        assert_eq!(git.resolve("refs/heads/changes/4").unwrap(), None);
    }
}
