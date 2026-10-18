//! The agent harness of a podman environment: the genie binary with the loader and libraries it
//! runs with, bundled for the container, and the host paths the harness program needs.
//!
//! An agent inside an environment runs on the base image's own libc, which may differ from the
//! server's, and its harness program (pi, say) lives on the host. So the server copies its binary
//! and the shared-library closure into `<env-root>/harness/genie-<hash16>/`, mounted read-only at
//! `/opt/genie`, and mounts the program and the directories it reads at the paths they have on the
//! host. The functions here build those pieces; `runtime` decides when to use them.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use genie_core::env::key::sha256_hex;

use super::layout::GENIE;
use super::runner::{Mount, MountMode};
use super::store;

/// The directory under the environment root that holds the bundles.
const HARNESS_DIR: &str = "harness";
/// Every published bundle is named `genie-<16 hex digits>`.
const BUNDLE_PREFIX: &str = "genie-";
/// Names inside a bundle.
const EXE_NAME: &str = "genie";
const LIB_DIR: &str = "lib";
const BIN_DIR: &str = "bin";
/// Symlinks followed when resolving the harness program; more than this is a cycle.
const MAX_LINKS: usize = 40;
const GIT_CONFIG_COUNT: &str = "GIT_CONFIG_COUNT";
/// What `ldd` prints for a binary it does not link dynamically (glibc, then musl). Lower case.
const STATIC_MARKERS: [&str; 3] = ["not a dynamic executable", "statically linked", "not a valid dynamic program"];

/// A published bundle: the genie binary, its loader and libraries, and the shim that runs them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    /// The host directory, `<env-root>/harness/genie-<hash16>`.
    pub dir: PathBuf,
    /// Where the bundle appears inside an environment (`/opt/genie`).
    pub mount_target: String,
    /// The directory of the shim, first on the agent's PATH (`/opt/genie/bin`).
    pub path_prefix: String,
}

impl Bundle {
    /// The read-only mount of the bundle at [`Bundle::mount_target`].
    pub fn mount(&self) -> Mount {
        Mount { source: self.dir.clone(), target: self.mount_target.clone(), mode: MountMode::ReadOnly }
    }
}

/// Build (or find) the bundle of the genie binary `exe` under `env_root`.
///
/// The bundle is named by the first 16 hex digits of the SHA-256 of the binary's bytes, so a new
/// build gets a new bundle and an unchanged one is reused. It is assembled in a hidden directory
/// and renamed into place, so a published bundle is always complete. Two processes that build the
/// same bundle at once both succeed; one copy is kept.
pub fn bundle(env_root: &Path, exe: &Path) -> Result<Bundle, String> {
    let bytes = fs::read(exe).map_err(|e| format!("cannot read the genie binary {}: {e}", exe.display()))?;
    let name = format!("{BUNDLE_PREFIX}{}", &sha256_hex(&bytes)[..16]);
    let harness = env_root.join(HARNESS_DIR);
    let dir = harness.join(&name);
    let made = Bundle { dir: dir.clone(), mount_target: GENIE.to_string(), path_prefix: format!("{GENIE}/{BIN_DIR}") };
    if dir.is_dir() {
        return Ok(made);
    }
    fs::create_dir_all(&harness).map_err(|e| format!("{}: {e}", harness.display()))?;
    let staged = harness.join(format!(".{name}.{}.{}", std::process::id(), nanos()));
    if let Err(e) = assemble(&staged, &bytes, exe).and_then(|()| store::make_readonly(&staged)) {
        let _ = store::remove_tree(&staged);
        return Err(e);
    }
    match fs::rename(&staged, &dir) {
        Ok(()) => Ok(made),
        // Another process published the same binary first; its bundle is identical to ours.
        Err(_) if dir.is_dir() => {
            let _ = store::remove_tree(&staged);
            Ok(made)
        }
        Err(e) => {
            let _ = store::remove_tree(&staged);
            Err(format!("cannot publish the agent harness at {}: {e}", dir.display()))
        }
    }
}

/// Remove the published bundles under `env_root` other than `keep`, and return the removed
/// directories. Only `genie-*` directories are touched. Call it only when no environment still
/// mounts an older bundle, since a running container keeps the directory it was started with.
pub fn prune_bundles(env_root: &Path, keep: &Bundle) -> Result<Vec<PathBuf>, String> {
    let harness = env_root.join(HARNESS_DIR);
    let entries = match fs::read_dir(&harness) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", harness.display())),
    };
    let mut removed = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", harness.display()))?;
        let path = entry.path();
        let is_bundle = entry.file_name().to_str().is_some_and(|n| n.starts_with(BUNDLE_PREFIX));
        if is_bundle && path != keep.dir {
            store::remove_tree(&path)?;
            removed.push(path);
        }
    }
    removed.sort();
    Ok(removed)
}

