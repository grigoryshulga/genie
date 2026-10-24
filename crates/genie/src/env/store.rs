//! The environment store on the server's disk: published snapshots, the views workspaces see them
//! through, download caches, and the directories of workspaces.
//!
//! A snapshot is written to a staging directory, then renamed into place whole, so a reader sees
//! either no snapshot or a complete one. Published trees are made read-only with permissions;
//! a copy view is a separate tree that a workspace may write. Removing a tree that a container
//! wrote into needs that runtime's own help, so every removal here takes a `Remover` the caller
//! supplies.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use genie_core::env::caps::ViewMode;
use serde::{Deserialize, Serialize};

use super::runner::MountMode;
use crate::config::EnvironmentsConfig;

/// Removes a path that may hold files owned by another user (a container's). The host runner
/// passes `remove_tree`; the podman runner passes its own removal.
pub type Remover<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

/// Staging area of snapshots, inside the store root.
const STAGING: &str = ".staging";

/// What `publish` records next to a snapshot's outputs. Its presence is what makes a snapshot published.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub key: String,
    pub outputs: Vec<String>,
    pub size: u64,
    pub files: u64,
}

/// The environment store: the directory tree under `root`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    pub root: PathBuf,
}

/// The store root from configuration: `environments.root`, relative paths against the data
/// directory, or `<data>/env-store` when unset.
pub fn store_root(cfg: &EnvironmentsConfig, data: &Path) -> PathBuf {
    match &cfg.root {
        Some(root) if root.is_absolute() => root.clone(),
        Some(root) => data.join(root),
        None => data.join("env-store"),
    }
}

/// The directory of a workspace in a store rooted at `root`.
pub fn workspace_dir(root: &Path, ws: &str) -> PathBuf {
    root.join("workspaces").join(ws)
}

/// The host directory where the runner keeps a workspace's state and pid files.
pub fn run_dir(root: &Path, ws: &str) -> PathBuf {
    workspace_dir(root, ws).join("run")
}

/// The home directory a host workspace uses when its profile does not name another.
pub fn workspace_home(root: &Path, ws: &str) -> PathBuf {
    workspace_dir(root, ws).join("home")
}

/// Names of snapshots, workspaces, projects, outputs and steps: letters, digits, `-`, `_` and `.`,
/// never a leading dot, so no name can climb out of the store.
pub fn check_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok { Ok(()) } else { Err(format!("{name:?} is not a valid name: use letters, digits, '-', '_' and '.', without a leading dot")) }
}

/// A path made of names (`api/prepare`), each checked by `check_name`.
fn check_path(path: &str) -> Result<(), String> {
    path.split('/').try_for_each(check_name)
}

impl Store {
    /// Open the store, creating its directories. A root inside a Git work tree is refused: snapshots
    /// and workspaces hold generated files that must never be committed.
    pub fn open(cfg: &EnvironmentsConfig, data: &Path) -> Result<Store, String> {
        let root = store_root(cfg, data);
        if !root.is_absolute() {
            return Err(format!("environments.root {} is not an absolute path", root.display()));
        }
        refuse_in_work_tree(&root)?;
        for dir in [root.join("snapshots").join(STAGING), root.join("caches"), root.join("workspaces")] {
            fs::create_dir_all(&dir).map_err(|e| format!("{}: cannot create the environment store: {e}", dir.display()))?;
        }
        Ok(Store { root })
    }

    /// Where a published snapshot lives: `out/<i>` and `manifest.json` below it.
    pub fn snapshot_dir(&self, key: &str) -> PathBuf {
        self.root.join("snapshots").join(key)
    }

    /// The staging directory of one attempt at a snapshot: `out/<i>` for its outputs, `ctx/`, `step.log`.
    pub fn staging_dir(&self, key: &str, attempt: i64) -> PathBuf {
        self.root.join("snapshots").join(STAGING).join(format!("{key}.{attempt}"))
    }

    /// The download cache of a project, by name. Caches are per project: a project never shares one with another.
    pub fn cache_dir(&self, project: &str, name: &str) -> PathBuf {
        self.root.join("caches").join(format!("project-{project}")).join(name)
    }

    /// The directory of a workspace: its views, caches, home and run directory.
    pub fn workspace_dir(&self, ws: &str) -> PathBuf {
        workspace_dir(&self.root, ws)
    }

