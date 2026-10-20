//! Git hosts for tests: bare repositories on disk behind a `plain` host of `git.json`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// Run `git` in `dir` and return its trimmed output (panics on failure).
pub fn sh(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com", "-c", "init.defaultBranch=main"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Like [`sh`], but returns whether it succeeded and everything it printed.
pub fn try_sh(dir: &Path, envs: &[(&str, &str)], args: &[&str]) -> (bool, String) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args(args)
        .envs(envs.iter().copied())
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

/// A bare repository `<hosts>/<remote>.git` with one commit on `main` (files: name → content).
pub fn upstream(hosts: &Path, remote: &str, files: &[(&str, &str)]) -> PathBuf {
    use std::io::Write;
    use std::process::Stdio;

    let bare = hosts.join(format!("{remote}.git"));
    std::fs::create_dir_all(&bare).unwrap();
    sh(&bare, &["init", "--bare", "-q", "-b", "main"]);
    // Build a real initial commit directly: no disposable worktree, index, or setup push.
    let mut input = String::from("commit refs/heads/main\ncommitter test <test@example.com> 0 +0000\ndata 5\ninit\n");
    for (path, content) in files {
        input.push_str(&format!("M 100644 inline {}\ndata {}\n{content}\n", serde_json::to_string(path).unwrap(), content.len()));
    }
    input.push_str("\ndone\n");
    let mut child = std::process::Command::new("git")
        .arg("-C")
        .arg(&bare)
        .args(["fast-import", "--quiet"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "git fast-import: {}", String::from_utf8_lossy(&output.stderr));
    bare
}

/// Write `<data>/git.json` with a `plain` host `files` whose repositories are `<hosts>/<remote>.git`.
pub fn write_git_json(data: &Path, hosts: &Path) {
    let cfg = serde_json::json!({ "hosts": { "files": { "kind": "plain", "url": format!("file://{}", hosts.display()) } } });
    std::fs::write(data.join("git.json"), cfg.to_string()).unwrap();
}
