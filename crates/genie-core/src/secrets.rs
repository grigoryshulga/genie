//! Secrets the server keeps for others (`user_secrets`, `repo_secrets`, `mcp_secrets`,
//! `global_secrets`, `observation_secrets` of the server database): a person's LiteLLM
//! key, which agents started on their behalf use, the access token of a project's
//! repository on its git host, the values of the environment variables MCP connections
//! refer to as `${env:NAME}`, the provider keys the server's administrator shares with
//! everyone, and the credentials of a project's observation sources.
//!
//! Unlike sessions and tokens these must be read back, so they are sealed with
//! ChaCha20-Poly1305 under a key kept next to the database (`secrets.key`, 0600)
//! and not copied by `genie backup`: a leaked database or backup does not leak
//! them. Lose the key and the secrets read as unset — people enter them again.

use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::db::now;
use crate::error::{GenieError, Result};
use crate::observation::sources;
use crate::server_db::ServerDb;

/// The LiteLLM key of a person (`LITELLM_API_KEY` of their agents).
pub const LITELLM: &str = "litellm";

pub const SECRETS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS user_secrets (
  user INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  sealed TEXT NOT NULL,
  hint TEXT NOT NULL DEFAULT '',
  updated TEXT NOT NULL,
  PRIMARY KEY (user, name)
);
-- The access token (PAT) of a project's repository on its git host.
CREATE TABLE IF NOT EXISTS repo_secrets (
  project TEXT NOT NULL REFERENCES projects(slug) ON DELETE CASCADE,
  repo TEXT NOT NULL,
  sealed TEXT NOT NULL,
  hint TEXT NOT NULL DEFAULT '',
  updated TEXT NOT NULL,
  PRIMARY KEY (project, repo)
);
-- The value of an environment variable an MCP connection refers to as `${env:NAME}`:
-- set by a server administrator, read by the gateway and checks, never given to agents.
CREATE TABLE IF NOT EXISTS mcp_secrets (
  name TEXT PRIMARY KEY,
  sealed TEXT NOT NULL,
  hint TEXT NOT NULL DEFAULT '',
  updated TEXT NOT NULL
);
-- A provider's key the server's administrator shares with everyone: a person's
-- own key of the same provider comes first, and without either an agent on the
-- provider runs as before.
CREATE TABLE IF NOT EXISTS global_secrets (
  name TEXT PRIMARY KEY,
  sealed TEXT NOT NULL,
  hint TEXT NOT NULL DEFAULT '',
  updated TEXT NOT NULL
);
-- The bearer token or `user:password` of an observation source (§7.3). Sealed under the
-- source's name and project, and removed together with the source.
CREATE TABLE IF NOT EXISTS observation_secrets (
  project TEXT NOT NULL REFERENCES projects(slug) ON DELETE CASCADE,
  source TEXT NOT NULL,
  sealed TEXT NOT NULL,
  hint TEXT NOT NULL DEFAULT '',
  updated TEXT NOT NULL,
  PRIMARY KEY (project, source)
);
"#;

/// A secret on its way in (a request, a new record): never printed, not even by `{:?}`.
#[derive(Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(transparent)]
pub struct Secret(pub String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(…)")
    }
}

/// What may be shown about a secret: never the value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretInfo {
    pub name: String,
    /// The last characters of the value (`…a1b2`).
    pub hint: String,
    pub updated: String,
    /// The value cannot be read back (the server's key changed): enter it again.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unreadable: bool,
}

/// The key file of a database at `db_path`.
pub fn key_path(db_path: &Path) -> PathBuf {
    db_path.with_file_name("secrets.key")
}

pub(crate) fn load_key(path: &Path, create: bool) -> Result<Option<[u8; 32]>> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let text = String::from_utf8_lossy(&bytes);
            let raw = hex::decode(text.trim()).map_err(|_| GenieError::invalid(format!("{}: not a hex key", path.display())))?;
            let key: [u8; 32] = raw.try_into().map_err(|_| GenieError::invalid(format!("{}: the key must be 32 bytes", path.display())))?;
            Ok(Some(key))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => {
            let mut key = [0u8; 32];
            getrandom::fill(&mut key).expect("OS random source");
            write_private(path, &hex::encode(key))?;
            Ok(Some(key))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn write_private(path: &Path, text: &str) -> Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(text.as_bytes())?;
    Ok(())
}

