//! The rootless Podman runner against a real Podman.
//!
//! They run only with `GENIE_TEST_PODMAN=1` (CI sets it on ubuntu-latest, where rootless podman is
//! installed); elsewhere each test prints a note and returns. Images: `docker.io/library/busybox` and
//! `docker.io/library/debian:bookworm-slim`. Every workspace, base and directory a test makes carries a
//! unique suffix and is removed when the test ends, even when it fails. Limits are zero here: cgroup
//! delegation differs between hosts, and these tests are about the runner's plumbing.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use genie::config::EnvironmentsConfig;
use genie::env::podman::PodmanRunner;
use genie::env::runner::{BaseRef, BaseSpec, Exec, ExecResult, Mount, MountMode, Resource, Runner, StepRun, WorkspaceSpec};
use genie_core::env::caps::{Limits, ViewMode};
use genie_core::env::spec::{NetworkMode, Probe};

const BUSYBOX: &str = "docker.io/library/busybox:latest";
const DEBIAN: &str = "docker.io/library/debian:bookworm-slim";

/// Skips a test unless the Podman tests are enabled. The note goes to stderr, where skips are reported.
fn enabled() -> bool {
    if std::env::var("GENIE_TEST_PODMAN").as_deref() == Ok("1") {
        return true;
    }
    eprintln!("skipped: GENIE_TEST_PODMAN is not 1 (the Podman tests need rootless podman)");
    false
}

/// A lowercase hex suffix no other test or run has: the process id, the time and a counter.
fn suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("{:x}{:x}{:x}", std::process::id(), nanos % 0xffff_ffff, COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn no_limits() -> Limits {
    Limits { memory: 0, cpus: 0.0, pids: 0, disk: 0 }
}

/// A test's runner, directory and the names it made, removed on drop.
struct Lab {
    runner: PodmanRunner,
    dir: tempfile::TempDir,
    suffix: String,
    workspaces: RefCell<Vec<String>>,
    images: RefCell<Vec<String>>,
}

impl Lab {
    fn new() -> Lab {
        Lab {
            runner: PodmanRunner::new(&EnvironmentsConfig::default()),
            dir: tempfile::tempdir().expect("a temporary directory"),
            suffix: suffix(),
            workspaces: RefCell::new(Vec::new()),
            images: RefCell::new(Vec::new()),
        }
    }

    /// A path under the test's directory; its parent directories are created.
    fn path(&self, name: &str) -> PathBuf {
        let p = self.dir.path().join(name);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("a test directory");
        }
        p
    }

    /// A workspace id unique to this test, removed when the test ends.
    fn workspace(&self, what: &str) -> String {
        let id = format!("{}-{what}", self.suffix);
        self.workspaces.borrow_mut().push(id.clone());
        id
    }

    /// A base tag unique to this test; the image built under it is removed when the test ends.
    fn tag(&self) -> String {
        self.images.borrow_mut().push(format!("localhost/genie-base:{}", self.suffix));
        format!("genie-base:{}", self.suffix)
    }

    /// The pulled base for an image, pinned by digest.
    fn pulled(&self, image: &str) -> BaseRef {
        let spec =
            BaseSpec { image: Some(image.into()), packages: Vec::new(), containerfile: None, tag: format!("genie-base:{}", self.suffix) };
        self.runner.ensure_base(&spec).expect("pulling the base image")
    }

    fn spec(&self, id: &str, base: &BaseRef, mounts: Vec<Mount>, network: NetworkMode) -> WorkspaceSpec {
        WorkspaceSpec {
            id: id.into(),
            base: base.clone(),
            mounts,
            env: BTreeMap::new(),
            secrets: Vec::new(),
            network,
            limits: no_limits(),
            services: Vec::new(),
            cwd: String::new(),
            run_dir: self.path(&format!("run-{id}")),
        }
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        for ws in self.workspaces.borrow().iter() {
            let _ = self.runner.remove(ws);
        }
        for image in self.images.borrow().iter() {
            let _ = Command::new("podman").args(["rmi", "-f", image]).stdin(Stdio::null()).output();
        }
        // Files written by container users with subuid ownership cannot be removed by the test's own user.
        let _ = self.runner.remove_tree(self.dir.path());
    }
}

