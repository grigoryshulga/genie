//! Snapshot inputs read from Git at a commit: the content hashes that give a snapshot its key, and
//! the files a preparing step sees as its context.
//!
//! Both come from the commit's tree, never from the working tree, so an uncommitted edit changes
//! neither a key nor a context. Patterns are relative to the repository root. The matcher supports
//! `*` (within one path segment; it matches dot files too), `?` (one character of a segment) and
//! `**` (a whole segment that matches any number of directories, including none).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use genie_core::env::spec::{Inputs, InputsKeyword};

/// A blob of the commit's tree: its path, its object id and its mode (`100644`, `100755`, `120000`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    path: String,
    blob: String,
    mode: String,
}

/// The content hashes of a step's inputs at `sha`: each matched file's path with its blob id, and
/// for a pattern that matches nothing, the pattern with `absent`, so that creating the file
/// changes the key. Repository inputs hash the whole tree under `tree`.
pub fn input_hashes(repo: &Path, sha: &str, inputs: &Inputs) -> Result<BTreeMap<String, String>, String> {
    check_sha(sha)?;
    match inputs {
        Inputs::Keyword(InputsKeyword::Repository) => {
            let tree = git_text(repo, &["rev-parse", &format!("{sha}^{{tree}}")])?;
            Ok(BTreeMap::from([("tree".to_string(), tree)]))
        }
        Inputs::Files(patterns) => {
            let entries = tree(repo, sha)?;
            let mut hashes = BTreeMap::new();
            for pattern in patterns {
                let matched: Vec<&Entry> = entries.iter().filter(|e| matches(pattern, &e.path)).collect();
                if matched.is_empty() {
                    hashes.insert(pattern.clone(), "absent".to_string());
                }
                for e in matched {
                    hashes.insert(e.path.clone(), e.blob.clone());
                }
            }
            Ok(hashes)
        }
    }
}

/// Write the step's context into `dest` (created when missing): the files its patterns match, with
/// their paths, executable bits and symlinks; for repository inputs, the whole tree as `git archive`
/// exports it. Only committed content is written.
pub fn stage_context(repo: &Path, sha: &str, inputs: &Inputs, dest: &Path) -> Result<(), String> {
    check_sha(sha)?;
    fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    match inputs {
        Inputs::Keyword(InputsKeyword::Repository) => archive(repo, sha, dest),
        Inputs::Files(patterns) => {
            for e in tree(repo, sha)?.iter().filter(|e| patterns.iter().any(|p| matches(p, &e.path))) {
                write_entry(repo, e, dest)?;
            }
            Ok(())
        }
    }
}

/// Whether `path` matches `pattern`. A pattern ending in `/` matches everything below that directory.
pub fn matches(pattern: &str, path: &str) -> bool {
    let mut pattern = pattern;
    while let Some(rest) = pattern.strip_prefix("./") {
        pattern = rest;
    }
    pattern = pattern.trim_start_matches('/');
    let owned;
    let pattern = if pattern.ends_with('/') {
        owned = format!("{pattern}**");
        owned.as_str()
    } else {
        pattern
    };
    let pat: Vec<&str> = pattern.split('/').collect();
    let segments: Vec<&str> = path.split('/').collect();
    segments_match(&pat, &segments)
}

/// Match path segments one by one; `**` takes zero or more of them.
fn segments_match(pat: &[&str], path: &[&str]) -> bool {
    match pat.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => (0..=path.len()).any(|i| segments_match(rest, &path[i..])),
        Some((segment, rest)) => !path.is_empty() && segment_match(segment, path[0]) && segments_match(rest, &path[1..]),
    }
}

/// Match one segment: `*` runs of characters, `?` one character, everything else literally.
fn segment_match(pattern: &str, text: &str) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        match p.split_first() {
            None => t.is_empty(),
            Some(('*', rest)) => (0..=t.len()).any(|i| go(rest, &t[i..])),
            Some(('?', rest)) => !t.is_empty() && go(rest, &t[1..]),
            Some((c, rest)) => t.first() == Some(c) && go(rest, &t[1..]),
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    go(&p, &t)
}

/// Every blob of the tree at `sha`, with its path. A path that is not UTF-8 cannot be matched by a
/// pattern, so it is refused by name.
fn tree(repo: &Path, sha: &str) -> Result<Vec<Entry>, String> {
    let out = git_bytes(repo, &["ls-tree", "-r", "-z", "--full-tree", sha])?;
    let mut entries = Vec::new();
    for record in out.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let tab = record.iter().position(|b| *b == b'\t').ok_or_else(|| "git ls-tree printed a line without a path".to_string())?;
        let (meta, path) = (&record[..tab], &record[tab + 1..]);
        let path = std::str::from_utf8(path).map_err(|_| format!("a path in {sha} is not UTF-8 and cannot be matched by a pattern"))?;
        let meta = String::from_utf8_lossy(meta);
        let mut fields = meta.split(' ');
        let (Some(mode), Some(kind), Some(blob)) = (fields.next(), fields.next(), fields.next()) else {
            return Err(format!("git ls-tree printed an unexpected line for {path}"));
        };
        if kind == "blob" {
            entries.push(Entry { path: path.to_string(), blob: blob.to_string(), mode: mode.to_string() });
        }
    }
    Ok(entries)
}