    /// Publish the staged outputs of one attempt as the snapshot `key`. Outputs are the staging
    /// `out/<i>` directories; their count is recorded in the manifest. Idempotent: an existing,
    /// verified snapshot is returned as it is. A directory at the snapshot's path without a manifest
    /// is an interrupted publish and is removed through `remove` first.
    pub fn publish(&self, key: &str, attempt: i64, outputs: &[String], remove: Remover) -> Result<(u64, u64), String> {
        check_name(key)?;
        if let Some(m) = self.verify_published(key) {
            return Ok((m.size, m.files));
        }
        let final_dir = self.snapshot_dir(key);
        remove_if_exists(&final_dir, remove)?;

        let staged = self.staging_dir(key, attempt).join("out");
        if !staged.is_dir() {
            return Err(format!("nothing is staged for snapshot {key} (attempt {attempt}): {} is missing", staged.display()));
        }
        for i in 0..outputs.len() {
            fs::create_dir_all(staged.join(i.to_string())).map_err(|e| format!("{}: {e}", staged.display()))?;
        }
        // The tree is built next to its final place, then renamed in one step.
        let tree = self.publishing_dir(key, attempt);
        remove_if_exists(&tree, remove)?;
        fs::create_dir_all(&tree).map_err(|e| format!("{}: {e}", tree.display()))?;
        fs::rename(&staged, tree.join("out")).map_err(|e| format!("cannot move the outputs of {key} into place: {e}"))?;

        let (size, files) = dir_size(&tree.join("out"));
        let manifest = Manifest { key: key.into(), outputs: outputs.to_vec(), size, files };
        let text = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
        fs::write(tree.join("manifest.json"), text).map_err(|e| format!("{}: {e}", tree.join("manifest.json").display()))?;
        // The contents are read-only before the rename; the directories that move keep their write bit.
        make_readonly(&tree.join("out"))?;
        make_readonly(&tree.join("manifest.json"))?;
        fs::rename(&tree, &final_dir).map_err(|e| format!("cannot publish snapshot {key}: {e}"))?;
        make_readonly(&final_dir)?;
        Ok((size, files))
    }

    /// The manifest of a published snapshot, when it is complete: the manifest names this key and
    /// every output directory exists. `None` for anything else.
    pub fn verify_published(&self, key: &str) -> Option<Manifest> {
        check_name(key).ok()?;
        let dir = self.snapshot_dir(key);
        let manifest: Manifest = serde_json::from_slice(&fs::read(dir.join("manifest.json")).ok()?).ok()?;
        if manifest.key != key {
            return None;
        }
        let out = dir.join("out");
        let complete = out.is_dir() && (0..manifest.outputs.len()).all(|i| out.join(i.to_string()).is_dir());
        complete.then_some(manifest)
    }

    /// How a workspace sees output `index` of snapshot `key`, with the view's mode.
    ///
    /// An overlay view is the snapshot itself as the lower layer, with an upper and a work directory
    /// of the workspace. A copy view is a full copy, made once with `cp -a --reflink=auto`; it is
    /// writable and independent of the snapshot and of other copies. Returns the source directory
    /// and the mount mode the runner must use for it.
    pub fn view(&self, ws: &str, qualified: &str, index: usize, key: &str, mode: ViewMode) -> Result<(PathBuf, MountMode), String> {
        check_name(ws)?;
        check_path(qualified)?;
        check_name(key)?;
        let lower = self.snapshot_dir(key).join("out").join(index.to_string());
        if !lower.is_dir() {
            return Err(format!("snapshot {key} has no output {index}: it is not published"));
        }
        let slot = self.workspace_dir(ws).join("views").join(qualified).join(index.to_string());
        match mode {
            ViewMode::Overlay => {
                let (upper, work) = (slot.join("upper"), slot.join("work"));
                for dir in [&upper, &work] {
                    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                }
                Ok((lower, MountMode::Overlay { upper, work }))
            }
            ViewMode::Copy => {
                let copy = slot.join("copy");
                if !copy.exists() {
                    fs::create_dir_all(&slot).map_err(|e| format!("{}: {e}", slot.display()))?;
                    // Copied under a partial name and renamed, so a finished copy is the only `copy` there is.
                    let partial = slot.join("copy.partial");
                    remove_tree(&partial)?;
                    let status = Command::new("cp")
                        .args(["-a", "--reflink=auto"])
                        .arg(&lower)
                        .arg(&partial)
                        .status()
                        .map_err(|e| format!("cannot run cp for snapshot {key}: {e}"))?;
                    if !status.success() {
                        remove_tree(&partial)?;
                        return Err(format!("copying snapshot {key} output {index} into the workspace failed"));
                    }
                    make_writable(&partial)?;
                    fs::rename(&partial, &copy).map_err(|e| format!("{}: {e}", copy.display()))?;
                }
                Ok((copy, MountMode::ReadWrite))
            }
        }
    }