pub(crate) fn seal(key: &[u8; 32], name: &str, value: &str) -> Result<String> {
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).expect("OS random source");
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    // The name is bound in: a sealed value cannot be moved to another secret.
    let payload = chacha20poly1305::aead::Payload { msg: value.as_bytes(), aad: name.as_bytes() };
    let sealed = cipher.encrypt(Nonce::from_slice(&nonce), payload).map_err(|_| GenieError::invalid("cannot seal the secret"))?;
    Ok(format!("{}{}", hex::encode(nonce), hex::encode(sealed)))
}

pub(crate) fn unseal(key: &[u8; 32], name: &str, sealed: &str) -> Option<String> {
    let raw = hex::decode(sealed).ok()?;
    if raw.len() < 12 {
        return None;
    }
    let (nonce, body) = raw.split_at(12);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let plain = cipher.decrypt(Nonce::from_slice(nonce), chacha20poly1305::aead::Payload { msg: body, aad: name.as_bytes() }).ok()?;
    String::from_utf8(plain).ok()
}

fn hint(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() < 12 {
        return String::new();
    }
    format!("…{}", chars[chars.len() - 4..].iter().collect::<String>())
}

fn check_value(value: &str) -> Result<()> {
    if value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(GenieError::invalid("the secret must be one line of at most 4096 characters"));
    }
    Ok(())
}

/// What a sealed repository token is bound to: it cannot be moved to another repository.
fn repo_aad(project: &str, repo: &str) -> String {
    format!("repo-token:{project}/{repo}")
}