/// The mounts that make the harness program and its data visible inside an environment at the
/// paths they have on the host: read-only, except the session directory and `extra_rw`.
///
/// `program` is the harness program as the agent is given it. A bare name is searched for along the
/// server's PATH, which the agent process inherits, so pass the resolved path to be exact. Paths that
/// do not exist are skipped, so create `<data>/sessions/<project>` before calling this. The root and
/// any ancestor of `data` are never mounted, nor are relative or non-UTF-8 paths.
pub fn harness_mounts(program: &Path, data: &Path, project: &str, extra_rw: &[PathBuf]) -> Vec<Mount> {
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    mounts_for(program, &path_var, data, project, extra_rw)
}

/// The argv of the probe that proves the harness program runs inside an environment:
/// `<program> --version`. Run it through the runner's harness wrapper and judge it with
/// [`check_probe`].
pub fn probe_argv(program: &Path) -> Vec<String> {
    vec![program.to_string_lossy().into_owned(), "--version".into()]
}

/// Judge the probe's outcome from its exit code and combined output.
pub fn check_probe(code: i32, output: &str) -> Result<(), String> {
    if code == 0 {
        return Ok(());
    }
    let last = output.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no output").trim();
    Err(harness_cannot_run(&format!("exit status {code}: {last}")))
}

/// The message for an agent whose harness cannot run in an environment's base.
pub fn harness_cannot_run(detail: &str) -> String {
    format!("the agent harness cannot run in this environment's base: {detail}")
}

/// Point an agent's environment at the server from inside an environment.
///
/// `GENIE_URL` becomes `http://<host_address>:<port>`. Git's `url.<that>.insteadOf` maps the loopback
/// proxy URLs the clones use (`http://127.0.0.1:<port>/` and `http://localhost:<port>/`) to it. The
/// mapping is appended after an existing `GIT_CONFIG_COUNT`, so other git settings stay. Call it once
/// per environment: a second call would add the mapping again.
pub fn rewrite_env(env: &mut Vec<(String, String)>, host_address: &str, port: u16) -> Result<(), String> {
    if host_address.trim().is_empty() {
        return Err("environments.podman.hostAddress is empty: agents in an environment cannot reach the server".into());
    }
    let count = match env.iter().rev().find(|(k, _)| k == GIT_CONFIG_COUNT) {
        Some((_, v)) => v.parse::<usize>().map_err(|_| format!("{GIT_CONFIG_COUNT} is not a number: {v:?}"))?,
        None => 0,
    };
    env.retain(|(k, _)| k != "GENIE_URL" && k != GIT_CONFIG_COUNT);
    let host = format!("http://{host_address}:{port}");
    env.push(("GENIE_URL".into(), host.clone()));
    let key = format!("url.{host}/.insteadOf");
    let values = [format!("http://127.0.0.1:{port}/"), format!("http://localhost:{port}/")];
    for (i, value) in values.iter().enumerate() {
        env.push((format!("GIT_CONFIG_KEY_{}", count + i), key.clone()));
        env.push((format!("GIT_CONFIG_VALUE_{}", count + i), value.clone()));
    }
    env.push((GIT_CONFIG_COUNT.into(), (count + values.len()).to_string()));
    Ok(())
}

