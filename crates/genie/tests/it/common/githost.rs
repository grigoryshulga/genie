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
    let bare = hosts.join(format!("{remote}.git"));
    std::fs::create_dir_all(&bare).unwrap();
    sh(&bare, &["init", "--bare", "-q", "-b", "main"]);
    let work = hosts.join(format!("seed-{}", remote.replace('/', "-")));
    std::fs::create_dir_all(&work).unwrap();
    sh(&work, &["init", "-q", "-b", "main"]);
    for (name, content) in files {
        let p = work.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }
    sh(&work, &["add", "."]);
    sh(&work, &["commit", "-q", "-m", "init"]);
    sh(&work, &["push", "-q", &bare.to_string_lossy(), "HEAD:refs/heads/main"]);
    bare
}

/// Write `<data>/git.json` with a `plain` host `files` whose repositories are `<hosts>/<remote>.git`.
pub fn write_git_json(data: &Path, hosts: &Path) {
    let cfg = serde_json::json!({ "hosts": { "files": { "kind": "plain", "url": format!("file://{}", hosts.display()) } } });
    std::fs::write(data.join("git.json"), cfg.to_string()).unwrap();
}
