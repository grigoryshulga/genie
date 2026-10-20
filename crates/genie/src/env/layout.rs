//! Paths inside a workspace. A runner with canonical paths gives every workspace the same
//! `/workspace` and `/home/genie`; one that runs on the host sees the real checkout and a home
//! directory of its own, so its paths are the host's.

use std::path::Path;

/// The workspace root of a runner with canonical paths. A repository mounted at `.` is here,
/// one mounted at `api` is `/workspace/api`.
pub const WORKSPACE: &str = "/workspace";
/// `$HOME` of a runner with canonical paths. `~/` in recipe paths expands to it.
pub const HOME: &str = "/home/genie";
/// Harness files: the bin shims and the genie binary bundle.
pub const GENIE: &str = "/opt/genie";

/// The root and home of one workspace as its processes see them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    root: String,
    home: String,
}

impl Layout {
    /// The layout of a workspace on a runner. With `canonical` the constants above apply and the
    /// real paths are unused. Otherwise the root is the real workspace directory and the home is
    /// the workspace's own home directory.
    pub fn for_workspace(canonical: bool, real_root: &Path, ws_home: &Path) -> Result<Layout, String> {
        if canonical {
            return Ok(Layout { root: WORKSPACE.into(), home: HOME.into() });
        }
        Ok(Layout { root: utf8(real_root)?, home: utf8(ws_home)? })
    }

    /// The workspace root.
    pub fn root(&self) -> &str {
        &self.root
    }

    /// Where the repository mounted at `mount` sits: `.` is the root, anything else is below it.
    pub fn repo(&self, mount: &str) -> String {
        match mount {
            "" | "." => self.root.clone(),
            m => format!("{}/{}", self.root, m.trim_matches('/')),
        }
    }

    /// `$HOME` of the workspace.
    pub fn home(&self) -> &str {
        &self.home
    }

    /// A path from a recipe or profile as the workspace sees it: `~` and `~/...` are the home
    /// directory, anything else is returned unchanged.
    pub fn expand(&self, path: &str) -> String {
        match path.strip_prefix("~/") {
            Some(rest) => format!("{}/{}", self.home, rest),
            None if path == "~" => self.home.clone(),
            None => path.to_string(),
        }
    }
}

/// A host path as text. A path that is not UTF-8 cannot be written into a mount or a command, so
/// it is refused with its name rather than mangled.
fn utf8(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(|s| s.trim_end_matches('/').to_string())
        .ok_or_else(|| format!("{}: the path is not UTF-8, which a host workspace cannot use; move the environment store", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_canonical_workspace_sees_the_fixed_paths_whatever_the_host_paths_are() {
        let l = Layout::for_workspace(true, Path::new("/srv/checkout"), Path::new("/data/home")).unwrap();
        assert_eq!((l.root(), l.home()), ("/workspace", "/home/genie"));
        assert_eq!(l.repo("."), "/workspace");
        assert_eq!(l.repo("api"), "/workspace/api");
        assert_eq!(l.expand("~/.cargo/registry"), "/home/genie/.cargo/registry");
    }

    #[test]
    fn a_host_workspace_uses_its_real_root_and_its_own_home() {
        let l = Layout::for_workspace(false, Path::new("/home/me/ws/"), Path::new("/data/env/workspaces/shop--api/home")).unwrap();
        assert_eq!(l.root(), "/home/me/ws", "a trailing slash is not part of the path");
        assert_eq!(l.home(), "/data/env/workspaces/shop--api/home");
        assert_eq!(l.repo("web"), "/home/me/ws/web");
        assert_eq!(l.repo("web/"), "/home/me/ws/web");
        assert_eq!(l.expand("~/.npm"), "/data/env/workspaces/shop--api/home/.npm");
    }

    #[test]
    fn a_path_is_expanded_only_at_the_home_prefix() {
        let l = Layout::for_workspace(true, Path::new("/x"), Path::new("/x")).unwrap();
        assert_eq!(l.expand("~"), "/home/genie");
        assert_eq!(l.expand("/abs/~/x"), "/abs/~/x", "a tilde inside a path is not home");
        assert_eq!(l.expand("~other/x"), "~other/x", "another user's home is not expanded");
        assert_eq!(l.expand("node_modules"), "node_modules", "relative paths stay relative");
    }

    #[test]
    fn the_repository_at_the_root_and_nested_mounts_are_distinct() {
        let l = Layout::for_workspace(true, Path::new("/x"), Path::new("/x")).unwrap();
        assert_eq!(l.repo(""), "/workspace");
        assert_eq!(l.repo("apps/web"), "/workspace/apps/web");
    }

    #[cfg(unix)]
    #[test]
    fn a_host_path_that_is_not_utf8_is_refused_by_name() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let bad = PathBuf::from(OsStr::from_bytes(b"/tmp/\xff"));
        let err = Layout::for_workspace(false, &bad, Path::new("/home/x")).unwrap_err();
        assert!(err.contains("not UTF-8"), "{err}");
        assert!(Layout::for_workspace(true, &bad, &bad).is_ok(), "a canonical workspace never reads the host path");
    }
}