fn exec(command: &str) -> Exec {
    Exec { cwd: "/".into(), command: command.into(), env: BTreeMap::new(), timeout: Some(Duration::from_secs(120)), log: None }
}

/// Runs a command in a workspace and checks it exited 0.
fn run_ok(runner: &PodmanRunner, ws: &str, command: &str) -> ExecResult {
    let r = runner.exec(ws, &exec(command)).unwrap_or_else(|e| panic!("exec `{command}`: {e}"));
    assert_eq!(r.code, 0, "exec `{command}` in {ws}: {}", r.output);
    r
}

/// Polls an attempt until it succeeds, for up to 90 seconds.
fn eventually(what: &str, mut attempt: impl FnMut() -> Result<bool, String>) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        match attempt() {
            Ok(true) => return,
            Ok(false) => {}
            Err(e) => panic!("{what}: {e}"),
        }
        assert!(Instant::now() < deadline, "{what}: not true after 90 s");
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn step(base: &BaseRef, ws: &str, command: &str, mounts: Vec<Mount>, network: NetworkMode, log: &Path, timeout: Duration) -> StepRun {
    StepRun {
        base: base.clone(),
        mounts,
        env: BTreeMap::new(),
        secrets: Vec::new(),
        cwd: "/".into(),
        command: command.into(),
        network,
        limits: no_limits(),
        timeout,
        log: log.to_path_buf(),
        labels: BTreeMap::from([("genie.workspace".to_string(), ws.to_string())]),
    }
}

fn mount(source: &Path, target: &str, mode: MountMode) -> Mount {
    Mount { source: source.to_path_buf(), target: target.into(), mode }
}

/// A server on the host's loopback that answers the next `requests` connections with `BODY`. It runs
/// on its own thread and is not joined: a test that fails early leaves it blocked, which ends with the process.
const BODY: &str = "genie-host-ok\n";

fn host_server(requests: usize) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let port = listener.local_addr().expect("the port").port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().take(requests).flatten() {
            let mut head = [0u8; 4096];
            let _ = stream.read(&mut head);
            let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{BODY}", BODY.len());
            let _ = stream.write_all(response.as_bytes());
        }
    });
    port
}

/// The interface names listed in `/proc/net/dev`: two header lines, then one line per interface.
fn interfaces(proc_net_dev: &str) -> Vec<String> {
    proc_net_dev.lines().skip(2).filter_map(|l| l.split_once(':')).map(|(name, _)| name.trim().to_string()).collect()
}

/// The resources the runner lists for one workspace.
fn resources_of(runner: &PodmanRunner, ws: &str) -> Vec<Resource> {
    runner.list().expect("listing the runner's resources").into_iter().filter(|r| r.workspace == ws).collect()
}

