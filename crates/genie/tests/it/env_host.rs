//! The host runner over real processes: steps and their published snapshots, the copy views of two
//! workspaces, a workspace's commands and detached processes, timeouts, and, when bubblewrap works,
//! the sandbox's read-only mounts and network isolation. Everything runs in temporary directories.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use genie::config::EnvironmentsConfig;
use genie::env::host::HostRunner;
use genie::env::runner::{BaseRef, Exec, Mount, MountMode, Runner, StepRun, WorkspaceSpec};
use genie::env::store::{Store, remove_tree};
use genie_core::env::caps::{Limits, ViewMode};
use genie_core::env::spec::{NetworkMode, Probe};

/// A store in a temporary directory, and a host runner over it.
struct Fixture {
    _dir: tempfile::TempDir,
    data: PathBuf,
    store: Store,
    runner: HostRunner,
}

fn fixture(sandbox: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().to_path_buf();
    let store = Store::open(&EnvironmentsConfig::default(), &data).unwrap();
    let runner = HostRunner::new(store.root.clone(), sandbox);
    Fixture { _dir: dir, data, store, runner }
}

fn base() -> BaseRef {
    BaseRef { reference: "host".into(), digest: format!("host:{}-{}", std::env::consts::OS, std::env::consts::ARCH) }
}

fn limits() -> Limits {
    Limits { memory: 0, cpus: 1.0, pids: 1, disk: 0 }
}

/// A workspace of the host runner: its home is its directory, which is also its working directory.
fn workspace(f: &Fixture, id: &str, env: &[(&str, &str)]) -> (WorkspaceSpec, PathBuf) {
    let home = genie::env::store::workspace_home(&f.store.root, id);
    let spec = WorkspaceSpec {
        id: id.into(),
        base: base(),
        mounts: Vec::new(),
        env: env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        secrets: Vec::new(),
        network: NetworkMode::Full,
        limits: limits(),
        services: Vec::new(),
        cwd: home.to_string_lossy().into_owned(),
        run_dir: genie::env::store::run_dir(&f.store.root, id),
    };
    (spec, home)
}

/// A step of a preparation, run in `cwd` with its outputs and log in the staging directory of `key`.
fn step(f: &Fixture, key: &str, cwd: &Path, command: String, mounts: Vec<Mount>, network: NetworkMode, timeout: Duration) -> StepRun {
    StepRun {
        base: base(),
        mounts,
        env: BTreeMap::new(),
        secrets: Vec::new(),
        cwd: cwd.to_string_lossy().into_owned(),
        command,
        network,
        limits: limits(),
        timeout,
        log: f.store.staging_dir(key, 1).join("step.log"),
        labels: BTreeMap::from([("genie.workspace".to_string(), format!("prep-{key}"))]),
    }
}

/// A server that answers every connection with `200 OK` and a short body, until the test ends.
fn answering_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        }
    });
    addr
}

fn pid_of(run: &Path, name: &str) -> u32 {
    std::fs::read_to_string(run.join(format!("{name}.pid"))).unwrap().trim().parse().unwrap()
}

