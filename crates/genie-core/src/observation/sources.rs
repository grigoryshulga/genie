//! Observation sources of a project (`observation_sources` of the server database, §13.2):
//! the server-wide configuration of each Prometheus- or Sentry-compatible source, and the rules
//! every source must pass (§7.3). Their secrets are sealed in `secrets`, never in these rows.

use rusqlite::{Row, params};
use serde::{Deserialize, Serialize};

use crate::db::now;
use crate::error::{GenieError, Result};
use crate::observation::{SourceAuth, SourceConfig, SourceKind};
use crate::server_db::ServerDb;

/// Executed by `server_db::open_migrated` after the secrets schema (§13.2).
pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS observation_sources (
  project TEXT NOT NULL REFERENCES projects(slug) ON DELETE CASCADE,
  name TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('prometheus', 'sentry')),
  url TEXT NOT NULL,
  config TEXT NOT NULL DEFAULT '{}',
  created TEXT NOT NULL,
  updated TEXT NOT NULL,
  PRIMARY KEY (project, name)
);
"#;

/// A source as the administration enters it (§15 `SourceBody`): every field of `SourceConfig`
/// except the timestamps. Blank optional text counts as absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewSource {
    pub name: String,
    pub kind: SourceKind,
    pub url: String,
    #[serde(default)]
    pub auth: SourceAuth,
    pub organization: Option<String>,
    pub sentry_project: Option<String>,
    pub environment: Option<String>,
    pub field: Option<String>,
    pub dataset: Option<String>,
    pub timeout_secs: Option<u64>,
}

/// The fields of `SourceConfig` the `config` column holds next to the table's own columns (§13.2).
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredConfig {
    #[serde(default)]
    auth: SourceAuth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    organization: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sentry_project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    environment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dataset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_secs: Option<u64>,
}

/// A row of `observation_sources` before its `config` is decoded.
struct RawSource {
    name: String,
    kind: SourceKind,
    url: String,
    config: String,
    created: String,
    updated: String,
}

impl RawSource {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<RawSource> {
        Ok(RawSource { name: r.get(0)?, kind: r.get(1)?, url: r.get(2)?, config: r.get(3)?, created: r.get(4)?, updated: r.get(5)? })
    }

    fn into_source(self) -> Result<SourceConfig> {
        let config: StoredConfig = serde_json::from_str(&self.config)?;
        Ok(SourceConfig {
            name: self.name,
            kind: self.kind,
            url: self.url,
            auth: config.auth,
            organization: config.organization,
            sentry_project: config.sentry_project,
            environment: config.environment,
            field: config.field,
            dataset: config.dataset,
            timeout_secs: config.timeout_secs,
            created: self.created,
            updated: self.updated,
        })
    }
}

const SOURCE_COLUMNS: &str = "SELECT name, kind, url, config, created, updated FROM observation_sources";

/// The error for a source that is not in the project.
pub(crate) fn not_found(name: &str) -> GenieError {
    GenieError::not_found(format!("source {name} not found"))
}

fn config_json(source: &SourceConfig) -> Result<String> {
    let config = StoredConfig {
        auth: source.auth,
        organization: source.organization.clone(),
        sentry_project: source.sentry_project.clone(),
        environment: source.environment.clone(),
        field: source.field.clone(),
        dataset: source.dataset.clone(),
        timeout_secs: source.timeout_secs,
    };
    Ok(serde_json::to_string(&config)?)
}

/// Trimmed text, or `None` when nothing is left of it.
fn text(value: &Option<String>) -> Option<String> {
    value.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// The URL as stored (§13.2): trimmed, scheme in lower case, trailing slashes removed. Input
/// without `://` is kept as typed, so the scheme rule rejects it.
fn canonical_url(raw: &str) -> String {
    let url = raw.trim();
    match url.split_once("://") {
        Some((scheme, rest)) => format!("{}://{}", scheme.to_ascii_lowercase(), rest.trim_end_matches('/')),
        None => url.to_string(),
    }
}

/// `^[a-z0-9][a-z0-9_-]{0,31}$`
fn valid_source_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        && name.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// The URL rules of §13.2, for a URL already canonical.
fn url_problems(url: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let Some((scheme, rest)) = url.split_once("://") else {
        problems.push("url must start with http:// or https://".into());
        return problems;
    };
    if !matches!(scheme, "http" | "https") {
        problems.push("url must start with http:// or https://".into());
    }
    if url.chars().any(char::is_whitespace) {
        problems.push("url must not contain spaces".into());
    }
    if rest.contains('?') {
        problems.push("url must not contain a query string; the query belongs to each criterion".into());
    }
    if rest.contains('#') {
        problems.push("url must not contain a fragment".into());
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        problems.push("url must not contain user info; keep credentials in the source's secret".into());
    }
    // The host is what precedes an optional port: `https://:9090` has no host.
    let host = authority.split(':').next().unwrap_or_default();
    if host.is_empty() {
        problems.push("url needs a host, for example https://prometheus.example.com".into());
    }
    problems
}

impl SourceConfig {
    /// Every rule of §13.2 the stored form breaks, each naming the field. Empty when the source is valid.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.name == "manual" {
            problems.push("source name manual is reserved for manual evidence; choose another name".into());
        } else if !valid_source_name(&self.name) {
            problems.push(
                "source name must be 1 to 32 lowercase letters, digits, dashes or underscores, starting with a letter or digit".into(),
            );
        }
        problems.extend(url_problems(&self.url));
        if self.kind == SourceKind::Sentry && self.organization.is_none() {
            problems.push("organization is required for a sentry source: the Sentry organization slug".into());
        }
        if self.timeout_secs.is_some_and(|t| !(1..=120).contains(&t)) {
            problems.push("timeoutSecs must be between 1 and 120".into());
        }
        problems
    }
}