/// `podman` with arguments, its stdout trimmed. For checks the runner does not expose.
fn podman_query(args: &[&str]) -> String {
    let out = Command::new("podman").args(args).stdin(Stdio::null()).output().expect("running podman");
    assert!(out.status.success(), "podman {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn podman_reports_rootless_capabilities_for_this_host() {
    if !enabled() {
        return;
    }
    let lab = Lab::new();
    let caps = lab.runner.capabilities().expect("rootless podman is required for these tests");
    assert_eq!(caps.runner, "podman");
    assert!(!caps.version.is_empty(), "the version is reported");
    assert!(matches!(caps.arch.as_str(), "x86_64" | "aarch64"), "arch {}", caps.arch);
    assert!(caps.views.contains(&ViewMode::Copy));
    assert!(caps.views.contains(&ViewMode::Overlay), "overlay views need podman 4 or later: {}", caps.version);
    assert!(caps.network.contains(&NetworkMode::None) && caps.network.contains(&NetworkMode::Full));
    assert!(caps.base_images && caps.services && caps.harness && caps.canonical_paths && caps.secrets);
}

#[test]
fn ensure_base_pulls_an_image_and_names_it_by_digest() {
    if !enabled() {
        return;
    }
    let lab = Lab::new();
    let base = lab.pulled(BUSYBOX);
    assert!(base.reference.starts_with(BUSYBOX.split(':').next().unwrap()), "{}", base.reference);
    assert!(base.reference.contains("@sha256:"), "the reference is pinned: {}", base.reference);
    let hex = base.digest.strip_prefix("sha256:").expect("a sha256 digest");
    assert_eq!(hex.len(), 64, "{}", base.digest);
    assert_eq!(lab.runner.image_digest(BUSYBOX).unwrap(), base.digest, "the same image has the same digest");
}

#[test]
fn a_base_with_packages_is_built_once_and_reused_with_its_packages_installed() {
    if !enabled() {
        return;
    }
    let lab = Lab::new();
    let tag = lab.tag();
    let spec = BaseSpec { image: Some(DEBIAN.into()), packages: vec!["jq".into()], containerfile: None, tag: tag.clone() };
    let first = lab.runner.ensure_base(&spec).expect("building the base with jq");
    assert_eq!(first.reference, format!("localhost/{tag}"));
    let id = podman_query(&["image", "inspect", "--format", "{{.Id}}", &first.reference]);

    let second = lab.runner.ensure_base(&spec).expect("reusing the base");
    assert_eq!(second, first, "the same base is named the same way");
    let id_after = podman_query(&["image", "inspect", "--format", "{{.Id}}", &first.reference]);
    assert_eq!(id_after, id, "the second call reuses the built image rather than building it again");

    let version = lab.runner.probe_tool(&first, "jq --version").expect("jq is in the base");
    assert!(version.starts_with("jq-"), "{version}");
}

#[test]
fn a_step_writes_to_its_rw_mount_sees_a_ro_mount_read_only_and_has_no_network_when_none() {
    if !enabled() {
        return;
    }
    let lab = Lab::new();
    let base = lab.pulled(BUSYBOX);
    let ws = lab.workspace("steps");
    let out = lab.path("out");
    fs::create_dir_all(&out).unwrap();
    let ro = lab.path("ro");
    fs::create_dir_all(&ro).unwrap();
    fs::write(ro.join("hello"), "hello").unwrap();
    let log = lab.path("logs/step.log");
    let command = "echo marker-from-step; \
        echo done > /out/result; \
        cat /ro/hello > /out/ro-read; \
        if echo x > /ro/hello 2>/dev/null; then echo writable > /out/ro-status; else echo readonly > /out/ro-status; fi; \
        cat /proc/net/dev > /out/net";
    let mounts = vec![mount(&out, "/out", MountMode::ReadWrite), mount(&ro, "/ro", MountMode::ReadOnly)];
    let r =
        lab.runner.run_step(&step(&base, &ws, command, mounts, NetworkMode::None, &log, Duration::from_secs(120))).expect("the step runs");
    assert_eq!(r.code, 0, "{}", r.output);
    assert!(r.output.contains("marker-from-step"), "{}", r.output);

    assert_eq!(fs::read_to_string(out.join("result")).unwrap().trim(), "done", "the rw mount takes writes");
    assert_eq!(fs::read_to_string(out.join("ro-read")).unwrap().trim(), "hello");
    assert_eq!(fs::read_to_string(out.join("ro-status")).unwrap().trim(), "readonly", "the ro mount refuses writes");
    assert_eq!(fs::read_to_string(ro.join("hello")).unwrap(), "hello", "the host file is unchanged");
    let net = fs::read_to_string(out.join("net")).unwrap();
    assert_eq!(interfaces(&net), ["lo"], "network none has only loopback");

    let logged = fs::read_to_string(&log).expect("the log file");
    assert!(logged.contains("marker-from-step"), "the log holds the output: {logged}");
}

#[test]
fn a_step_with_full_network_has_an_interface_beyond_loopback_and_the_host_address() {
    if !enabled() {
        return;
    }
    let lab = Lab::new();
    let base = lab.pulled(BUSYBOX);
    let ws = lab.workspace("full");
    let out = lab.path("out");
    fs::create_dir_all(&out).unwrap();
    let log = lab.path("logs/full.log");
    let command = "cat /proc/net/dev > /out/net; echo \"$GENIE_HOST_ADDRESS\" > /out/host";
    let mounts = vec![mount(&out, "/out", MountMode::ReadWrite)];
    let r =
        lab.runner.run_step(&step(&base, &ws, command, mounts, NetworkMode::Full, &log, Duration::from_secs(120))).expect("the step runs");
    assert_eq!(r.code, 0, "{}", r.output);
    let names = interfaces(&fs::read_to_string(out.join("net")).unwrap());
    assert!(names.iter().any(|n| n != "lo"), "full network has a non-loopback interface: {names:?}");
    assert_eq!(fs::read_to_string(out.join("host")).unwrap().trim(), "10.0.2.2", "the host address is in the environment");
}

#[test]
fn a_step_that_outlives_its_timeout_is_killed_and_its_container_removed() {
    if !enabled() {
        return;
    }
    let lab = Lab::new();
    let base = lab.pulled(BUSYBOX);
    let ws = lab.workspace("timeout");
    let log = lab.path("logs/timeout.log");
    let started = Instant::now();
    let err = lab
        .runner
        .run_step(&step(&base, &ws, "sleep 300", Vec::new(), NetworkMode::None, &log, Duration::from_secs(5)))
        .expect_err("the step is killed at its timeout");
    assert!(err.contains("timed out"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(120), "the timeout is enforced promptly");
    assert!(resources_of(&lab.runner, &ws).is_empty(), "the step's container is removed");
}

#[test]
fn a_workspace_execs_into_an_overlay_view_and_two_views_over_one_lower_directory_are_isolated() {
    if !enabled() {
        return;
    }
    let lab = Lab::new();
    let base = lab.pulled(BUSYBOX);
    let lower = lab.path("lower");
    fs::create_dir_all(&lower).unwrap();
    fs::write(lower.join("seed.txt"), "lower\n").unwrap();
    let overlay = |what: &str| Mount {
        source: lower.clone(),
        target: "/workspace".into(),
        mode: MountMode::Overlay { upper: lab.path(&format!("upper-{what}")), work: lab.path(&format!("work-{what}")) },
    };
    let a = lab.workspace("view-a");
    let b = lab.workspace("view-b");
    lab.runner.create(&lab.spec(&a, &base, vec![overlay("a")], NetworkMode::None)).expect("creating a");
    lab.runner.create(&lab.spec(&b, &base, vec![overlay("b")], NetworkMode::None)).expect("creating b");

    run_ok(&lab.runner, &a, "echo from-a > /workspace/a.txt; echo changed-by-a > /workspace/seed.txt");
    run_ok(&lab.runner, &b, "test ! -e /workspace/a.txt && grep -qx lower /workspace/seed.txt && echo from-b > /workspace/b.txt");

    assert_eq!(run_ok(&lab.runner, &a, "cat /workspace/a.txt").output.trim(), "from-a");
    assert_eq!(run_ok(&lab.runner, &a, "cat /workspace/seed.txt").output.trim(), "changed-by-a");
    assert_eq!(run_ok(&lab.runner, &b, "cat /workspace/b.txt").output.trim(), "from-b");
    assert_eq!(run_ok(&lab.runner, &b, "cat /workspace/seed.txt").output.trim(), "lower", "a's write is invisible in b");
    let missing = lab.runner.exec(&b, &exec("test -e /workspace/a.txt")).unwrap();
    assert_ne!(missing.code, 0, "a's new file is invisible in b");

    assert_eq!(fs::read_to_string(lower.join("seed.txt")).unwrap(), "lower\n", "the lower directory is unchanged");
    assert!(!lower.join("a.txt").exists() && !lower.join("b.txt").exists(), "no view wrote into the lower directory");
}

#[test]
fn a_started_process_becomes_ready_on_a_tcp_listener_and_its_pid_is_written() {
    if !enabled() {
        return;
    }
    let lab = Lab::new();
    // netcat gives the base a listener; bash in the base makes the tcp probe use /dev/tcp.
    let base = lab
        .runner
        .ensure_base(&BaseSpec { image: Some(DEBIAN.into()), packages: vec!["netcat-openbsd".into()], containerfile: None, tag: lab.tag() })
        .expect("building the base with netcat");
    let ws = lab.workspace("probes");
    let spec = lab.spec(&ws, &base, Vec::new(), NetworkMode::None);
    let run_dir = spec.run_dir.clone();
    lab.runner.create(&spec).expect("creating the workspace");

    let tcp = Probe { command: None, http: None, tcp: Some("127.0.0.1:8080".into()), timeout_secs: Some(5) };
    assert!(!lab.runner.probe(&ws, &tcp).unwrap(), "nothing listens before the process starts");

    let listener = "while true; do nc -l -p 8080 </dev/null >/dev/null 2>&1; done";
    lab.runner
        .start_process(&ws, "listener", &Exec { cwd: "/".into(), command: listener.into(), env: BTreeMap::new(), timeout: None, log: None })
        .expect("starting the listener");

    let pid = fs::read_to_string(run_dir.join("listener.pid")).expect("the pid file");
    assert!(pid.trim().parse::<u32>().is_ok(), "a pid: {pid:?}");
    eventually("the tcp probe", || lab.runner.probe(&ws, &tcp));

    let command = Probe { command: Some("test -s /opt/genie-run/listener.pid".into()), http: None, tcp: None, timeout_secs: Some(5) };
    assert!(lab.runner.probe(&ws, &command).unwrap(), "a command probe that passes is ready");
    let failing = Probe { command: Some("false".into()), http: None, tcp: None, timeout_secs: Some(5) };
    assert!(!lab.runner.probe(&ws, &failing).unwrap(), "a command that exits non-zero is not ready");
    let http = Probe { command: None, http: Some("http://127.0.0.1:8080/".into()), tcp: None, timeout_secs: Some(5) };
    assert!(lab.runner.probe(&ws, &http).unwrap(), "an http probe falls back to the tcp check where no curl or wget is");
}

#[test]
fn stop_keeps_the_workspace_data_and_create_starts_it_again() {
    if !enabled() {
        return;
    }
    let lab = Lab::new();
    let base = lab.pulled(BUSYBOX);
    let ws = lab.workspace("restart");
    let spec = lab.spec(&ws, &base, Vec::new(), NetworkMode::None);
    lab.runner.create(&spec).expect("creating");
    lab.runner.create(&spec).expect("creating again is idempotent");
    run_ok(&lab.runner, &ws, "echo kept > /tmp/marker");

    lab.runner.stop(&ws).expect("stopping");
    let stopped = resources_of(&lab.runner, &ws);
    assert!(stopped.iter().any(|r| r.kind == "main" && !r.running), "the main container is stopped: {stopped:?}");

    lab.runner.create(&spec).expect("create starts a stopped workspace");
    let running = resources_of(&lab.runner, &ws);
    assert!(running.iter().any(|r| r.kind == "main" && r.running), "{running:?}");
    assert_eq!(run_ok(&lab.runner, &ws, "cat /tmp/marker").output.trim(), "kept", "the data survived the stop");
    lab.runner.stop(&ws).expect("stopping again is fine");
}

#[test]
fn remove_leaves_no_runtime_objects_and_keeps_the_source_directories() {
    if !enabled() {
        return;
    }
    let lab = Lab::new();
    let base = lab.pulled(BUSYBOX);
    let ws = lab.workspace("gone");
    let source = lab.path("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("keep.txt"), "mine").unwrap();
    let spec = lab.spec(&ws, &base, vec![mount(&source, "/workspace", MountMode::ReadWrite)], NetworkMode::None);
    lab.runner.create(&spec).expect("creating");
    assert!(!resources_of(&lab.runner, &ws).is_empty());

    lab.runner.remove(&ws).expect("removing");
    assert!(resources_of(&lab.runner, &ws).is_empty(), "no pod or container is left");
    lab.runner.remove(&ws).expect("removing again is fine");
    assert_eq!(fs::read_to_string(source.join("keep.txt")).unwrap(), "mine", "the source directory is never touched");
}

#[test]
fn remove_tree_deletes_files_a_container_user_with_subuid_ownership_created() {
    if !enabled() {
        return;
    }
    let lab = Lab::new();
    lab.pulled(BUSYBOX);
    let root = lab.path("tree");
    fs::create_dir_all(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o777)).unwrap();
    // Uid 1 in the container is a subuid on the host, not this user: the files it writes are not ours.
    let made = Command::new("podman")
        .args(["run", "--rm", "--network", "none", "--user", "1:1", "-v", &format!("{}:/tree", root.display()), BUSYBOX])
        .args(["sh", "-c", "mkdir -p /tree/sub && echo x > /tree/owned && echo y > /tree/sub/inner"])
        .stdin(Stdio::null())
        .output()
        .expect("running podman");
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    let mine = fs::metadata(lab.dir.path()).unwrap().uid();
    assert_ne!(fs::metadata(root.join("owned")).unwrap().uid(), mine, "the file is owned by a subuid");

    lab.runner.remove_tree(&root).expect("removing the tree");
    assert!(!root.exists(), "the whole tree is gone");
}

#[test]
fn the_harness_command_forwards_named_variables_and_no_values_on_its_command_line() {
    if !enabled() {
        return;
    }
    let lab = Lab::new();
    let base = lab.pulled(BUSYBOX);
    let ws = lab.workspace("harness");
    lab.runner.create(&lab.spec(&ws, &base, Vec::new(), NetworkMode::None)).expect("creating");

    let name = "GENIE_TEST_FORWARDED";
    let value = "forwarded-7f3a9c";
    let argv = vec!["sh".to_string(), "-c".to_string(), format!("echo \"${name}\" > /tmp/forwarded")];
    let (program, args) = lab.runner.harness(&ws, "/", &[name.to_string()], &argv).expect("the harness command");
    assert!(!args.iter().any(|a| a.contains(value)), "the value is not on the command line: {args:?}");
    let status = Command::new(&program).args(&args).env(name, value).stdin(Stdio::null()).status().expect("running the harness");
    assert!(status.success(), "the harness command exits 0");
    assert_eq!(run_ok(&lab.runner, &ws, "cat /tmp/forwarded").output.trim(), value);
}

#[test]
fn full_network_steps_and_workspaces_reach_the_host_loopback_through_the_host_address() {
    if !enabled() {
        return;
    }
    let lab = Lab::new();
    let base = lab.pulled(BUSYBOX);
    let port = host_server(2);
    // The address comes from GENIE_HOST_ADDRESS inside the container, so the test does not assume it.
    let url = format!("http://$GENIE_HOST_ADDRESS:{port}/");

    let out = lab.path("out");
    fs::create_dir_all(&out).unwrap();
    let step_ws = lab.workspace("host-step");
    let command = format!("wget -q -O /out/body {url}");
    let mounts = vec![mount(&out, "/out", MountMode::ReadWrite)];
    let log = lab.path("logs/host-step.log");
    let r = lab
        .runner
        .run_step(&step(&base, &step_ws, &command, mounts, NetworkMode::Full, &log, Duration::from_secs(120)))
        .expect("the step runs");
    assert_eq!(r.code, 0, "a full-network step reaches the host loopback: {}", r.output);
    assert_eq!(fs::read_to_string(out.join("body")).unwrap(), BODY, "the body came from the host server");

    let ws = lab.workspace("host-exec");
    lab.runner.create(&lab.spec(&ws, &base, Vec::new(), NetworkMode::Full)).expect("creating a full-network workspace");
    assert_eq!(run_ok(&lab.runner, &ws, &format!("wget -q -O - {url}")).output, BODY, "an exec reaches the host loopback too");
}