fn process_alive(pid: u32) -> bool {
    // A process that has exited and been reaped has no /proc entry; one that is still running has one.
    Path::new(&format!("/proc/{pid}")).exists()
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_step_output_is_published_and_each_workspace_gets_its_own_copy() {
    let f = fixture(false);
    let key = "deps-key";
    let out0 = f.store.staging_dir(key, 1).join("out/0");
    std::fs::create_dir_all(&out0).unwrap();
    // The output is an identity mount: the step writes straight into the staging directory.
    let mount = Mount { source: out0.clone(), target: out0.to_string_lossy().into_owned(), mode: MountMode::ReadWrite };
    let command = format!("echo built > '{}/lib.txt' && echo finished", out0.display());
    let r = f.runner.run_step(&step(&f, key, &out0, command, vec![mount], NetworkMode::Full, Duration::from_secs(60))).unwrap();
    assert_eq!(r.code, 0, "{}", r.output);
    assert!(r.output.contains("finished"));

    let (size, files) = f.store.publish(key, 1, &["node_modules".to_string()], &remove_tree).unwrap();
    assert_eq!((size, files), (6, 1));
    assert_eq!(f.store.verify_published(key).unwrap().files, 1);

    // Two workspaces each get a copy: a write in one changes neither the snapshot nor the other copy.
    let (api, _) = f.store.view("shop--api", "api/prepare", 0, key, ViewMode::Copy).unwrap();
    let (web, _) = f.store.view("shop--web", "web/prepare", 0, key, ViewMode::Copy).unwrap();
    std::fs::write(api.join("lib.txt"), "api only").unwrap();
    assert_eq!(std::fs::read_to_string(web.join("lib.txt")).unwrap(), "built\n");
    assert_eq!(std::fs::read_to_string(f.store.snapshot_dir(key).join("out/0/lib.txt")).unwrap(), "built\n");
}

#[test]
fn a_created_workspace_runs_commands_with_its_environment_in_its_directory() {
    let f = fixture(false);
    let (spec, home) = workspace(&f, "shop--api", &[("GREETING", "hi")]);
    f.runner.create(&spec).unwrap();
    f.runner.create(&spec).expect("create is idempotent");

    let exec = |command: &str| Exec {
        cwd: home.to_string_lossy().into_owned(),
        command: command.into(),
        env: BTreeMap::new(),
        timeout: Some(Duration::from_secs(30)),
        log: None,
    };
    let r = f.runner.exec("shop--api", &exec("echo \"$GREETING $(pwd) $HOME\"")).unwrap();
    assert_eq!(r.code, 0, "{}", r.output);
    assert_eq!(r.output.trim(), format!("hi {0} {0}", home.display()));

    let listed = f.runner.list().unwrap();
    assert!(listed.iter().any(|r| r.workspace == "shop--api" && r.kind == "workspace" && r.running), "{listed:?}");
    f.runner.remove("shop--api").unwrap();
    assert!(!spec.run_dir.exists(), "the run directory is removed");
    f.runner.remove("shop--api").expect("remove is idempotent");
}

#[test]
fn a_detached_process_answers_probes_and_stop_kills_it() {
    let f = fixture(false);
    let (spec, home) = workspace(&f, "api", &[]);
    f.runner.create(&spec).unwrap();
    let addr = answering_server();

    let http = Probe { command: None, http: Some(format!("http://{addr}/health")), tcp: None, timeout_secs: Some(5) };
    assert!(f.runner.probe("api", &http).unwrap(), "an answering server is ready");
    let tcp = Probe { command: None, http: None, tcp: Some(addr.clone()), timeout_secs: Some(5) };
    assert!(f.runner.probe("api", &tcp).unwrap());
    let nothing = Probe { command: None, http: None, tcp: Some("127.0.0.1:1".into()), timeout_secs: Some(2) };
    assert!(!f.runner.probe("api", &nothing).unwrap(), "a port with no listener is not ready");

    let sleeper =
        Exec { cwd: home.to_string_lossy().into_owned(), command: "exec sleep 600".into(), env: BTreeMap::new(), timeout: None, log: None };
    f.runner.start_process("api", "sleeper", &sleeper).unwrap();
    let pid = pid_of(&spec.run_dir, "sleeper");
    assert!(process_alive(pid));
    f.runner.start_process("api", "sleeper", &sleeper).unwrap();
    assert_eq!(pid_of(&spec.run_dir, "sleeper"), pid, "a process already running under its name is not started again");
    let listed = f.runner.list().unwrap();
    assert!(listed.iter().any(|r| r.kind == "process" && r.name == "sleeper" && r.running), "{listed:?}");

    f.runner.stop("api").unwrap();
    wait_until("the sleeper to die", || !process_alive(pid));
    let listed = f.runner.list().unwrap();
    assert!(listed.iter().any(|r| r.kind == "workspace" && !r.running), "the workspace is stopped: {listed:?}");
    let err = f.runner.exec("api", &sleeper).unwrap_err();
    assert!(err.contains("stopped"), "{err}");
    f.runner.stop("api").expect("stop is idempotent");

    // Creating the workspace again starts it, and it runs commands again.
    f.runner.create(&spec).unwrap();
    let hello = Exec { command: "echo back".into(), ..sleeper.clone() };
    assert_eq!(f.runner.exec("api", &hello).unwrap().output.trim(), "back");
    f.runner.remove("api").unwrap();
}

#[test]
fn a_step_that_runs_past_its_timeout_is_killed() {
    let f = fixture(false);
    let key = "slow";
    std::fs::create_dir_all(f.store.staging_dir(key, 1)).unwrap();
    let started = Instant::now();
    let s = step(&f, key, &f.data, "sleep 120; echo late".into(), Vec::new(), NetworkMode::Full, Duration::from_secs(1));
    let r = f.runner.run_step(&s).unwrap();
    assert_eq!(r.code, 124, "{}", r.output);
    assert!(r.output.contains("timed out"), "{}", r.output);
    assert!(started.elapsed() < Duration::from_secs(30), "the step was not left to run");
    let log = std::fs::read_to_string(&s.log).unwrap();
    assert!(log.contains("timed out"), "the log names the timeout: {log:?}");
}

#[test]
fn bubblewrap_keeps_read_only_mounts_read_only_and_cuts_network_off_for_none() {
    if !genie::sandbox::works() {
        assert!(std::env::var("GENIE_REQUIRE_SANDBOX").is_err(), "bubblewrap does not work here, and GENIE_REQUIRE_SANDBOX is set");
        eprintln!("skipped: bubblewrap does not work on this machine");
        return;
    }
    let f = fixture(true);
    let readonly = f.data.join("readonly");
    std::fs::create_dir_all(&readonly).unwrap();
    std::fs::write(readonly.join("keep.txt"), "kept\n").unwrap();
    let writable = f.data.join("writable");
    std::fs::create_dir_all(&writable).unwrap();
    let addr = answering_server();
    let port = addr.rsplit(':').next().unwrap().to_string();

    let mounts = vec![
        Mount { source: readonly.clone(), target: readonly.to_string_lossy().into_owned(), mode: MountMode::ReadOnly },
        Mount { source: writable.clone(), target: writable.to_string_lossy().into_owned(), mode: MountMode::ReadWrite },
    ];
    let probe = format!(
        "if echo x > '{ro}/new.txt' 2>/dev/null; then echo ro:writable; else echo ro:read-only; fi; \
         if echo x > '{rw}/new.txt' 2>/dev/null; then echo rw:writable; else echo rw:read-only; fi; \
         bash -c 'exec 3<>/dev/tcp/127.0.0.1/{port}' 2>/dev/null && echo net:reachable || echo net:blocked",
        ro = readonly.display(),
        rw = writable.display(),
    );
    let mut none = step(&f, "sandboxed", &writable, probe.clone(), mounts.clone(), NetworkMode::None, Duration::from_secs(60));
    none.labels.insert("genie.workspace".into(), "sandboxed".into());
    let r = f.runner.run_step(&none).unwrap();
    assert!(r.output.contains("ro:read-only"), "{}", r.output);
    assert!(r.output.contains("rw:writable"), "{}", r.output);
    assert!(r.output.contains("net:blocked"), "network none cuts the connection: {}", r.output);
    assert!(!readonly.join("new.txt").exists(), "nothing was written to the read-only mount");
    assert!(writable.join("new.txt").exists(), "the read-write mount takes the write");
    assert_eq!(std::fs::read_to_string(readonly.join("keep.txt")).unwrap(), "kept\n");

    // The same probe with full network reaches the server, so the block above is the sandbox's doing.
    let full = step(&f, "sandboxed", &writable, probe, mounts, NetworkMode::Full, Duration::from_secs(60));
    let r = f.runner.run_step(&full).unwrap();
    assert!(r.output.contains("net:reachable"), "{}", r.output);

    // A workspace in the sandbox runs its commands too.
    let (spec, home) = workspace(&f, "sandboxed-ws", &[]);
    f.runner.create(&spec).unwrap();
    let exec = Exec {
        cwd: home.to_string_lossy().into_owned(),
        command: "echo inside".into(),
        env: BTreeMap::new(),
        timeout: Some(Duration::from_secs(30)),
        log: None,
    };
    let r = f.runner.exec("sandboxed-ws", &exec).unwrap();
    assert_eq!((r.code, r.output.trim()), (0, "inside"), "{}", r.output);
    f.runner.remove("sandboxed-ws").unwrap();
}