impl NewSource {
    /// The stored form at `at`, normalized but not yet checked.
    fn stored(&self, at: &str) -> SourceConfig {
        SourceConfig {
            name: self.name.trim().to_string(),
            kind: self.kind,
            url: canonical_url(&self.url),
            auth: self.auth,
            organization: text(&self.organization),
            sentry_project: text(&self.sentry_project),
            environment: text(&self.environment),
            field: text(&self.field),
            dataset: text(&self.dataset),
            timeout_secs: self.timeout_secs,
            created: at.to_string(),
            updated: at.to_string(),
        }
    }

    /// The stored form, or every problem joined into one invalid-input message.
    fn checked(&self, at: &str) -> Result<SourceConfig> {
        let source = self.stored(at);
        let problems = source.problems();
        if problems.is_empty() { Ok(source) } else { Err(GenieError::invalid(problems.join("; "))) }
    }
}

impl ServerDb {
    /// Add a source to a project. The name must be unused in the project and the configuration
    /// must pass `SourceConfig::problems`.
    pub fn add_observation_source(&self, project: &str, new: NewSource) -> Result<SourceConfig> {
        self.project(project)?;
        let source = new.checked(&now())?;
        self.tx(|| {
            if self.observation_source(project, &source.name)?.is_some() {
                return Err(GenieError::invalid(format!("source {} already exists", source.name)));
            }
            self.conn().execute(
                "INSERT INTO observation_sources(project, name, kind, url, config, created, updated) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![project, source.name, source.kind, source.url, config_json(&source)?, source.created, source.updated],
            )?;
            Ok(())
        })?;
        Ok(source)
    }

    /// Replace the configuration of a source. The name cannot change: the source's secret is
    /// sealed under its name, so a renamed source would lose it. The creation time is kept.
    pub fn update_observation_source(&self, project: &str, name: &str, new: NewSource) -> Result<SourceConfig> {
        self.tx(|| {
            let current = self.observation_source(project, name)?.ok_or_else(|| not_found(name))?;
            if new.name.trim() != name {
                return Err(GenieError::invalid(format!(
                    "source {name} cannot be renamed; add {} and remove {name} instead",
                    new.name.trim()
                )));
            }
            let mut source = new.checked(&now())?;
            source.created = current.created;
            let changed = self.conn().execute(
                "UPDATE observation_sources SET kind = ?1, url = ?2, config = ?3, updated = ?4 WHERE project = ?5 AND name = ?6",
                params![source.kind, source.url, config_json(&source)?, source.updated, project, name],
            )?;
            if changed == 0 {
                return Err(not_found(name));
            }
            Ok(source)
        })
    }

    /// Remove a source and its secret, in one transaction. Cases that use the source keep
    /// their contract; their collection then fails with an unknown-source error.
    pub fn remove_observation_source(&self, project: &str, name: &str) -> Result<()> {
        self.tx(|| {
            if self.observation_source(project, name)?.is_none() {
                return Err(not_found(name));
            }
            self.conn().execute("DELETE FROM observation_secrets WHERE project = ?1 AND source = ?2", params![project, name])?;
            self.conn().execute("DELETE FROM observation_sources WHERE project = ?1 AND name = ?2", params![project, name])?;
            Ok(())
        })
    }

    /// One source of a project, or `None` when there is none by that name.
    pub fn observation_source(&self, project: &str, name: &str) -> Result<Option<SourceConfig>> {
        let rows = self.load_sources(&format!("{SOURCE_COLUMNS} WHERE project = ?1 AND name = ?2"), params![project, name])?;
        Ok(rows.into_iter().next())
    }

    /// The sources of a project, by name. Secrets are never part of them.
    pub fn observation_sources(&self, project: &str) -> Result<Vec<SourceConfig>> {
        self.load_sources(&format!("{SOURCE_COLUMNS} WHERE project = ?1 ORDER BY name"), params![project])
    }

    fn load_sources(&self, sql: &str, args: impl rusqlite::Params) -> Result<Vec<SourceConfig>> {
        let mut stmt = self.conn().prepare_cached(sql)?;
        let rows = stmt.query_map(args, RawSource::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(RawSource::into_source).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_stored_in_canonical_form() {
        assert_eq!(canonical_url("  HTTPS://Prom.Example.com//  "), "https://Prom.Example.com");
        assert_eq!(canonical_url("http://localhost:9090/prometheus/"), "http://localhost:9090/prometheus");
        assert_eq!(canonical_url("https:///"), "https://");
        assert_eq!(canonical_url("prom.example.com/"), "prom.example.com/");
    }

    #[test]
    fn source_names_follow_the_pattern() {
        assert!(valid_source_name("prom"));
        assert!(valid_source_name("2fast_1-x"));
        assert!(valid_source_name(&"a".repeat(32)));
        assert!(!valid_source_name(&"a".repeat(33)));
        assert!(!valid_source_name(""));
        assert!(!valid_source_name("-prom"));
        assert!(!valid_source_name("_prom"));
        assert!(!valid_source_name("Prom"));
        assert!(!valid_source_name("pro m"));
    }
}