fn write_entry(repo: &Path, e: &Entry, dest: &Path) -> Result<(), String> {
    let target = dest.join(&e.path);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    }
    let content = git_bytes(repo, &["cat-file", "blob", &e.blob])?;
    if e.mode == "120000" {
        let link = String::from_utf8_lossy(&content).into_owned();
        std::os::unix::fs::symlink(&link, &target).map_err(|err| format!("{}: {err}", target.display()))?;
        return Ok(());
    }
    fs::write(&target, &content).map_err(|err| format!("{}: {err}", target.display()))?;
    use std::os::unix::fs::PermissionsExt;
    let mode = if e.mode == "100755" { 0o755 } else { 0o644 };
    fs::set_permissions(&target, fs::Permissions::from_mode(mode)).map_err(|err| format!("{}: {err}", target.display()))
}

/// `git archive` of the commit, unpacked into `dest`.
fn archive(repo: &Path, sha: &str, dest: &Path) -> Result<(), String> {
    let mut git = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["archive", "--format=tar", sha])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run git archive: {e}"))?;
    let pipe = git.stdout.take().ok_or("git archive has no output pipe")?;
    let tar = Command::new("tar")
        .arg("-x")
        .arg("-C")
        .arg(dest)
        .stdin(Stdio::from(pipe))
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("cannot run tar: {e}"))?;
    let git_status = git.wait_with_output().map_err(|e| format!("git archive: {e}"))?;
    if !git_status.status.success() {
        return Err(format!("git archive {sha}: {}", String::from_utf8_lossy(&git_status.stderr).trim()));
    }
    if !tar.status.success() {
        return Err(format!("unpacking the repository at {sha}: {}", String::from_utf8_lossy(&tar.stderr).trim()));
    }
    Ok(())
}

/// A revision as git receives it: hexadecimal, so no option can be smuggled in.
fn check_sha(sha: &str) -> Result<(), String> {
    if sha.len() >= 4 && sha.len() <= 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(format!("{sha:?} is not a commit id"))
    }
}

fn git_bytes(repo: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out =
        Command::new("git").arg("-C").arg(repo).args(args).stdin(Stdio::null()).output().map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout)
}