    /// Bytes a workspace owns (its views, home and run directory), and that plus the sizes of the
    /// snapshots it uses. Snapshots are counted at their manifest size.
    pub fn sizes(&self, ws: &str, snapshots: &[String]) -> Result<(u64, u64), String> {
        check_name(ws)?;
        let exclusive = dir_size(&self.workspace_dir(ws)).0;
        let mut logical = exclusive;
        for key in snapshots {
            let m = self.verify_published(key).ok_or_else(|| format!("snapshot {key} is not published"))?;
            logical += m.size;
        }
        Ok((exclusive, logical))
    }

    /// Staging directories left by attempts that did not finish or were not discarded.
    pub fn staging_leftovers(&self) -> Result<Vec<PathBuf>, String> {
        let dir = self.root.join("snapshots").join(STAGING);
        let mut out: Vec<PathBuf> =
            fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?.filter_map(|e| e.ok().map(|e| e.path())).collect();
        out.sort();
        Ok(out)
    }

    /// Remove a published snapshot through `remove`. The caller has checked that nothing uses it.
    pub fn remove_snapshot(&self, key: &str, remove: Remover) -> Result<(), String> {
        check_name(key)?;
        remove_if_exists(&self.snapshot_dir(key), remove)
    }

    /// Remove a workspace's directory through `remove`.
    pub fn remove_workspace(&self, ws: &str, remove: Remover) -> Result<(), String> {
        check_name(ws)?;
        remove_if_exists(&self.workspace_dir(ws), remove)
    }

    /// Remove what an attempt staged, including an unfinished publish, through `remove`.
    pub fn discard_staging(&self, key: &str, attempt: i64, remove: Remover) -> Result<(), String> {
        check_name(key)?;
        remove_if_exists(&self.staging_dir(key, attempt), remove)?;
        remove_if_exists(&self.publishing_dir(key, attempt), remove)
    }

    /// The tree an attempt builds before its rename into place. It sits in the staging area, so
    /// leftovers show up in `staging_leftovers`.
    fn publishing_dir(&self, key: &str, attempt: i64) -> PathBuf {
        self.root.join("snapshots").join(STAGING).join(format!("{key}.{attempt}.publish"))
    }
}

/// Remove `path` through `remove` when something is there (a symlink counts).
fn remove_if_exists(path: &Path, remove: Remover) -> Result<(), String> {
    if fs::symlink_metadata(path).is_ok() { remove(path) } else { Ok(()) }
}

/// Refuse a root that is inside a Git work tree: the nearest existing ancestor, and every
/// directory above it, must not hold a `.git` entry.
fn refuse_in_work_tree(root: &Path) -> Result<(), String> {
    let mut existing = root.to_path_buf();
    while !existing.exists() && existing.pop() {}
    for dir in existing.ancestors() {
        if dir.join(".git").exists() {
            return Err(format!(
                "the environment store {} is inside the Git work tree {}: set environments.root to a directory outside it",
                root.display(),
                dir.display()
            ));
        }
    }
    Ok(())
}

/// Remove a tree that may be read-only, as the host runner does for its own files.
pub fn remove_tree(path: &Path) -> Result<(), String> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()));
    }
    make_writable(path)?;
    fs::remove_dir_all(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Visit `path` and everything below it, without following symlinks.
fn each_entry(path: &Path, f: &mut dyn FnMut(&Path, &fs::Metadata) -> Result<(), String>) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    f(path, &meta)?;
    if meta.is_dir() && !meta.file_type().is_symlink() {
        for entry in fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))? {
            let entry = entry.map_err(|e| format!("{}: {e}", path.display()))?;
            each_entry(&entry.path(), f)?;
        }
    }
    Ok(())
}