/// Copy the binary `exe` and its shared libraries into the bundle `staged`.
fn assemble(staged: &Path, bytes: &[u8], exe: &Path) -> Result<(), String> {
    let lib = staged.join(LIB_DIR);
    let bin = staged.join(BIN_DIR);
    for dir in [&lib, &bin] {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    write_file(&staged.join(EXE_NAME), bytes, 0o755)?;
    let shim = match linkage(exe)? {
        Linkage::Static => static_shim_text(),
        Linkage::Dynamic { interpreter, libraries } => {
            let mut copied = BTreeSet::new();
            for item in std::iter::once(&interpreter).chain(&libraries) {
                if copied.insert(item.name.as_str()) {
                    fs::copy(&item.source, lib.join(&item.name))
                        .map_err(|e| format!("cannot copy {} into the agent harness: {e}", item.source.display()))?;
                }
            }
            shim_text(&interpreter.name)
        }
    };
    write_file(&bin.join(EXE_NAME), shim.as_bytes(), 0o755)
}

/// How `ldd` links `exe`. Needs `ldd` on the server's PATH.
fn linkage(exe: &Path) -> Result<Linkage, String> {
    let out = Command::new("ldd").arg(exe).output().map_err(|e| format!("ldd is needed to bundle the agent harness: {e}"))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    parse_ldd(&text)
}

/// A shared object of a binary: the name it is loaded by, which is its file name in the bundle, and
/// the file it resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Lib {
    name: String,
    source: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Linkage {
    Static,
    Dynamic { interpreter: Lib, libraries: Vec<Lib> },
}

/// Parse the output of `ldd <binary>` from glibc or musl. The lines look like
/// `libc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x…)`, the loader is either `/lib64/ld-… (0x…)` or
/// `/lib64/ld-… => /usr/lib64/ld-… (0x…)`, and the kernel's vDSO has no file and is skipped.
fn parse_ldd(text: &str) -> Result<Linkage, String> {
    let mut interpreter = None;
    let mut libraries = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let lower = line.to_ascii_lowercase();
        if STATIC_MARKERS.iter().any(|marker| lower.contains(marker)) {
            return Ok(Linkage::Static);
        }
        if line.contains("not found") {
            return Err(format!("the genie binary needs a library that is not installed: {line}"));
        }
        let (requested, source) = match line.split_once(" => ") {
            Some((left, right)) => (left.trim(), first_field(right)),
            None if line.starts_with('/') => (first_field(line), first_field(line)),
            None => continue,
        };
        if source.is_empty() {
            continue;
        }
        let name = Path::new(requested)
            .file_name()
            .and_then(OsStr::to_str)
            .filter(|n| plain_name(n))
            .ok_or_else(|| format!("unexpected line in ldd output: {line}"))?;
        let lib = Lib { name: name.to_string(), source: PathBuf::from(source) };
        if requested.starts_with('/') {
            if interpreter.is_none() {
                interpreter = Some(lib);
            }
        } else {
            libraries.push(lib);
        }
    }
    interpreter
        .map(|interpreter| Linkage::Dynamic { interpreter, libraries })
        .ok_or_else(|| format!("ldd names no program interpreter for the genie binary: {}", text.trim()))
}

/// The part of an ldd entry before its load address.
fn first_field(s: &str) -> &str {
    s.split(" (").next().unwrap_or_default().trim()
}

/// A file name that can be written into a shell script and a bundle without quoting.
fn plain_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'+'))
}

/// The shim of a dynamic binary: the bundled loader runs the bundled binary against the bundled libraries.
fn shim_text(loader: &str) -> String {
    format!("#!/bin/sh\nexec {GENIE}/{LIB_DIR}/{loader} --library-path {GENIE}/{LIB_DIR} {GENIE}/{EXE_NAME} \"$@\"\n")
}

/// The shim of a static binary, which runs on its own.
fn static_shim_text() -> String {
    format!("#!/bin/sh\nexec {GENIE}/{EXE_NAME} \"$@\"\n")
}

/// [`harness_mounts`] with the PATH given, so tests need not change the process environment.
fn mounts_for(program: &Path, path_var: &OsStr, data: &Path, project: &str, extra_rw: &[PathBuf]) -> Vec<Mount> {
    let hops = program_hops(program, path_var);
    let scripts = runtime_scripts(data);
    let wanted = wanted_mounts(&hops, &scripts, data, project, extra_rw);
    select_mounts(&wanted, data, |path| path.exists())
}

/// The file `program` names: a bare name is searched for along `path_var`, anything else is taken as given.
fn resolve_program(program: &Path, path_var: &OsStr) -> Option<PathBuf> {
    let found = if program.components().count() == 1 && !program.is_absolute() {
        std::env::split_paths(path_var).map(|dir| dir.join(program)).find(|candidate| candidate.is_file())?
    } else {
        program.to_path_buf()
    };
    std::path::absolute(found).ok().filter(|path| path.is_file())
}

/// The harness program's path as found, each symlink it passes through, and the file it ends at.
/// Empty when the program is not found.
fn program_hops(program: &Path, path_var: &OsStr) -> Vec<PathBuf> {
    let Some(found) = resolve_program(program, path_var) else {
        return Vec::new();
    };
    let mut hops = vec![found.clone()];
    let mut current = found.clone();
    for _ in 0..MAX_LINKS {
        let Ok(target) = fs::read_link(&current) else {
            break;
        };
        let next = lexical(current.parent().unwrap_or(Path::new("/")).join(target));
        hops.push(next.clone());
        current = next;
    }
    if let Ok(real) = fs::canonicalize(&found)
        && !hops.contains(&real)
    {
        hops.push(real);
    }
    hops
}

