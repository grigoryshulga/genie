//! The lock of a workspace: every resolved input its environment was built from.
//!
//! Written when preparation starts and amended only by an explicit refresh. It is stored as JSON
//! in `env_workspaces.lock`, so a later version may add fields; readers ignore unknown ones.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The resolved inputs of one workspace.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceLock {
    pub profile: String,
    pub profile_hash: String,
    pub runner: String,
    pub runner_version: String,
    pub base: BaseLock,
    pub repos: Vec<RepoLock>,
    /// `<repository>/<tool>` to the resolved version.
    pub tools: BTreeMap<String, String>,
    /// Qualified step to its snapshot key.
    pub snapshots: BTreeMap<String, String>,
    /// Service name to its image digest.
    pub services: BTreeMap<String, String>,
    pub view_mode: String,
    /// When the lock was written (RFC 3339).
    pub created: String,
}

/// The base environment: the reference it was given and the digest it resolved to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseLock {
    pub reference: String,
    pub digest: String,
}

/// A repository's commit at the moment the workspace was created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoLock {
    pub name: String,
    pub mount: String,
    pub sha: String,
    /// The branch the commit was on; none when it was detached.
    pub branch: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> WorkspaceLock {
        WorkspaceLock {
            profile: "default".into(),
            profile_hash: "ph".into(),
            runner: "podman".into(),
            runner_version: "5.0.0".into(),
            base: BaseLock { reference: "genie-base:0123456789abcdef".into(), digest: "sha256:abc".into() },
            repos: vec![RepoLock { name: "api".into(), mount: ".".into(), sha: "f00d".into(), branch: Some("genie/team-1".into()) }],
            tools: BTreeMap::from([("api/cargo".to_string(), "1.80.0".to_string())]),
            snapshots: BTreeMap::from([("api/fetch".to_string(), "key1".to_string())]),
            services: BTreeMap::from([("db".to_string(), "sha256:db".to_string())]),
            view_mode: "overlay".into(),
            created: "2026-10-17T10:00:00.000Z".into(),
        }
    }

    #[test]
    fn the_lock_is_camel_case_json_that_round_trips() {
        let lock = sample();
        let json = serde_json::to_value(&lock).unwrap();
        assert_eq!(json["profileHash"], "ph");
        assert_eq!(json["runnerVersion"], "5.0.0");
        assert_eq!(json["base"]["reference"], "genie-base:0123456789abcdef");
        assert_eq!(json["repos"][0]["branch"], "genie/team-1");
        assert_eq!(json["viewMode"], "overlay");
        let back: WorkspaceLock = serde_json::from_value(json).unwrap();
        assert_eq!(back, lock);
    }

    #[test]
    fn a_detached_repository_has_no_branch_and_unknown_fields_are_ignored() {
        let text = r#"{"name":"web","mount":"web","sha":"1","branch":null,"futureField":true}"#;
        let repo: RepoLock = serde_json::from_str(text).unwrap();
        assert_eq!(repo.branch, None);
    }
}
