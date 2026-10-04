//! Request context: who is calling and which project they act in.
//!
//! Callers authenticate with a session cookie (browser), a bearer token (CLI,
//! agents) or — only while the server has no users — as the local owner from a
//! loopback address, which keeps the single-person local setup working with no
//! login at all.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::StatusCode;
use axum::http::header::{AUTHORIZATION, COOKIE};
use axum::http::request::Parts;
use genie_core::server_db::{Principal, ProjectRole, User};
use genie_core::{Actor, Capability, Role};

use super::ApiError;
use crate::state::App;

pub const SESSION_COOKIE: &str = "genie_session";
pub const PROJECT_COOKIE: &str = "genie_project";

#[derive(Debug, Clone)]
pub enum Who {
    Anonymous,
    /// A person. `local` marks the implicit owner of a server without users.
    User {
        user: User,
        local: bool,
    },
    Agent {
        project: String,
        /// The class of the agent's role.
        role: Role,
        /// The configured role it acts in (tokens issued before roles were configurable have none).
        role_id: Option<String>,
        name: String,
        team: Option<String>,
        job: Option<i64>,
    },
}

#[derive(Debug, Clone)]
pub struct Ctx {
    pub who: Who,
    /// Project named by the `X-Genie-Project` header, `?project=` or the project cookie.
    pub project_hint: Option<String>,
    /// The hint is the only project to act in (the command line and MCP ask for this).
    pub project_strict: bool,
    /// Raw session secret, for logout.
    pub session: Option<String>,
}

/// What a caller may do in one project.
#[derive(Debug, Clone)]
pub struct Access {
    pub project: String,
    pub actor: Actor,
    /// Membership role of a person; agents act with `Member` rights plus their workflow role.
    pub role: ProjectRole,
    pub user: Option<User>,
    pub agent: bool,
    /// Team of an agent member (its token is bound to it).
    pub agent_team: Option<String>,
    /// Job of a one-shot agent.
    pub agent_job: Option<i64>,
    /// The configured role of an agent.
    pub agent_role_id: Option<String>,
}

impl Access {
    pub fn write(&self) -> Result<(), ApiError> {
        if self.role.can_write() { Ok(()) } else { Err(ApiError::new(StatusCode::FORBIDDEN, "read-only access to this project")) }
    }
    pub fn admin(&self) -> Result<(), ApiError> {
        if self.role.can_admin() && !self.agent {
            Ok(())
        } else {
            Err(ApiError::new(StatusCode::FORBIDDEN, "project admin rights required"))
        }
    }
    pub fn is_human(&self) -> bool {
        self.actor.role == Role::Human
    }
    /// The caller of a task command.
    pub fn caller(&self) -> crate::tasks::Caller {
        let kind = if self.agent {
            crate::tasks::Kind::Agent { team: self.agent_team.clone(), job: self.agent_job }
        } else {
            crate::tasks::Kind::Person
        };
        crate::tasks::Caller { project: self.project.clone(), actor: self.actor.clone(), kind }
    }
    /// An agent needs the permission in its role; people and the orchestrator hold them all.
    pub fn can(&self, cap: Capability) -> Result<(), ApiError> {
        if !self.agent || self.actor.can(cap) {
            Ok(())
        } else {
            let role = self.agent_role_id.clone().unwrap_or_else(|| self.actor.role.to_string());
            Err(ApiError::new(StatusCode::FORBIDDEN, format!("role {role} does not have the permission {cap}")))
        }
    }
}

