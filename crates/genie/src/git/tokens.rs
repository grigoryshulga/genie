//! The token a repository bound to a provider connection is reached with.
//!
//! Nothing here is stored: an installation token is minted for one repository when an operation
//! needs it, kept in memory until shortly before it lapses, and handed to the code that talks to the
//! host ([`super::store::host_of`]). Agents never see it: their clones point at the proxy.
//!
//! A connection that cannot be used (revoked, suspended, owner removed) fails here, before any call
//! to the provider, so a block holds for every path that reaches the host.

use genie_core::connections::{Connection, Health};
use genie_core::repos::ProjectRepo;

use super::github_app;
use super::hosts::Host;
use super::provider::ApiError;
use crate::state::App;

/// Run a future to completion from synchronous code on any thread (a request handler's blocking
/// pool, the delivery poller, a test): on a thread of its own, so it cannot panic inside a runtime.
fn run<T: Send>(fut: impl std::future::Future<Output = T> + Send) -> T {
    std::thread::scope(|s| {
        s.spawn(|| tokio::runtime::Builder::new_current_thread().enable_all().build().expect("a current-thread runtime").block_on(fut))
            .join()
            .expect("the token thread did not panic")
    })
}

/// The connection of `repo` when it may be used now.
pub fn usable(app: &App, repo: &ProjectRepo) -> Result<Option<Connection>, String> {
    let Some(id) = &repo.connection else { return Ok(None) };
    let conn = app.with_server(|db| db.connection(id)).map_err(|e| e.to_string())?;
    if let Some(why) = conn.blocked() {
        return Err(why);
    }
    if conn.host != repo.host {
        return Err(format!("repository {} is bound to a connection on host {}, not {}", repo.name, conn.host, repo.host));
    }
    Ok(Some(conn))
}

/// A token for `repo`, narrowed to that repository.
pub fn resolve(app: &App, repo: &ProjectRepo, host: &Host) -> Result<String, String> {
    let conn = usable(app, repo)?.ok_or_else(|| format!("repository {} has no connection", repo.name))?;
    let repo_id =
        repo.provider_repo_id.ok_or_else(|| format!("repository {} is bound to a connection but has no provider id", repo.name))?;
    let key = format!("{}:{repo_id}", conn.id);
    if let Some(token) = app.git.cached_token(&key) {
        return Ok(token);
    }
    let cfg = app.cfg.git_apps.github.as_ref().ok_or("the GitHub App is not configured on this server (gitApps.github)")?;
    match run(github_app::mint(host, cfg, conn.installation_id, &[repo_id])) {
        Ok(minted) => {
            app.git.cache_token(&key, minted.token.clone(), minted.expires);
            if conn.health != Health::Ok {
                let _ = app.with_server(|db| db.set_connection_health(&conn.id, Health::Ok, ""));
            }
            Ok(minted.token)
        }
        Err(e) => {
            let (health, note) = classify(&e);
            let _ = app.with_server(|db| db.set_connection_health(&conn.id, health, &note));
            Err(format!("the connection to {} could not give a token: {note}", conn.account_login))
        }
    }
}

/// What a failed mint says about the connection.
pub fn classify(e: &ApiError) -> (Health, String) {
    match e {
        ApiError::NotFound(m) => (Health::Revoked, format!("the installation no longer exists on the provider ({m})")),
        ApiError::Auth(m) if m.to_lowercase().contains("suspended") => (Health::Suspended, m.clone()),
        ApiError::Auth(m) | ApiError::Rejected(m)
            if m.to_lowercase().contains("not accessible") || m.to_lowercase().contains("permission") =>
        {
            (Health::PermissionMissing, m.clone())
        }
        ApiError::Auth(m) => (Health::NeedsAttention, format!("the provider refused the app's credentials: {m}")),
        other => (Health::NeedsAttention, other.to_string()),
    }
}

/// When the server last needs to forget tokens: a connection was revoked or reassigned.
pub fn forget(app: &App, connection: &str) {
    app.git.forget_tokens(connection);
}