impl ServerDb {
    /// Whether any token is stored sealed (people's keys, repositories' tokens,
    /// the provider keys shared with everyone, observation sources' credentials).
    pub fn has_sealed_secrets(&self) -> Result<bool> {
        let n: i64 = self.conn().query_row(
            "SELECT (SELECT COUNT(*) FROM user_secrets) + (SELECT COUNT(*) FROM repo_secrets) + (SELECT COUNT(*) FROM global_secrets) + (SELECT COUNT(*) FROM project_telegram) + (SELECT COUNT(*) FROM observation_secrets)",
            [],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// Set a person's secret (a blank value removes it).
    pub fn set_user_secret(&self, user: i64, name: &str, value: &str) -> Result<()> {
        let value = value.trim();
        if value.is_empty() {
            return self.delete_user_secret(user, name);
        }
        check_value(value)?;
        let key = load_key(&self.secrets_key, true)?.expect("created");
        self.conn().execute(
            "INSERT INTO user_secrets(user, name, sealed, hint, updated) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(user, name) DO UPDATE SET sealed = excluded.sealed, hint = excluded.hint, updated = excluded.updated",
            params![user, name, seal(&key, name, value)?, hint(value), now()],
        )?;
        Ok(())
    }

    pub fn delete_user_secret(&self, user: i64, name: &str) -> Result<()> {
        self.conn().execute("DELETE FROM user_secrets WHERE user = ?1 AND name = ?2", params![user, name])?;
        Ok(())
    }

    /// A person's secret in the clear, for the agents started on their behalf;
    /// `None` when it is unset or cannot be read back.
    pub fn user_secret(&self, user: i64, name: &str) -> Result<Option<String>> {
        let sealed: Option<String> = self
            .conn()
            .query_row("SELECT sealed FROM user_secrets WHERE user = ?1 AND name = ?2", params![user, name], |r| r.get(0))
            .optional()?;
        let Some(sealed) = sealed else { return Ok(None) };
        let Some(key) = load_key(&self.secrets_key, false)? else { return Ok(None) };
        Ok(unseal(&key, name, &sealed))
    }

    /// What can be shown about a person's secret.
    pub fn user_secret_info(&self, user: i64, name: &str) -> Result<Option<SecretInfo>> {
        let row: Option<(String, String, String)> = self
            .conn()
            .query_row("SELECT sealed, hint, updated FROM user_secrets WHERE user = ?1 AND name = ?2", params![user, name], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        let Some((sealed, hint, updated)) = row else { return Ok(None) };
        let readable = load_key(&self.secrets_key, false)?.is_some_and(|k| unseal(&k, name, &sealed).is_some());
        Ok(Some(SecretInfo { name: name.to_string(), hint, updated, unreadable: !readable }))
    }
}

impl ServerDb {
    /// Set the access token of a project's repository (a blank value removes it).
    pub fn set_repo_token(&self, project: &str, repo: &str, value: &str) -> Result<()> {
        let value = value.trim();
        if value.is_empty() {
            return self.delete_repo_token(project, repo);
        }
        check_value(value)?;
        self.repo(project, repo)?;
        let key = load_key(&self.secrets_key, true)?.expect("created");
        self.conn().execute(
            "INSERT INTO repo_secrets(project, repo, sealed, hint, updated) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(project, repo) DO UPDATE SET sealed = excluded.sealed, hint = excluded.hint, updated = excluded.updated",
            params![project, repo, seal(&key, &repo_aad(project, repo), value)?, hint(value), now()],
        )?;
        Ok(())
    }

    pub fn delete_repo_token(&self, project: &str, repo: &str) -> Result<()> {
        self.conn().execute("DELETE FROM repo_secrets WHERE project = ?1 AND repo = ?2", params![project, repo])?;
        Ok(())
    }

    /// A repository's token in the clear, for the server's own calls to the host;
    /// `None` when it is unset or cannot be read back.
    pub fn repo_token(&self, project: &str, repo: &str) -> Result<Option<String>> {
        let sealed: Option<String> = self
            .conn()
            .query_row("SELECT sealed FROM repo_secrets WHERE project = ?1 AND repo = ?2", params![project, repo], |r| r.get(0))
            .optional()?;
        let Some(sealed) = sealed else { return Ok(None) };
        let Some(key) = load_key(&self.secrets_key, false)? else { return Ok(None) };
        Ok(unseal(&key, &repo_aad(project, repo), &sealed))
    }

    /// What can be shown about a repository's token.
    pub fn repo_token_info(&self, project: &str, repo: &str) -> Result<Option<SecretInfo>> {
        let row: Option<(String, String, String)> = self
            .conn()
            .query_row("SELECT sealed, hint, updated FROM repo_secrets WHERE project = ?1 AND repo = ?2", params![project, repo], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        let Some((sealed, hint, updated)) = row else { return Ok(None) };
        let aad = repo_aad(project, repo);
        let readable = load_key(&self.secrets_key, false)?.is_some_and(|k| unseal(&k, &aad, &sealed).is_some());
        Ok(Some(SecretInfo { name: "token".to_string(), hint, updated, unreadable: !readable }))
    }
}

impl ServerDb {
    /// Set the value of an environment variable MCP connections refer to (a blank
    /// value removes it). The name is what `${env:…}` writes, so it is checked the
    /// way a shell would accept it.
    pub fn set_mcp_secret(&self, name: &str, value: &str) -> Result<()> {
        let value = value.trim();
        if value.is_empty() {
            return self.delete_mcp_secret(name);
        }
        check_env_name(name)?;
        check_value(value)?;
        let key = load_key(&self.secrets_key, true)?.expect("created");
        self.conn().execute(
            "INSERT INTO mcp_secrets(name, sealed, hint, updated) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(name) DO UPDATE SET sealed = excluded.sealed, hint = excluded.hint, updated = excluded.updated",
            params![name, seal(&key, &mcp_aad(name), value)?, hint(value), now()],
        )?;
        Ok(())
    }

    pub fn delete_mcp_secret(&self, name: &str) -> Result<()> {
        self.conn().execute("DELETE FROM mcp_secrets WHERE name = ?1", params![name])?;
        Ok(())
    }

    /// The value of an MCP environment variable in the clear, for the gateway and
    /// checks; `None` when it is unset or cannot be read back.
    pub fn mcp_secret(&self, name: &str) -> Result<Option<String>> {
        let sealed: Option<String> =
            self.conn().query_row("SELECT sealed FROM mcp_secrets WHERE name = ?1", params![name], |r| r.get(0)).optional()?;
        let Some(sealed) = sealed else { return Ok(None) };
        let Some(key) = load_key(&self.secrets_key, false)? else { return Ok(None) };
        Ok(unseal(&key, &mcp_aad(name), &sealed))
    }

    /// All MCP environment variables in the clear, for resolving `${env:…}`.
    pub fn mcp_env(&self) -> Result<std::collections::BTreeMap<String, String>> {
        let mut out = std::collections::BTreeMap::new();
        let rows: Vec<(String, String)> = self
            .conn()
            .prepare("SELECT name, sealed FROM mcp_secrets")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        if rows.is_empty() {
            return Ok(out);
        }
        let Some(key) = load_key(&self.secrets_key, false)? else { return Ok(out) };
        for (name, sealed) in rows {
            if let Some(v) = unseal(&key, &mcp_aad(&name), &sealed) {
                out.insert(name, v);
            }
        }
        Ok(out)
    }

    /// All MCP environment variables as the administration shows them: never the values.
    pub fn mcp_secret_infos(&self) -> Result<Vec<SecretInfo>> {
        let rows: Vec<(String, String, String, String)> = self
            .conn()
            .prepare("SELECT name, sealed, hint, updated FROM mcp_secrets ORDER BY name")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let key = load_key(&self.secrets_key, false)?;
        Ok(rows
            .into_iter()
            .map(|(name, sealed, hint, updated)| SecretInfo {
                unreadable: key.as_ref().is_none_or(|k| unseal(k, &mcp_aad(&name), &sealed).is_none()),
                name,
                hint,
                updated,
            })
            .collect())
    }
}

/// What a sealed MCP environment value is bound to: it cannot be moved to another name.
fn mcp_aad(name: &str) -> String {
    format!("mcp-env:{name}")
}

/// An environment variable name a shell accepts: letters, digits and `_`, not a digit first.
fn check_env_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(GenieError::invalid(format!("{name}: an environment variable name is letters, digits and `_}}`, not a digit first")))
    }
}

impl ServerDb {
    /// Set the provider key the server shares with everyone (a blank value
    /// removes it). The name is a provider of the registry, checked by the caller.
    pub fn set_global_secret(&self, name: &str, value: &str) -> Result<()> {
        let value = value.trim();
        if value.is_empty() {
            return self.delete_global_secret(name);
        }
        check_value(value)?;
        let key = load_key(&self.secrets_key, true)?.expect("created");
        self.conn().execute(
            "INSERT INTO global_secrets(name, sealed, hint, updated) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(name) DO UPDATE SET sealed = excluded.sealed, hint = excluded.hint, updated = excluded.updated",
            params![name, seal(&key, &global_aad(name), value)?, hint(value), now()],
        )?;
        Ok(())
    }

    pub fn delete_global_secret(&self, name: &str) -> Result<()> {
        self.conn().execute("DELETE FROM global_secrets WHERE name = ?1", params![name])?;
        Ok(())
    }

    /// A shared provider key in the clear, for the agents of people who keep no
    /// key of their own; `None` when it is unset or cannot be read back.
    pub fn global_secret(&self, name: &str) -> Result<Option<String>> {
        let sealed: Option<String> =
            self.conn().query_row("SELECT sealed FROM global_secrets WHERE name = ?1", params![name], |r| r.get(0)).optional()?;
        let Some(sealed) = sealed else { return Ok(None) };
        let Some(key) = load_key(&self.secrets_key, false)? else { return Ok(None) };
        Ok(unseal(&key, &global_aad(name), &sealed))
    }

    /// All shared provider keys as the administration shows them: never the values.
    pub fn global_secret_infos(&self) -> Result<Vec<SecretInfo>> {
        let rows: Vec<(String, String, String, String)> = self
            .conn()
            .prepare("SELECT name, sealed, hint, updated FROM global_secrets ORDER BY name")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let key = load_key(&self.secrets_key, false)?;
        Ok(rows
            .into_iter()
            .map(|(name, sealed, hint, updated)| SecretInfo {
                unreadable: key.as_ref().is_none_or(|k| unseal(k, &global_aad(&name), &sealed).is_none()),
                name,
                hint,
                updated,
            })
            .collect())
    }
}

/// What a sealed shared provider key is bound to: it cannot be moved to another table.
fn global_aad(name: &str) -> String {
    format!("global-key:{name}")
}

/// What a sealed observation source secret is bound to: it cannot be moved to another source
/// or project, and it does not open as a repository token (their prefixes differ).
fn observation_aad(project: &str, source: &str) -> String {
    format!("observation-source:{project}/{source}")
}

impl ServerDb {
    /// Set the secret of an observation source: its bearer token, or the `user:password` its
    /// auth needs. A blank value removes it. The source must exist.
    pub fn set_observation_secret(&self, project: &str, source: &str, value: &str) -> Result<()> {
        let value = value.trim();
        if value.is_empty() {
            return self.delete_observation_secret(project, source);
        }
        check_value(value)?;
        self.tx(|| {
            if self.observation_source(project, source)?.is_none() {
                return Err(sources::not_found(source));
            }
            let key = load_key(&self.secrets_key, true)?.expect("created");
            self.conn().execute(
                "INSERT INTO observation_secrets(project, source, sealed, hint, updated) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(project, source) DO UPDATE SET sealed = excluded.sealed, hint = excluded.hint, updated = excluded.updated",
                params![project, source, seal(&key, &observation_aad(project, source), value)?, hint(value), now()],
            )?;
            Ok(())
        })
    }

    /// Remove the secret of an observation source. The source must exist.
    pub fn delete_observation_secret(&self, project: &str, source: &str) -> Result<()> {
        self.tx(|| {
            if self.observation_source(project, source)?.is_none() {
                return Err(sources::not_found(source));
            }
            self.conn().execute("DELETE FROM observation_secrets WHERE project = ?1 AND source = ?2", params![project, source])?;
            Ok(())
        })
    }

    /// An observation source's secret in the clear, for the server's own calls to the source;
    /// `None` when it is unset or cannot be read back.
    pub fn observation_secret(&self, project: &str, source: &str) -> Result<Option<String>> {
        let sealed: Option<String> = self
            .conn()
            .query_row("SELECT sealed FROM observation_secrets WHERE project = ?1 AND source = ?2", params![project, source], |r| r.get(0))
            .optional()?;
        let Some(sealed) = sealed else { return Ok(None) };
        let Some(key) = load_key(&self.secrets_key, false)? else { return Ok(None) };
        Ok(unseal(&key, &observation_aad(project, source), &sealed))
    }

    /// What can be shown about an observation source's secret: a hint, never the value.
    pub fn observation_secret_info(&self, project: &str, source: &str) -> Result<Option<SecretInfo>> {
        let row: Option<(String, String, String)> = self
            .conn()
            .query_row(
                "SELECT sealed, hint, updated FROM observation_secrets WHERE project = ?1 AND source = ?2",
                params![project, source],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((sealed, hint, updated)) = row else { return Ok(None) };
        let aad = observation_aad(project, source);
        let readable = load_key(&self.secrets_key, false)?.is_some_and(|k| unseal(&k, &aad, &sealed).is_some());
        Ok(Some(SecretInfo { name: source.to_string(), hint, updated, unreadable: !readable }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_sealed_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let db = ServerDb::open(&dir.path().join("server.db")).unwrap();
        let u = db.create_user("ann", "Ann", None, None, false).unwrap();
        assert_eq!(db.user_secret(u.id, LITELLM).unwrap(), None);
        db.set_user_secret(u.id, LITELLM, "  sk-litellm-0123456789  ").unwrap();
        assert_eq!(db.user_secret(u.id, LITELLM).unwrap().as_deref(), Some("sk-litellm-0123456789"));
        let info = db.user_secret_info(u.id, LITELLM).unwrap().unwrap();
        assert_eq!(info.hint, "…6789");
        assert!(!info.unreadable);
        // Stored sealed, never in the clear.
        let sealed: String = db.conn().query_row("SELECT sealed FROM user_secrets", [], |r| r.get(0)).unwrap();
        assert!(!sealed.contains("sk-litellm"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("secrets.key")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // A blank value removes it.
        db.set_user_secret(u.id, LITELLM, " ").unwrap();
        assert_eq!(db.user_secret_info(u.id, LITELLM).unwrap(), None);
    }

    #[test]
    fn a_lost_key_makes_secrets_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let db = ServerDb::open(&dir.path().join("server.db")).unwrap();
        let u = db.create_user("ann", "Ann", None, None, false).unwrap();
        db.set_user_secret(u.id, LITELLM, "sk-litellm-0123456789").unwrap();
        std::fs::remove_file(dir.path().join("secrets.key")).unwrap();
        assert_eq!(db.user_secret(u.id, LITELLM).unwrap(), None);
        assert!(db.user_secret_info(u.id, LITELLM).unwrap().unwrap().unreadable);
        // Entering it again makes a new key.
        db.set_user_secret(u.id, LITELLM, "sk-litellm-abcdefghij").unwrap();
        assert_eq!(db.user_secret(u.id, LITELLM).unwrap().as_deref(), Some("sk-litellm-abcdefghij"));
    }

    #[test]
    fn a_repository_token_is_sealed_bound_to_its_repository_and_removed_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let db = ServerDb::open(&dir.path().join("server.db")).unwrap();
        db.create_project("shop", "Shop", "tracker", None, None).unwrap();
        for name in ["api", "web"] {
            db.add_repo(
                "shop",
                crate::repos::NewRepo {
                    name: name.into(),
                    host: "gl".into(),
                    remote: format!("acme/{name}"),
                    mount: Some(name.into()),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        assert_eq!(db.repo_token("shop", "api").unwrap(), None);
        db.set_repo_token("shop", "api", "  glpat-0123456789abcdef \n").unwrap();
        assert_eq!(db.repo_token("shop", "api").unwrap().as_deref(), Some("glpat-0123456789abcdef"));
        assert_eq!(db.repo_token("shop", "web").unwrap(), None);
        let info = db.repo_token_info("shop", "api").unwrap().unwrap();
        assert_eq!(info.hint, "…cdef");
        assert!(!info.unreadable);
        let sealed: String = db.conn().query_row("SELECT sealed FROM repo_secrets", [], |r| r.get(0)).unwrap();
        assert!(!sealed.contains("glpat"), "stored sealed, never in the clear");
        // Moved to another repository, the sealed value does not open.
        db.conn().execute("UPDATE repo_secrets SET repo = 'web'", []).unwrap();
        assert_eq!(db.repo_token("shop", "web").unwrap(), None);
        assert!(db.repo_token_info("shop", "web").unwrap().unwrap().unreadable);
        db.conn().execute("UPDATE repo_secrets SET repo = 'api'", []).unwrap();
        // A blank value removes it; so does detaching the repository.
        db.set_repo_token("shop", "api", " ").unwrap();
        assert_eq!(db.repo_token_info("shop", "api").unwrap(), None);
        db.set_repo_token("shop", "api", "glpat-0123456789abcdef").unwrap();
        db.remove_repo("shop", "api").unwrap();
        let left: i64 = db.conn().query_row("SELECT COUNT(*) FROM repo_secrets", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0);
        assert!(db.set_repo_token("shop", "api", "x").is_err(), "no token for a repository that is not there");
    }

    #[test]
    fn mcp_env_values_are_sealed_bound_to_their_name_and_listed() {
        let dir = tempfile::tempdir().unwrap();
        let db = ServerDb::open(&dir.path().join("server.db")).unwrap();
        assert!(db.mcp_env().unwrap().is_empty());
        db.set_mcp_secret("GITHUB_MCP_TOKEN", "  ghp_0123456789abcdef  ").unwrap();
        db.set_mcp_secret("SAP_USER", "a1b2c3d4e5f6g7h8").unwrap();
        assert_eq!(db.mcp_secret("GITHUB_MCP_TOKEN").unwrap().as_deref(), Some("ghp_0123456789abcdef"));
        let env = db.mcp_env().unwrap();
        assert_eq!(env.get("SAP_USER").map(String::as_str), Some("a1b2c3d4e5f6g7h8"));
        let infos = db.mcp_secret_infos().unwrap();
        assert_eq!(infos.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["GITHUB_MCP_TOKEN", "SAP_USER"]);
        assert_eq!(infos[0].hint, "…cdef");
        assert!(infos.iter().all(|i| !i.unreadable));
        let sealed: String = db.conn().query_row("SELECT sealed FROM mcp_secrets WHERE name = 'SAP_USER'", [], |r| r.get(0)).unwrap();
        assert!(!sealed.contains("a1b2c3"), "stored sealed, never in the clear");
        // A name a shell would not accept is refused before anything is stored.
        assert!(db.set_mcp_secret("1BAD", "x").is_err());
        assert!(db.set_mcp_secret("has space", "x").is_err());
        // Moved to another name, the sealed value does not open.
        db.conn().execute("UPDATE mcp_secrets SET name = 'OTHER' WHERE name = 'SAP_USER'", []).unwrap();
        assert_eq!(db.mcp_secret("OTHER").unwrap(), None);
        assert!(db.mcp_secret_infos().unwrap().iter().find(|i| i.name == "OTHER").unwrap().unreadable);
        // A blank value removes it.
        db.set_mcp_secret("OTHER", " ").unwrap();
        assert_eq!(db.mcp_secret("OTHER").unwrap(), None);
    }

    #[test]
    fn shared_provider_keys_are_sealed_bound_to_their_table_and_listed() {
        let dir = tempfile::tempdir().unwrap();
        let db = ServerDb::open(&dir.path().join("server.db")).unwrap();
        let u = db.create_user("ann", "Ann", None, None, false).unwrap();
        assert_eq!(db.global_secret(LITELLM).unwrap(), None);
        assert!(db.global_secret_infos().unwrap().is_empty());
        db.set_global_secret(LITELLM, "  sk-shared-0123456789  ").unwrap();
        db.set_global_secret("anthropic", "sk-ant-shared-0123456789").unwrap();
        assert_eq!(db.global_secret(LITELLM).unwrap().as_deref(), Some("sk-shared-0123456789"));
        // A person's key of the same provider is their own, not the shared one.
        assert_eq!(db.user_secret(u.id, LITELLM).unwrap(), None);
        db.set_user_secret(u.id, LITELLM, "sk-ann-0123456789").unwrap();
        assert_eq!(db.user_secret(u.id, LITELLM).unwrap().as_deref(), Some("sk-ann-0123456789"));
        let infos = db.global_secret_infos().unwrap();
        assert_eq!(infos.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["anthropic", LITELLM]);
        assert_eq!(infos[0].hint, "…6789", "anthropic, the first by name, is sk-ant-shared-0123456789");
        assert!(infos.iter().all(|i| !i.unreadable));
        assert!(db.has_sealed_secrets().unwrap());
        let sealed: String = db.conn().query_row("SELECT sealed FROM global_secrets WHERE name = 'litellm'", [], |r| r.get(0)).unwrap();
        assert!(!sealed.contains("sk-shared"), "stored sealed, never in the clear");
        // Overwriting the person's row with the shared sealed value, it does not open.
        db.conn()
            .execute(
                "UPDATE user_secrets SET sealed = (SELECT sealed FROM global_secrets WHERE name = 'litellm') WHERE user = ?1 AND name = 'litellm'",
                params![u.id],
            )
            .unwrap();
        assert_eq!(db.user_secret(u.id, "litellm").unwrap(), None, "the copied row does not unseal");
        // A blank value removes it.
        db.set_global_secret("anthropic", " ").unwrap();
        assert_eq!(db.global_secret("anthropic").unwrap(), None);
        assert_eq!(db.global_secret_infos().unwrap().len(), 1);
    }
}