fn cookie(parts: &Parts, name: &str) -> Option<String> {
    parts.headers.get_all(COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

fn query_param(parts: &Parts, name: &str) -> Option<String> {
    parts.uri.query()?.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

impl FromRequestParts<Arc<App>> for Ctx {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, app: &Arc<App>) -> Result<Self, Self::Rejection> {
        let bearer = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|v| v.trim().to_string());
        let session = cookie(parts, SESSION_COOKIE);
        let loopback = ConnectInfo::<SocketAddr>::from_request_parts(parts, app).await.is_ok_and(|c| c.0.ip().is_loopback());
        let project_hint = parts
            .headers
            .get("x-genie-project")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .or_else(|| query_param(parts, "project"))
            .or_else(|| cookie(parts, PROJECT_COOKIE))
            .filter(|p| !p.is_empty());
        let project_strict = project_hint.is_some() && parts.headers.contains_key(crate::ops::api::STRICT);
        // The operator on the server's machine (the command line without a token).
        if parts.extensions.get::<crate::ops::api::OperatorAccess>().is_some() {
            let login = std::env::var("USER").ok().filter(|u| !u.is_empty()).unwrap_or_else(|| "operator".into());
            let user = User {
                id: 0,
                name: login.clone(),
                login,
                email: None,
                is_admin: true,
                disabled: false,
                created: String::new(),
                avatar: None,
            };
            return Ok(Ctx { who: Who::User { user, local: true }, project_hint, project_strict, session: None });
        }
        let (bearer2, session2) = (bearer.clone(), session.clone());
        let who = app
            .blocking(move |app| {
                app.with_server(|db| {
                    if let Some(token) = bearer2 {
                        return Ok(match db.resolve_token(&token)? {
                            Some(Principal::User { user }) => Who::User { user, local: false },
                            Some(Principal::Agent { project, role, role_id, name, team, job }) => {
                                Who::Agent { project, role, role_id, name, team, job }
                            }
                            None => Who::Anonymous,
                        });
                    }
                    if let Some(s) = session2
                        && let Some(user) = db.session_user(&s)?
                    {
                        return Ok(Who::User { user, local: false });
                    }
                    if loopback && db.user_count()? == 0 {
                        let login = std::env::var("USER").ok().filter(|u| !u.is_empty()).unwrap_or_else(|| "owner".into());
                        let user = User {
                            id: 0,
                            name: login.clone(),
                            login,
                            email: None,
                            is_admin: true,
                            disabled: false,
                            created: String::new(),
                            avatar: None,
                        };
                        return Ok(Who::User { user, local: true });
                    }
                    Ok(Who::Anonymous)
                })
            })
            .await?;
        if bearer.is_some() && matches!(who, Who::Anonymous) {
            return Err(ApiError::new(StatusCode::UNAUTHORIZED, "invalid or expired token"));
        }
        Ok(Ctx { who, project_hint, project_strict, session })
    }
}

impl Ctx {
    pub fn user(&self) -> Result<&User, ApiError> {
        match &self.who {
            Who::User { user, .. } => Ok(user),
            Who::Agent { .. } => Err(ApiError::new(StatusCode::FORBIDDEN, "agents cannot do this")),
            Who::Anonymous => Err(ApiError::unauthorized()),
        }
    }

    pub fn server_admin(&self) -> Result<&User, ApiError> {
        let u = self.user()?;
        if u.is_admin { Ok(u) } else { Err(ApiError::new(StatusCode::FORBIDDEN, "server admin rights required")) }
    }

    /// Resolve the project and the caller's rights in it. `explicit` wins over the hint.
    pub async fn access(&self, app: &Arc<App>, explicit: Option<&str>) -> Result<Access, ApiError> {
        let strict = explicit.is_some() || self.project_strict;
        let wanted = explicit.map(str::to_string).or_else(|| self.project_hint.clone());
        match &self.who {
            Who::Anonymous => Err(ApiError::unauthorized()),
            Who::Agent { project, role, role_id, name, team, job } => {
                if let Some(w) = &wanted
                    && w != project
                {
                    return Err(ApiError::new(StatusCode::FORBIDDEN, format!("this agent token is bound to project {project}")));
                }
                // The role's permissions come from the configuration at the time of the
                // request, so a revoked permission stops working at once.
                let caps = role_id.as_deref().and_then(|id| app.agents().roles.get(id).map(|r| r.capabilities.clone()));
                let actor = match caps {
                    Some(caps) if *role != Role::Orchestrator => Actor::with_caps(name.clone(), *role, caps),
                    _ => Actor::new(name.clone(), *role),
                };
                Ok(Access {
                    project: project.clone(),
                    actor,
                    role: ProjectRole::Member,
                    user: None,
                    agent: true,
                    agent_team: team.clone(),
                    agent_job: *job,
                    agent_role_id: role_id.clone(),
                })
            }
            Who::User { user, .. } => {
                let user = user.clone();
                app.blocking(move |app| {
                    app.with_server(|db| {
                        // An explicit project must be accessible; a hint (cookie, header) may be
                        // stale and falls back to the other accessible projects.
                        let mut candidates: Vec<String> = wanted.iter().cloned().collect();
                        if !strict {
                            candidates.extend(db.projects()?.into_iter().map(|p| p.slug));
                        }
                        for slug in candidates {
                            if let Some(role) = db.project_role(&slug, &user)? {
                                return Ok(Some(Access {
                                    project: slug,
                                    actor: Actor::new(user.login.clone(), Role::Human),
                                    role,
                                    user: Some(user.clone()),
                                    agent: false,
                                    agent_team: None,
                                    agent_job: None,
                                    agent_role_id: None,
                                }));
                            }
                        }
                        Ok(None)
                    })
                })
                .await?
                .ok_or_else(|| match (strict, explicit.map(str::to_string).or_else(|| self.project_hint.clone())) {
                    (true, Some(p)) => ApiError::new(StatusCode::FORBIDDEN, format!("no access to project {p}")),
                    _ => ApiError::new(StatusCode::NOT_FOUND, "no accessible project; create one or ask for an invitation"),
                })
            }
        }
    }
}