fn set_mode(path: &Path, meta: &fs::Metadata, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|e| format!("{}: {e}", path.display()))
}

/// Remove every write bit below `path`. Directories keep their search bit, so their contents stay reachable.
pub fn make_readonly(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    each_entry(path, &mut |p, meta| set_mode(p, meta, meta.permissions().mode() & !0o222))
}

/// Give the owner read and write (and search for directories) below `path`, so a copy or a read-only
/// tree can be written or removed.
pub fn make_writable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    each_entry(path, &mut |p, meta| {
        let bits = if meta.is_dir() { 0o700 } else { 0o600 };
        set_mode(p, meta, meta.permissions().mode() | bits)
    })
}

/// Bytes and entries below `path`: the apparent size of regular files, and every non-directory
/// entry counted once. Symlinks are counted as entries and never followed. A missing path is empty.
pub fn dir_size(path: &Path) -> (u64, u64) {
    let (mut bytes, mut files) = (0u64, 0u64);
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        let Ok(meta) = fs::symlink_metadata(&p) else { continue };
        if meta.is_dir() {
            if let Ok(entries) = fs::read_dir(&p) {
                stack.extend(entries.flatten().map(|e| e.path()));
            }
            continue;
        }
        files += 1;
        if meta.is_file() {
            bytes += meta.len();
        }
    }
    (bytes, files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EnvironmentsConfig;
    use std::os::unix::fs::PermissionsExt;

    /// A store under a temporary directory, and the directory itself.
    fn open(data: &Path) -> Store {
        let cfg = EnvironmentsConfig { root: Some(data.join("store")), ..Default::default() };
        Store::open(&cfg, data).unwrap()
    }

    /// Stage one output for `key` as the step would have written it.
    fn stage(store: &Store, key: &str, attempt: i64, text: &str) {
        let out = store.staging_dir(key, attempt).join("out").join("0");
        fs::create_dir_all(out.join("deps")).unwrap();
        fs::write(out.join("deps").join("lib.txt"), text).unwrap();
    }

    fn outputs() -> Vec<String> {
        vec!["node_modules".into()]
    }

    #[test]
    fn a_staged_snapshot_is_published_whole_and_the_staging_outputs_move_with_it() {
        let t = tempfile::tempdir().unwrap();
        let store = open(t.path());
        let key = "abc123";
        stage(&store, key, 1, "hello");
        assert!(store.verify_published(key).is_none(), "nothing is published before publish");

        let (size, files) = store.publish(key, 1, &outputs(), &remove_tree).unwrap();
        assert_eq!((size, files), (5, 1));
        let m = store.verify_published(key).expect("published");
        assert_eq!(m, Manifest { key: key.into(), outputs: outputs(), size: 5, files: 1 });
        assert_eq!(fs::read_to_string(store.snapshot_dir(key).join("out/0/deps/lib.txt")).unwrap(), "hello");
        assert!(!store.staging_dir(key, 1).join("out").exists(), "the outputs left the staging directory");
    }

    #[test]
    fn publishing_again_is_idempotent_and_keeps_the_first_result() {
        let t = tempfile::tempdir().unwrap();
        let store = open(t.path());
        stage(&store, "k1", 1, "first");
        let first = store.publish("k1", 1, &outputs(), &remove_tree).unwrap();
        // A second attempt that finds the snapshot ready does not need its staging at all.
        let second = store.publish("k1", 2, &outputs(), &remove_tree).unwrap();
        assert_eq!(first, second);
        assert_eq!(fs::read_to_string(store.snapshot_dir("k1").join("out/0/deps/lib.txt")).unwrap(), "first");
    }

    #[test]
    fn a_published_tree_is_read_only() {
        let t = tempfile::tempdir().unwrap();
        let store = open(t.path());
        stage(&store, "ro", 1, "fixed");
        store.publish("ro", 1, &outputs(), &remove_tree).unwrap();
        let file = store.snapshot_dir("ro").join("out/0/deps/lib.txt");
        let mode = fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o222, 0, "no write bit on a published file");
        assert_eq!(fs::metadata(store.snapshot_dir("ro").join("out")).unwrap().permissions().mode() & 0o222, 0);
        assert!(fs::write(&file, "changed").is_err() || is_root(), "a published file cannot be written by its owner");
        // Removal still works: the remover makes it writable first.
        store.remove_snapshot("ro", &remove_tree).unwrap();
        assert!(!store.snapshot_dir("ro").exists());
    }

    /// Whether the tests run as root, which writes read-only files anyway.
    fn is_root() -> bool {
        std::process::Command::new("id").arg("-u").output().is_ok_and(|o| o.stdout == b"0\n")
    }

    #[test]
    fn publishing_without_a_staged_output_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let store = open(t.path());
        let err = store.publish("none", 1, &outputs(), &remove_tree).unwrap_err();
        assert!(err.contains("nothing is staged"), "{err}");
    }

    #[test]
    fn an_interrupted_publish_without_a_manifest_is_removed_before_the_retry() {
        let t = tempfile::tempdir().unwrap();
        let store = open(t.path());
        // A crash left the snapshot directory without its manifest.
        fs::create_dir_all(store.snapshot_dir("half").join("out/0")).unwrap();
        fs::write(store.snapshot_dir("half").join("out/0/partial"), "x").unwrap();
        assert!(store.verify_published("half").is_none());
        stage(&store, "half", 1, "whole");
        let removed = std::cell::RefCell::new(Vec::new());
        let record = |p: &Path| {
            removed.borrow_mut().push(p.to_path_buf());
            remove_tree(p)
        };
        store.publish("half", 1, &outputs(), &record).unwrap();
        assert_eq!(removed.borrow().as_slice(), [store.snapshot_dir("half")]);
        assert_eq!(fs::read_to_string(store.snapshot_dir("half").join("out/0/deps/lib.txt")).unwrap(), "whole");
    }

    #[test]
    fn a_manifest_that_does_not_match_its_outputs_is_not_verified() {
        let t = tempfile::tempdir().unwrap();
        let store = open(t.path());
        stage(&store, "m", 1, "x");
        store.publish("m", 1, &outputs(), &remove_tree).unwrap();
        // Remove the output directory the manifest promises, in a snapshot made writable for the purpose.
        make_writable(&store.snapshot_dir("m")).unwrap();
        fs::remove_dir_all(store.snapshot_dir("m").join("out/0")).unwrap();
        assert!(store.verify_published("m").is_none());
    }

    #[test]
    fn a_copy_view_is_independent_of_the_snapshot_and_of_other_copies() {
        let t = tempfile::tempdir().unwrap();
        let store = open(t.path());
        stage(&store, "dep", 1, "shared");
        store.publish("dep", 1, &outputs(), &remove_tree).unwrap();

        let (first, mode) = store.view("shop--api", "api/prepare", 0, "dep", ViewMode::Copy).unwrap();
        assert_eq!(mode, MountMode::ReadWrite);
        let (second, _) = store.view("shop--web", "web/prepare", 0, "dep", ViewMode::Copy).unwrap();
        assert_ne!(first, second);

        // The copy is writable: a write in one workspace changes neither the snapshot nor the other copy.
        fs::write(first.join("deps/lib.txt"), "changed").unwrap();
        fs::write(first.join("mine.txt"), "only here").unwrap();
        assert_eq!(fs::read_to_string(second.join("deps/lib.txt")).unwrap(), "shared");
        assert!(!second.join("mine.txt").exists());
        assert_eq!(fs::read_to_string(store.snapshot_dir("dep").join("out/0/deps/lib.txt")).unwrap(), "shared");

        // A second request returns the same copy as it is: it is made once.
        let (again, _) = store.view("shop--api", "api/prepare", 0, "dep", ViewMode::Copy).unwrap();
        assert_eq!(again, first);
        assert!(again.join("mine.txt").exists(), "the copy was not replaced");
    }

    #[test]
    fn an_overlay_view_is_the_snapshot_with_upper_and_work_directories_of_the_workspace() {
        let t = tempfile::tempdir().unwrap();
        let store = open(t.path());
        stage(&store, "ov", 1, "lower");
        store.publish("ov", 1, &outputs(), &remove_tree).unwrap();
        let (lower, mode) = store.view("shop--api", "api/prepare", 0, "ov", ViewMode::Overlay).unwrap();
        assert_eq!(lower, store.snapshot_dir("ov").join("out/0"));
        let slot = store.workspace_dir("shop--api").join("views/api/prepare/0");
        assert_eq!(mode, MountMode::Overlay { upper: slot.join("upper"), work: slot.join("work") });
        assert!(slot.join("upper").is_dir() && slot.join("work").is_dir());
        assert!(store.view("shop--api", "api/prepare", 1, "ov", ViewMode::Overlay).is_err(), "output 1 is not published");
    }

    #[test]
    fn sizes_count_the_workspace_and_the_snapshots_it_uses() {
        let t = tempfile::tempdir().unwrap();
        let store = open(t.path());
        stage(&store, "s", 1, "12345");
        store.publish("s", 1, &outputs(), &remove_tree).unwrap();
        let ws = store.workspace_dir("w");
        fs::create_dir_all(ws.join("home")).unwrap();
        fs::write(ws.join("home/notes.txt"), "abc").unwrap();

        let (exclusive, logical) = store.sizes("w", &["s".to_string()]).unwrap();
        assert_eq!(exclusive, 3);
        assert_eq!(logical, 3 + 5);
        assert!(store.sizes("w", &["missing".to_string()]).is_err());
    }

    #[test]
    fn dir_size_does_not_follow_symlinks() {
        let t = tempfile::tempdir().unwrap();
        let big = t.path().join("big");
        fs::create_dir_all(&big).unwrap();
        fs::write(big.join("data"), vec![0u8; 4096]).unwrap();
        let tree = t.path().join("tree");
        fs::create_dir_all(&tree).unwrap();
        fs::write(tree.join("small"), "ab").unwrap();
        std::os::unix::fs::symlink(&big, tree.join("link")).unwrap();
        assert_eq!(dir_size(&tree), (2, 2), "the link counts as one entry and its target is not read");
        assert_eq!(dir_size(&t.path().join("missing")), (0, 0));
    }

    #[test]
    fn staging_leftovers_are_listed_and_discarded_through_the_remover() {
        let t = tempfile::tempdir().unwrap();
        let store = open(t.path());
        stage(&store, "left", 3, "x");
        let leftovers = store.staging_leftovers().unwrap();
        assert_eq!(leftovers, vec![store.staging_dir("left", 3)]);
        let calls = std::cell::Cell::new(0);
        let count = |p: &Path| {
            calls.set(calls.get() + 1);
            remove_tree(p)
        };
        store.discard_staging("left", 3, &count).unwrap();
        assert_eq!(calls.get(), 1);
        assert!(store.staging_leftovers().unwrap().is_empty());
        store.discard_staging("left", 3, &count).unwrap();
        assert_eq!(calls.get(), 1, "nothing to remove the second time");
    }

    #[test]
    fn names_that_could_leave_the_store_are_refused() {
        assert!(check_name("shop--api").is_ok());
        assert!(check_name("../etc").is_err());
        assert!(check_name(".hidden").is_err());
        assert!(check_name("a/b").is_err());
        assert!(check_name("").is_err());
        let t = tempfile::tempdir().unwrap();
        let store = open(t.path());
        assert!(store.view("../x", "p", 0, "k", ViewMode::Copy).is_err());
        assert!(store.view("w", "../p", 0, "k", ViewMode::Copy).is_err());
    }

    #[test]
    fn a_store_inside_a_git_work_tree_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let repo = t.path().join("repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        let cfg = EnvironmentsConfig { root: Some(repo.join("env-store")), ..Default::default() };
        let err = Store::open(&cfg, t.path()).unwrap_err();
        assert!(err.contains("Git work tree"), "{err}");
        // Outside any work tree the same store opens and its directories exist.
        let outside = EnvironmentsConfig { root: Some(t.path().join("elsewhere")), ..Default::default() };
        let store = Store::open(&outside, t.path()).unwrap();
        assert!(store.root.join("snapshots/.staging").is_dir() && store.root.join("workspaces").is_dir());
    }

    #[test]
    fn the_store_root_defaults_under_the_data_directory_and_resolves_relative_paths_there() {
        let data = Path::new("/srv/genie/data");
        assert_eq!(store_root(&EnvironmentsConfig::default(), data), data.join("env-store"));
        let relative = EnvironmentsConfig { root: Some("envs".into()), ..Default::default() };
        assert_eq!(store_root(&relative, data), data.join("envs"));
        let absolute = EnvironmentsConfig { root: Some("/mnt/envs".into()), ..Default::default() };
        assert_eq!(store_root(&absolute, data), PathBuf::from("/mnt/envs"));
    }
}