/// `path` with `.` and `..` resolved as text. Used only for link targets, which are then checked
/// against the file system.
fn lexical(path: PathBuf) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// The `genie-*.ts` files of `<data>/runtime`: the TypeScript extensions the agent loads.
fn runtime_scripts(data: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(data.join("runtime")) else {
        return Vec::new();
    };
    let mut scripts: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file() && path.file_name().and_then(OsStr::to_str).is_some_and(|n| n.starts_with("genie-") && n.ends_with(".ts"))
        })
        .collect();
    scripts.sort();
    scripts
}

/// Every mount the harness wants, before filtering. Nothing here touches the file system.
fn wanted_mounts(hops: &[PathBuf], scripts: &[PathBuf], data: &Path, project: &str, extra_rw: &[PathBuf]) -> Vec<(PathBuf, MountMode)> {
    let mut wanted = Vec::new();
    for hop in hops {
        wanted.push((hop.clone(), MountMode::ReadOnly));
        if let Some(dir) = hop.parent() {
            wanted.push((dir.to_path_buf(), MountMode::ReadOnly));
        }
    }
    wanted.extend(scripts.iter().map(|script| (script.clone(), MountMode::ReadOnly)));
    wanted.push((data.join("skills"), MountMode::ReadOnly));
    // A project name that is not one path component would reach outside its directory.
    if is_name(project) {
        wanted.push((data.join("runtime").join(project), MountMode::ReadOnly));
        wanted.push((data.join("sessions").join(project), MountMode::ReadWrite));
    }
    wanted.extend(extra_rw.iter().map(|path| (path.clone(), MountMode::ReadWrite)));
    wanted
}

/// Whether `name` is exactly one normal path component.
fn is_name(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    matches!((parts.next(), parts.next()), (Some(Component::Normal(_)), None))
}

/// The mounts to make from `wanted`, in path order. Dropped: relative or non-UTF-8 paths, paths that
/// `exists` rejects, and `/` or any ancestor of `data`, where the server keeps its databases and
/// workspaces. A path wanted both ways is mounted read-write.
fn select_mounts(wanted: &[(PathBuf, MountMode)], data: &Path, exists: impl Fn(&Path) -> bool) -> Vec<Mount> {
    let mut chosen: BTreeMap<PathBuf, MountMode> = BTreeMap::new();
    for (path, mode) in wanted {
        if !path.is_absolute() || path.to_str().is_none() || data.starts_with(path) || !exists(path) {
            continue;
        }
        let slot = chosen.entry(path.clone()).or_insert(MountMode::ReadOnly);
        if *mode == MountMode::ReadWrite {
            *slot = MountMode::ReadWrite;
        }
    }
    chosen.into_iter().map(|(path, mode)| Mount { target: path.to_string_lossy().into_owned(), source: path, mode }).collect()
}

/// A unique-enough suffix for a staging directory.
fn nanos() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos())
}

fn write_file(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, bytes)
        .and_then(|()| fs::set_permissions(path, fs::Permissions::from_mode(mode)))
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    /// A temporary directory that can be removed even after `bundle` made it read-only.
    struct Scratch(tempfile::TempDir);

    impl Scratch {
        fn new() -> Scratch {
            Scratch(tempfile::tempdir().expect("a temporary directory"))
        }

        /// The directory with symlinks resolved, so expected paths compare equal.
        fn path(&self) -> PathBuf {
            fs::canonicalize(self.0.path()).expect("the temporary directory")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = store::make_writable(self.0.path());
        }
    }

    fn have_ldd() -> bool {
        Command::new("ldd").arg("--version").output().is_ok()
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).expect("metadata").permissions().mode() & 0o777
    }

    fn lib(name: &str, source: &str) -> Lib {
        Lib { name: name.into(), source: source.into() }
    }

    const GLIBC_X86_64: &str = "\tlinux-vdso.so.1 (0x00007ffd9a7f9000)
\tlibgcc_s.so.1 => /lib/x86_64-linux-gnu/libgcc_s.so.1 (0x00007f0e5c8a0000)
\tlibc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x00007f0e5c400000)
\t/lib64/ld-linux-x86-64.so.2 (0x00007f0e5c8f0000)
";

    /// An older glibc prints the vDSO with an empty path.
    const GLIBC_OLD_VDSO: &str = "\tlinux-vdso.so.1 =>  (0x00007ffc5a5fd000)
\tlibc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x00007f1f8e400000)
\t/lib64/ld-linux-x86-64.so.2 (0x00007f1f8e6f0000)
";

    const GLIBC_AARCH64: &str = "\tlinux-vdso.so.1 (0x0000ffff8a5f0000)
\tlibc.so.6 => /lib/aarch64-linux-gnu/libc.so.6 (0x0000ffff8a400000)
\t/lib/ld-linux-aarch64.so.1 (0x0000ffff8a5e0000)
";

    /// The shape of a recent glibc (2.44 here): the loader is a link with a target.
    const GLIBC_LOADER_LINK: &str = "\tlinux-vdso.so.1 (0x00007f1720862000)