fn git_text(repo: &Path, args: &[&str]) -> Result<String, String> {
    Ok(String::from_utf8_lossy(&git_bytes(repo, args)?).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn files(patterns: &[&str]) -> Inputs {
        Inputs::Files(patterns.iter().map(|p| p.to_string()).collect())
    }

    /// A git repository with a committer identity, for the tests that commit.
    struct Repo {
        dir: tempfile::TempDir,
    }

    impl Repo {
        fn new() -> Repo {
            let dir = tempfile::tempdir().unwrap();
            let repo = Repo { dir };
            repo.git(&["init", "-q", "-b", "main"]);
            repo
        }

        fn path(&self) -> PathBuf {
            self.dir.path().to_path_buf()
        }

        fn git(&self, args: &[&str]) -> String {
            let out = Command::new("git")
                .arg("-C")
                .arg(self.dir.path())
                .args(["-c", "user.name=genie", "-c", "user.email=genie@example.com", "-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }

        fn write(&self, path: &str, text: &str) {
            let p = self.dir.path().join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }

        /// Commit everything and return the commit id.
        fn commit(&self, message: &str) -> String {
            self.git(&["add", "-A"]);
            self.git(&["commit", "-q", "--allow-empty", "-m", message]);
            self.git(&["rev-parse", "HEAD"])
        }
    }

    #[test]
    fn the_matcher_supports_star_within_a_segment_question_marks_and_double_stars() {
        assert!(matches("*.json", "package.json"));
        assert!(!matches("*.json", "dir/package.json"), "a star does not cross a slash");
        assert!(matches("*", ".hidden"), "a star matches dot files too");
        assert!(matches("src/?.rs", "src/a.rs"));
        assert!(!matches("src/?.rs", "src/ab.rs"));
        assert!(matches("**/Cargo.toml", "Cargo.toml"), "a double star takes zero directories");
        assert!(matches("**/Cargo.toml", "crates/x/Cargo.toml"));
        assert!(matches("gradle/**", "gradle/wrapper/gradle-wrapper.jar"));
        assert!(matches("./Cargo.lock", "Cargo.lock"));
        assert!(matches("api/", "api/src/main.rs"), "a trailing slash names a directory's contents");
        assert!(!matches("api", "api2/x"));
    }

    #[test]
    fn changing_an_input_file_changes_its_hash_and_changing_another_file_does_not() {
        let repo = Repo::new();
        repo.write("package.json", "{}\n");
        repo.write("README.md", "one\n");
        let first = repo.commit("first");
        let inputs = files(&["package.json"]);
        let before = input_hashes(&repo.path(), &first, &inputs).unwrap();
        assert_eq!(before.len(), 1);
        assert_eq!(before["package.json"].len(), 40, "the blob id");

        repo.write("README.md", "two\n");
        let readme = repo.commit("readme");
        assert_eq!(input_hashes(&repo.path(), &readme, &inputs).unwrap(), before, "a non-input file leaves the hashes alone");

        repo.write("package.json", "{\"x\":1}\n");
        let changed = repo.commit("package");
        assert_ne!(input_hashes(&repo.path(), &changed, &inputs).unwrap(), before);
    }

    #[test]
    fn an_absent_input_becomes_present_when_its_file_is_committed() {
        let repo = Repo::new();
        repo.write("README.md", "x\n");
        let first = repo.commit("first");
        let inputs = files(&["Cargo.lock"]);
        let absent = input_hashes(&repo.path(), &first, &inputs).unwrap();
        assert_eq!(absent, BTreeMap::from([("Cargo.lock".to_string(), "absent".to_string())]));

        repo.write("Cargo.lock", "lock\n");
        let present = repo.commit("lock");
        let map = input_hashes(&repo.path(), &present, &inputs).unwrap();
        assert_ne!(map, absent);
        assert!(!map.values().any(|v| v == "absent"));
    }

    #[test]
    fn repository_inputs_hash_the_tree_id() {
        let repo = Repo::new();
        repo.write("a.txt", "a\n");
        let first = repo.commit("first");
        let map = input_hashes(&repo.path(), &first, &Inputs::default()).unwrap();
        let tree = repo.git(&["rev-parse", &format!("{first}^{{tree}}")]);
        assert_eq!(map, BTreeMap::from([("tree".to_string(), tree.clone())]));

        repo.write("a.txt", "b\n");
        let second = repo.commit("second");
        let changed = input_hashes(&repo.path(), &second, &Inputs::default()).unwrap();
        assert_ne!(changed, map, "any change in the repository changes the tree");
    }

    #[test]
    fn uncommitted_changes_do_not_change_the_hashes_or_the_context() {
        let repo = Repo::new();
        repo.write("package.json", "{}\n");
        let commit = repo.commit("first");
        let inputs = files(&["package.json"]);
        let before = input_hashes(&repo.path(), &commit, &inputs).unwrap();
        repo.write("package.json", "dirty\n");
        repo.write("new.json", "untracked\n");
        assert_eq!(input_hashes(&repo.path(), &commit, &inputs).unwrap(), before);

        let ctx = tempfile::tempdir().unwrap();
        stage_context(&repo.path(), &commit, &inputs, ctx.path()).unwrap();
        assert_eq!(fs::read_to_string(ctx.path().join("package.json")).unwrap(), "{}\n");
    }

    #[test]
    fn stage_context_writes_only_the_matched_files_with_their_modes() {
        let repo = Repo::new();
        repo.write("scripts/build.sh", "#!/bin/sh\necho build\n");
        repo.write("scripts/notes.txt", "not an input\n");
        repo.write("lib/tool.sh", "#!/bin/sh\n");
        repo.write("Cargo.toml", "[package]\n");
        repo.git(&["add", "-A"]);
        repo.git(&["update-index", "--chmod=+x", "scripts/build.sh"]);
        repo.git(&["commit", "-q", "-m", "first"]);
        let commit = repo.git(&["rev-parse", "HEAD"]);

        let dest = tempfile::tempdir().unwrap();
        let ctx = dest.path().join("ctx");
        stage_context(&repo.path(), &commit, &files(&["scripts/*.sh", "Cargo.toml"]), &ctx).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let script = ctx.join("scripts/build.sh");
        assert_eq!(fs::read_to_string(&script).unwrap(), "#!/bin/sh\necho build\n");
        assert_eq!(fs::metadata(&script).unwrap().permissions().mode() & 0o777, 0o755, "the executable bit is kept");
        assert!(ctx.join("Cargo.toml").is_file());
        assert!(!ctx.join("scripts/notes.txt").exists(), "a file no pattern matches is not written");
        assert!(!ctx.join("lib").exists());
    }

    #[test]
    fn repository_context_is_the_whole_tree_at_the_commit() {
        let repo = Repo::new();
        repo.write("a/one.txt", "1\n");
        repo.write("two.txt", "2\n");
        let commit = repo.commit("first");
        let dest = tempfile::tempdir().unwrap();
        stage_context(&repo.path(), &commit, &Inputs::default(), dest.path()).unwrap();
        assert_eq!(fs::read_to_string(dest.path().join("a/one.txt")).unwrap(), "1\n");
        assert_eq!(fs::read_to_string(dest.path().join("two.txt")).unwrap(), "2\n");
    }

    #[test]
    fn a_revision_that_is_not_a_commit_id_is_refused() {
        let repo = Repo::new();
        repo.write("x", "x");
        repo.commit("first");
        let err = input_hashes(&repo.path(), "--output=/tmp/x", &files(&["x"])).unwrap_err();
        assert!(err.contains("not a commit id"), "{err}");
    }
}