\tlibgcc_s.so.1 => /usr/lib/libgcc_s.so.1 (0x00007f17207fd000)
\tlibm.so.6 => /usr/lib/libm.so.6 (0x00007f171dcc9000)
\tlibc.so.6 => /usr/lib/libc.so.6 (0x00007f171da00000)
\t/lib64/ld-linux-x86-64.so.2 => /usr/lib64/ld-linux-x86-64.so.2 (0x00007f1720864000)
";

    const MUSL_X86_64: &str = "\t/lib/ld-musl-x86_64.so.1 (0x7f1c2b5d4000)
\tlibc.musl-x86_64.so.1 => /lib/ld-musl-x86_64.so.1 (0x7f1c2b5d4000)
";

    #[test]
    fn ldd_of_a_glibc_x86_64_binary_names_the_loader_and_its_libraries() {
        let expected = Linkage::Dynamic {
            interpreter: lib("ld-linux-x86-64.so.2", "/lib64/ld-linux-x86-64.so.2"),
            libraries: vec![
                lib("libgcc_s.so.1", "/lib/x86_64-linux-gnu/libgcc_s.so.1"),
                lib("libc.so.6", "/lib/x86_64-linux-gnu/libc.so.6"),
            ],
        };
        assert_eq!(parse_ldd(GLIBC_X86_64).unwrap(), expected);
    }

    #[test]
    fn ldd_with_an_empty_vdso_path_and_an_aarch64_loader() {
        assert_eq!(
            parse_ldd(GLIBC_OLD_VDSO).unwrap(),
            Linkage::Dynamic {
                interpreter: lib("ld-linux-x86-64.so.2", "/lib64/ld-linux-x86-64.so.2"),
                libraries: vec![lib("libc.so.6", "/lib/x86_64-linux-gnu/libc.so.6")],
            }
        );
        assert_eq!(
            parse_ldd(GLIBC_AARCH64).unwrap(),
            Linkage::Dynamic {
                interpreter: lib("ld-linux-aarch64.so.1", "/lib/ld-linux-aarch64.so.1"),
                libraries: vec![lib("libc.so.6", "/lib/aarch64-linux-gnu/libc.so.6")],
            }
        );
    }

    #[test]
    fn a_loader_shown_as_a_link_is_copied_from_its_target_under_its_own_name() {
        assert_eq!(
            parse_ldd(GLIBC_LOADER_LINK).unwrap(),
            Linkage::Dynamic {
                interpreter: lib("ld-linux-x86-64.so.2", "/usr/lib64/ld-linux-x86-64.so.2"),
                libraries: vec![
                    lib("libgcc_s.so.1", "/usr/lib/libgcc_s.so.1"),
                    lib("libm.so.6", "/usr/lib/libm.so.6"),
                    lib("libc.so.6", "/usr/lib/libc.so.6"),
                ],
            }
        );
    }

    #[test]
    fn ldd_of_a_musl_binary_names_its_loader() {
        assert_eq!(
            parse_ldd(MUSL_X86_64).unwrap(),
            Linkage::Dynamic {
                interpreter: lib("ld-musl-x86_64.so.1", "/lib/ld-musl-x86_64.so.1"),
                libraries: vec![lib("libc.musl-x86_64.so.1", "/lib/ld-musl-x86_64.so.1")],
            }
        );
    }

    #[test]
    fn ldd_of_a_binary_that_is_not_dynamic_is_static() {
        for text in ["\tnot a dynamic executable\n", "\tstatically linked\n", "ldd: /tmp/genie: Not a valid dynamic program\n"] {
            assert_eq!(parse_ldd(text).unwrap(), Linkage::Static, "{text:?}");
        }
    }

    #[test]
    fn ldd_that_finds_a_missing_library_is_an_error_naming_it() {
        let err = parse_ldd("\tlibfoo.so.2 => not found\n\t/lib64/ld-linux-x86-64.so.2 (0x1)\n").unwrap_err();
        assert!(err.contains("libfoo.so.2 => not found"), "{err}");
    }

    #[test]
    fn ldd_without_an_interpreter_is_an_error() {
        let err = parse_ldd("\tlinux-vdso.so.1 (0x1)\n\tlibc.so.6 => /usr/lib/libc.so.6 (0x2)\n").unwrap_err();
        assert!(err.contains("no program interpreter"), "{err}");
    }

    #[test]
    fn the_shims_run_the_bundled_loader_or_the_binary_itself() {
        assert_eq!(
            shim_text("ld-linux-x86-64.so.2"),
            "#!/bin/sh\nexec /opt/genie/lib/ld-linux-x86-64.so.2 --library-path /opt/genie/lib /opt/genie/genie \"$@\"\n"
        );
        assert_eq!(static_shim_text(), "#!/bin/sh\nexec /opt/genie/genie \"$@\"\n");
    }

    #[test]
    fn a_static_binary_is_bundled_with_a_direct_shim_and_is_found_again() {
        if !have_ldd() {
            eprintln!("skipped: ldd is not installed");
            return;
        }
        let scratch = Scratch::new();
        let base = scratch.path();
        let script = b"#!/bin/sh\necho genie\n";
        let exe = base.join("genie-stand-in");
        fs::write(&exe, script).unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        let env_root = base.join("env");

        let made = bundle(&env_root, &exe).unwrap();

        let name = format!("genie-{}", &sha256_hex(script)[..16]);
        assert_eq!(made.dir, env_root.join("harness").join(&name));
        assert_eq!((made.mount_target.as_str(), made.path_prefix.as_str()), ("/opt/genie", "/opt/genie/bin"));
        assert_eq!(fs::read(made.dir.join("genie")).unwrap(), script);
        assert_eq!(fs::read_to_string(made.dir.join("bin/genie")).unwrap(), static_shim_text());
        assert_eq!(mode(&made.dir.join("bin/genie")) & 0o111, 0o111, "the shim is executable");
        assert_eq!(mode(&made.dir.join("genie")) & 0o222, 0, "the bundle is read-only");
        assert_eq!(bundle(&env_root, &exe).unwrap(), made, "a second call finds the same bundle");
        let entries: Vec<_> = fs::read_dir(env_root.join("harness")).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(entries, vec![std::ffi::OsString::from(name)], "no staging directory is left behind");
    }

    #[test]
    fn prune_removes_other_bundles_and_keeps_the_current_one() {
        let scratch = Scratch::new();
        let env_root = scratch.path().join("env");
        let harness = env_root.join("harness");
        for dir in ["genie-aaaaaaaaaaaaaaaa", "genie-bbbbbbbbbbbbbbbb", "other"] {
            fs::create_dir_all(harness.join(dir)).unwrap();
        }
        let keep = Bundle { dir: harness.join("genie-bbbbbbbbbbbbbbbb"), mount_target: GENIE.into(), path_prefix: format!("{GENIE}/bin") };

        let removed = prune_bundles(&env_root, &keep).unwrap();

        assert_eq!(removed, vec![harness.join("genie-aaaaaaaaaaaaaaaa")]);
        assert!(harness.join("genie-bbbbbbbbbbbbbbbb").is_dir());
        assert!(harness.join("other").is_dir(), "directories that are not bundles are kept");
        assert!(!harness.join("genie-aaaaaaaaaaaaaaaa").exists());
    }

    #[test]
    fn prune_without_a_harness_directory_removes_nothing() {
        let scratch = Scratch::new();
        let keep = Bundle { dir: scratch.path().join("x"), mount_target: GENIE.into(), path_prefix: String::new() };
        assert_eq!(prune_bundles(&scratch.path().join("absent"), &keep).unwrap(), Vec::<PathBuf>::new());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_bundle_of_this_test_binary_runs_under_its_loader() {
        if !have_ldd() {
            eprintln!("skipped: ldd is not installed");
            return;
        }
        let scratch = Scratch::new();
        let exe = std::env::current_exe().unwrap();
        let made = bundle(&scratch.path().join("env"), &exe).unwrap();

        let lib_dir = made.dir.join("lib");
        let loader = fs::read_dir(&lib_dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("ld-")))
            .expect("the bundle holds a loader");
        let loader_name = loader.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(fs::read_to_string(made.dir.join("bin/genie")).unwrap(), shim_text(&loader_name), "the shim names the bundled loader");

        // The equivalent of the shim's command, with libtest's `--list` in place of `--help`.
        let out = Command::new(&loader)
            .arg("--library-path")
            .arg(&lib_dir)
            .arg(made.dir.join("genie"))
            .arg("--list")
            .output()
            .expect("the loader starts");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(String::from_utf8_lossy(&out.stdout).contains("env::harness::tests::"), "the test list is printed");
    }

    /// The data directory layout the harness reads, with a program that is reached through a
    /// relative symlink.
    fn data_fixture(root: &Path) -> (PathBuf, PathBuf) {
        let data = root.join("data");
        for dir in ["runtime/proj", "skills", "sessions/proj", "pi-agent"] {
            fs::create_dir_all(data.join(dir)).unwrap();
        }
        for file in ["runtime/genie-a.ts", "runtime/genie-b.ts", "runtime/other.ts"] {
            fs::write(data.join(file), "").unwrap();
        }
        fs::create_dir_all(root.join("tools/bin")).unwrap();
        fs::write(root.join("tools/bin/pi-real"), "").unwrap();
        symlink("bin/pi-real", root.join("tools/pi")).unwrap();
        (data, root.join("tools/pi"))
    }

    #[test]
    fn the_program_its_links_and_its_data_are_mounted_at_their_host_paths() {
        let scratch = Scratch::new();
        let root = scratch.path();
        let (data, program) = data_fixture(&root);
        let pi_dir = data.join("pi-agent");

        let mounts = mounts_for(&program, OsStr::new(""), &data, "proj", std::slice::from_ref(&pi_dir));

        let got: BTreeMap<PathBuf, MountMode> = mounts.iter().map(|m| (m.source.clone(), m.mode.clone())).collect();
        let ro = MountMode::ReadOnly;
        let rw = MountMode::ReadWrite;
        let expected: BTreeMap<PathBuf, MountMode> = [
            (program.clone(), ro.clone()),
            (root.join("tools"), ro.clone()),
            (root.join("tools/bin/pi-real"), ro.clone()),
            (root.join("tools/bin"), ro.clone()),
            (data.join("runtime/genie-a.ts"), ro.clone()),
            (data.join("runtime/genie-b.ts"), ro.clone()),
            (data.join("runtime/proj"), ro.clone()),
            (data.join("skills"), ro),
            (data.join("sessions/proj"), rw.clone()),
            (pi_dir, rw),
        ]
        .into_iter()
        .collect();
        assert_eq!(got, expected);
        assert!(mounts.iter().all(|m| m.target == m.source.to_string_lossy()), "identity mounts");
    }

    #[test]
    fn a_program_that_is_not_found_mounts_only_the_data() {
        let scratch = Scratch::new();
        let root = scratch.path();
        let (data, _) = data_fixture(&root);

        let mounts = mounts_for(Path::new("no-such-harness"), OsStr::new("/nonexistent"), &data, "proj", &[]);

        assert!(mounts.iter().all(|m| m.source.starts_with(&data)), "{mounts:?}");
        assert!(mounts.iter().any(|m| m.source == data.join("skills")));
    }

    #[test]
    fn a_bare_program_name_is_found_along_the_path() {
        let scratch = Scratch::new();
        let root = scratch.path();
        data_fixture(&root);
        let path_var = std::env::join_paths([PathBuf::from("/nonexistent"), root.join("tools/bin")]).unwrap();

        assert_eq!(resolve_program(Path::new("pi-real"), &path_var), Some(root.join("tools/bin/pi-real")));
        assert_eq!(resolve_program(Path::new("absent"), &path_var), None);
    }

    #[test]
    fn the_root_and_the_ancestors_of_data_are_never_mounted() {
        let wanted = [
            (PathBuf::from("/"), MountMode::ReadOnly),
            (PathBuf::from("/home"), MountMode::ReadOnly),
            (PathBuf::from("/home/u/data"), MountMode::ReadOnly),
            (PathBuf::from("/home/u/data/x"), MountMode::ReadOnly),
        ];

        let mounts = select_mounts(&wanted, Path::new("/home/u/data"), |_| true);

        let sources: Vec<PathBuf> = mounts.into_iter().map(|m| m.source).collect();
        assert_eq!(sources, vec![PathBuf::from("/home/u/data/x")]);
    }

    #[test]
    fn a_path_wanted_twice_is_mounted_once_and_read_write_wins() {
        let wanted = [
            (PathBuf::from("/a/x"), MountMode::ReadOnly),
            (PathBuf::from("/a/x"), MountMode::ReadWrite),
            (PathBuf::from("/a/x"), MountMode::ReadOnly),
            (PathBuf::from("/a/y"), MountMode::ReadOnly),
        ];

        let mounts = select_mounts(&wanted, Path::new("/data"), |_| true);

        let got: Vec<(PathBuf, MountMode)> = mounts.into_iter().map(|m| (m.source, m.mode)).collect();
        assert_eq!(got, vec![(PathBuf::from("/a/x"), MountMode::ReadWrite), (PathBuf::from("/a/y"), MountMode::ReadOnly)]);
    }

    #[test]
    fn relative_and_missing_paths_are_dropped() {
        let wanted = [
            (PathBuf::from("relative/x"), MountMode::ReadOnly),
            (PathBuf::from("/present"), MountMode::ReadOnly),
            (PathBuf::from("/missing"), MountMode::ReadOnly),
        ];

        let mounts = select_mounts(&wanted, Path::new("/data"), |p| p != Path::new("/missing"));

        assert_eq!(mounts.iter().map(|m| m.source.clone()).collect::<Vec<_>>(), vec![PathBuf::from("/present")]);
    }

    #[cfg(unix)]
    #[test]
    fn a_non_utf8_path_cannot_be_a_mount_and_is_dropped() {
        use std::os::unix::ffi::OsStrExt;
        let bad = PathBuf::from(OsStr::from_bytes(b"/tmp/\xff"));
        let mounts = select_mounts(&[(bad, MountMode::ReadOnly)], Path::new("/data"), |_| true);
        assert!(mounts.is_empty());
    }

    #[test]
    fn a_project_name_that_is_not_one_component_mounts_no_project_directories() {
        let data = Path::new("/srv/data");
        for project in ["", ".", "..", "../other", "a/b"] {
            let wanted = wanted_mounts(&[], &[], data, project, &[]);
            assert_eq!(wanted, vec![(data.join("skills"), MountMode::ReadOnly)], "project {project:?}");
        }
    }

    #[test]
    fn the_genie_url_points_at_the_host_and_the_loopback_proxy_is_mapped_to_it() {
        let mut env = vec![("GENIE_URL".to_string(), "http://127.0.0.1:7000".to_string()), ("GENIE_TOKEN".to_string(), "t".to_string())];

        rewrite_env(&mut env, "10.0.2.2", 7000).unwrap();

        assert_eq!(get(&env, "GENIE_URL"), Some("http://10.0.2.2:7000"));
        assert_eq!(get(&env, "GENIE_TOKEN"), Some("t"), "other variables stay");
        assert_eq!(get(&env, "GIT_CONFIG_COUNT"), Some("2"));
        assert_eq!(get(&env, "GIT_CONFIG_KEY_0"), Some("url.http://10.0.2.2:7000/.insteadOf"));
        assert_eq!(get(&env, "GIT_CONFIG_VALUE_0"), Some("http://127.0.0.1:7000/"));
        assert_eq!(get(&env, "GIT_CONFIG_KEY_1"), Some("url.http://10.0.2.2:7000/.insteadOf"));
        assert_eq!(get(&env, "GIT_CONFIG_VALUE_1"), Some("http://localhost:7000/"));
    }

    #[test]
    fn the_git_mapping_is_appended_after_an_existing_count() {
        let mut env = vec![
            ("GIT_CONFIG_COUNT".to_string(), "1".to_string()),
            ("GIT_CONFIG_KEY_0".to_string(), "http.extraheader".to_string()),
            ("GIT_CONFIG_VALUE_0".to_string(), "X: y".to_string()),
        ];

        rewrite_env(&mut env, "host.internal", 7001).unwrap();

        assert_eq!(get(&env, "GIT_CONFIG_COUNT"), Some("3"));
        assert_eq!(get(&env, "GIT_CONFIG_KEY_0"), Some("http.extraheader"), "the existing entry stays");
        assert_eq!(get(&env, "GIT_CONFIG_VALUE_0"), Some("X: y"));
        assert_eq!(get(&env, "GIT_CONFIG_KEY_1"), Some("url.http://host.internal:7001/.insteadOf"));
        assert_eq!(get(&env, "GIT_CONFIG_VALUE_2"), Some("http://localhost:7001/"));
        assert_eq!(env.iter().filter(|(k, _)| k == "GIT_CONFIG_COUNT").count(), 1);
    }

    #[test]
    fn a_malformed_git_count_or_an_empty_host_leaves_the_environment_alone() {
        let mut env = vec![("GIT_CONFIG_COUNT".to_string(), "many".to_string())];
        let before = env.clone();
        assert!(rewrite_env(&mut env, "10.0.2.2", 7000).is_err());
        assert_eq!(env, before);

        let mut env = Vec::new();
        assert!(rewrite_env(&mut env, " ", 7000).unwrap_err().contains("hostAddress"));
        assert!(env.is_empty());
    }

    fn get<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
        env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn the_probe_runs_the_program_with_version_and_a_failure_names_the_base() {
        assert_eq!(probe_argv(Path::new("/usr/bin/pi")), vec!["/usr/bin/pi".to_string(), "--version".to_string()]);
        assert_eq!(check_probe(0, "pi 1.2.3\n"), Ok(()));
        let err = check_probe(127, "sh: pi: not found\n\n").unwrap_err();
        assert_eq!(err, "the agent harness cannot run in this environment's base: exit status 127: sh: pi: not found");
        assert_eq!(harness_cannot_run("x"), "the agent harness cannot run in this environment's base: x");
    }
}
