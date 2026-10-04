//! The git proxy: `http://<server>/git/<project>/<repo>.git`, git's smart HTTP protocol.
//!
//! Agents' clones have this as `origin` and authenticate with their genie token (HTTP
//! basic, any user name); they hold no credentials for the git host. What the proxy does:
//!
//! - **fetch/clone** are served from the server's mirror (freshened from the host at
//!   most every 30 s), if the effective policy lets the agent read;
//! - **push** is checked before anything is stored: every ref update must be allowed by
//!   the effective policy (branch name, protected branches, deletes, force-pushes;
//!   git itself refuses non-fast-forwards and deletes in the mirror unless the policy
//!   allows them). An accepted push lands in the mirror and is then forwarded to the
//!   host with the host's token; if the host refuses, the mirror is put back and the
//!   agent sees the refusal like any `! [remote rejected]`.
//!
//! The protocol is spoken in version 0 (stateless RPC), which every git client accepts.
//! Refused pushes and accepted ones are recorded in the project's journal.

// A refusal here is the HTTP answer itself (a `Response`), which is what the helpers hand back.
#![allow(clippy::result_large_err)]

use std::io::Write as _;
use std::path::{Path as FsPath, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine;
use futures_util::StreamExt;
use genie_core::events;
use genie_core::repos::{Delivery, ProjectRepo};
use genie_core::server_db::Principal;
use serde::Deserialize;
use serde_json::json;
use tokio::io::AsyncReadExt;

use crate::git::delivery::watching;
use crate::git::policy::{Effective, Push, glob_match};
use crate::git::service::{self, AgentId};
use crate::git::store::{self, ZERO_SHA};
use crate::state::App;

/// How fresh the mirror must be for a clone or fetch.
const FRESH: Duration = Duration::from_secs(30);
/// The largest push body accepted.
const MAX_PUSH: u64 = 1 << 30;
/// The largest fetch request (wants and haves) accepted.
const MAX_FETCH_REQUEST: usize = 64 << 20;

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/git/{project}/{repo}/info/refs", get(info_refs))
        .route("/git/{project}/{repo}/git-upload-pack", post(upload_pack))
        .route("/git/{project}/{repo}/git-receive-pack", post(receive_pack))
}

// --- who is asking ---------------------------------------------------------------

enum Caller {
    Agent {
        id: AgentId,
        name: String,
    },
    /// A person with read access to the project.
    User,
}

fn text(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, [(header::CONTENT_TYPE, "text/plain; charset=utf-8")], msg.into() + "\n").into_response()
}

fn unauthorized() -> Response {
    let mut r = text(StatusCode::UNAUTHORIZED, "genie: authentication required (an agent's genie token)");
    r.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Basic realm=\"genie\""));
    r
}

/// The token in a basic or bearer `Authorization` header.
fn token_of(headers: &HeaderMap) -> Option<String> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    if let Some(b) = v.strip_prefix("Bearer ") {
        return Some(b.trim().to_string());
    }
    let raw = base64::engine::general_purpose::STANDARD.decode(v.strip_prefix("Basic ")?.trim()).ok()?;
    let s = String::from_utf8(raw).ok()?;
    let (_, pass) = s.split_once(':')?;
    Some(pass.to_string())
}

async fn authenticate(app: &Arc<App>, headers: &HeaderMap, project: &str) -> Result<Caller, Response> {
    let Some(token) = token_of(headers) else { return Err(unauthorized()) };
    let slug = project.to_string();
    let found = app
        .blocking(move |app| {
            app.with_server(|db| {
                Ok(match db.resolve_token(&token)? {
                    Some(Principal::Agent { project, role, role_id, name, team, job }) => {
                        Some((Some((project, role, role_id, name, team, job)), false))
                    }
                    Some(Principal::User { user }) => Some((None, db.project_role(&slug, &user)?.is_some())),
                    None => None,
                })
            })
        })
        .await
        .map_err(|e| text(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    match found {
        None => Err(unauthorized()),
        Some((Some((p, role, role_id, name, team, job)), _)) => {
            if p != project {
                return Err(text(StatusCode::FORBIDDEN, format!("genie: this token is bound to project {p}")));
            }
            Ok(Caller::Agent { id: AgentId { role, role_id, team, job }, name })
        }
        Some((None, true)) => Ok(Caller::User),
        Some((None, false)) => Err(text(StatusCode::FORBIDDEN, "genie: no access to this project")),
    }
}

/// What a request needs: the repository, the host and the caller's rules for it.
struct Target {
    repo: ProjectRepo,
    host: crate::git::hosts::Host,
    /// `None` for a person (read only).
    eff: Option<Effective>,
    agent: Option<(String, AgentId)>,
}

async fn target(app: &Arc<App>, project: &str, repo: &str, caller: &Caller) -> Result<Target, Response> {
    let name = repo.strip_suffix(".git").unwrap_or(repo).to_string();
    let (project, agent) = (
        project.to_string(),
        match caller {
            Caller::Agent { id, name } => Some((name.clone(), id.clone())),
            Caller::User => None,
        },
    );
    let a2 = agent.clone();
    app.blocking(move |app| {
        let Some(record) = app.with_server(|db| db.repo_opt(&project, &name))? else {
            return Ok(Err(text(StatusCode::NOT_FOUND, "genie: no such repository in this project")));
        };
        let host = match store::host_of(app, &record) {
            Ok(h) => h,
            Err(e) => return Ok(Err(text(StatusCode::BAD_GATEWAY, format!("genie: {e}")))),
        };
        let eff = match &a2 {
            Some((_, id)) => match service::effective_for(app, &project, &record, id) {
                Ok(e) => Some(e),
                Err(e) => return Ok(Err(text(StatusCode::INTERNAL_SERVER_ERROR, format!("genie: {e}")))),
            },
            None => None,
        };
        Ok(Ok(Target { repo: record, host, eff, agent: a2 }))
    })
    .await
    .map_err(|e| text(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
}

/// Write an entry into the project's journal (best effort: the protocol answer does not wait on it).
async fn record(app: &Arc<App>, project: &str, kind: &'static str, t: &Target, payload: serde_json::Value) {
    let Some((name, id)) = &t.agent else { return };
    let (project, subject, actor, class) = (project.to_string(), id.team.clone(), name.clone(), id.role);
    let _ = app
        .blocking(move |app| {
            app.with_tracker(&project, |tr| {
                events::append(tr.conn(), kind, subject.as_deref(), &actor, class.as_str(), payload).map(|_| ())
            })
        })
        .await;
}

fn forbid(why: String) -> Response {
    text(StatusCode::FORBIDDEN, format!("genie: {why}"))
}

// --- info/refs ---------------------------------------------------------------------

#[derive(Deserialize)]
struct ServiceQuery {
    service: Option<String>,
}

fn pkt(data: &[u8]) -> Vec<u8> {
    let mut out = format!("{:04x}", data.len() + 4).into_bytes();
    out.extend_from_slice(data);
    out
}

async fn info_refs(
    State(app): State<Arc<App>>,
    Path((project, repo)): Path<(String, String)>,
    Query(q): Query<ServiceQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(service) = q.service.filter(|s| s == "git-upload-pack" || s == "git-receive-pack") else {
        return text(StatusCode::FORBIDDEN, "genie: only the smart protocol is served (git-upload-pack, git-receive-pack)");
    };
    let caller = match authenticate(&app, &headers, &project).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let t = match target(&app, &project, &repo, &caller).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    if let Some(e) = &t.eff
        && !e.read
    {
        record(&app, &project, events::GIT_DENIED, &t, json!({ "repo": t.repo.name, "op": "read", "reason": "no read access" })).await;
        return forbid(format!("{}: no access to this repository for your role", t.repo.name));
    }
    if service == "git-receive-pack" && t.eff.is_none() {
        return forbid("people push to the git host directly; the proxy takes pushes of agents".into());
    }
    let svc = service.clone();
    let out = app.blocking(move |app| Ok(advertise(app, &t, &svc))).await;
    match out {
        Ok(Ok(body)) => git_response(&format!("application/x-{service}-advertisement"), Body::from(body)),
        Ok(Err(e)) => text(StatusCode::BAD_GATEWAY, format!("genie: {e}")),
        Err(e) => text(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// The mirror's refs in the smart protocol's advertisement (freshening the mirror first).
fn advertise(app: &App, t: &Target, service: &str) -> Result<Vec<u8>, String> {
    let mirror = store::refresh(app, &t.host, &t.repo.remote, FRESH)?;
    let sub = service.strip_prefix("git-").unwrap_or_default();
    let out = Command::new("git")
        .args([sub, "--stateless-rpc", "--advertise-refs"])
        .arg(&mirror.path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!("git {sub} failed"));
    }
    let mut body = pkt(format!("# service={service}\n").as_bytes());
    body.extend_from_slice(b"0000");
    body.extend_from_slice(&out.stdout);
    Ok(body)
}

fn git_response(content_type: &str, body: Body) -> Response {
    let mut r = Response::new(body);
    r.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_str(content_type).unwrap_or(HeaderValue::from_static("application/octet-stream")));
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache, max-age=0, must-revalidate"));
    r
}

// --- fetch ---------------------------------------------------------------------------

/// The request body, un-gzipped when the client compressed it.
async fn plain_body(headers: &HeaderMap, body: Bytes) -> Result<Vec<u8>, Response> {
    let gzip = headers.get(header::CONTENT_ENCODING).and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("gzip"));
    if !gzip {
        return Ok(body.to_vec());
    }
    let mut child = tokio::process::Command::new("gzip")
        .arg("-dc")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| text(StatusCode::INTERNAL_SERVER_ERROR, format!("genie: gzip: {e}")))?;
    let mut stdin = child.stdin.take().expect("piped");
    let feed = tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        let _ = stdin.write_all(&body).await;
    });
    let out = child.wait_with_output().await.map_err(|e| text(StatusCode::INTERNAL_SERVER_ERROR, format!("genie: gzip: {e}")))?;
    let _ = feed.await;
    if !out.status.success() || out.stdout.len() > MAX_FETCH_REQUEST {
        return Err(text(StatusCode::BAD_REQUEST, "genie: the request body is not valid gzip or is too large"));
    }
    Ok(out.stdout)
}

async fn upload_pack(
    State(app): State<Arc<App>>,
    Path((project, repo)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let caller = match authenticate(&app, &headers, &project).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let t = match target(&app, &project, &repo, &caller).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    if let Some(e) = &t.eff
        && !e.read
    {
        return forbid(format!("{}: no access to this repository for your role", t.repo.name));
    }
    if body.len() > MAX_FETCH_REQUEST {
        return text(StatusCode::PAYLOAD_TOO_LARGE, "genie: the request is too large");
    }
    let request = match plain_body(&headers, body).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    let mirror = match app
        .blocking(move |app| store::refresh(app, &t.host, &t.repo.remote, FRESH).map(|r| r.path).map_err(crate::state::AppError::Internal))
        .await
    {
        Ok(p) => p,
        Err(e) => return text(StatusCode::BAD_GATEWAY, format!("genie: {e}")),
    };
    let mut child = match tokio::process::Command::new("git")
        .args(["upload-pack", "--stateless-rpc"])
        .arg(&mirror)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return text(StatusCode::INTERNAL_SERVER_ERROR, format!("genie: git: {e}")),
    };
    let mut stdin = child.stdin.take().expect("piped");
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        let _ = stdin.write_all(&request).await;
    });
    let stdout = child.stdout.take().expect("piped");
    // The child lives as long as the response is being read; dropping the response kills it.
    let stream = futures_util::stream::unfold((stdout, child), |(mut out, child)| async move {
        let mut buf = vec![0u8; 64 * 1024];
        match out.read(&mut buf).await {
            Ok(0) | Err(_) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok::<Bytes, std::io::Error>(Bytes::from(buf)), (out, child)))
            }
        }
    })
    .boxed();
    git_response("application/x-git-upload-pack-result", Body::from_stream(stream))
}

// --- push ----------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Command2 {
    old: String,
    new: String,
    refname: String,
}

impl Command2 {
    fn delete(&self) -> bool {
        self.new == ZERO_SHA
    }
    fn branch(&self) -> Option<&str> {
        self.refname.strip_prefix("refs/heads/")
    }
}

/// The ref updates at the head of a `git-receive-pack` request, and the capabilities the client asked for.
fn parse_commands(head: &[u8]) -> Result<(Vec<Command2>, String), String> {
    let (mut cmds, mut caps, mut i) = (Vec::new(), String::new(), 0usize);
    loop {
        let len = head
            .get(i..i + 4)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| usize::from_str_radix(h, 16).ok())
            .ok_or("malformed push request")?;
        if len == 0 {
            return Ok((cmds, caps));
        }
        if len < 4 || i + len > head.len() {
            return Err("malformed push request".into());
        }
        let mut line = &head[i + 4..i + len];
        i += len;
        if let Some(nul) = line.iter().position(|b| *b == 0) {
            caps = String::from_utf8_lossy(&line[nul + 1..]).trim().to_string();
            line = &line[..nul];
        }
        let line = String::from_utf8_lossy(line);
        let line = line.trim_end();
        if line.starts_with("shallow ") {
            continue;
        }
        if line.starts_with("push-cert") {
            return Err("signed pushes are not supported".into());
        }
        let mut it = line.splitn(3, ' ');
        let (Some(old), Some(new), Some(refname)) = (it.next(), it.next(), it.next()) else { return Err("malformed push command".into()) };
        let sha = |s: &str| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit());
        if !sha(old) || !sha(new) || refname.is_empty() {
            return Err("malformed push command".into());
        }
        cmds.push(Command2 { old: old.into(), new: new.into(), refname: refname.into() });
    }
}

/// git's `report-status` answer (`unpack ok`, then `ok`/`ng` per ref), in side-band when the client asked for it.
fn report(unpack: &str, results: &[(String, Result<(), String>)], sideband: bool, notes: &[String]) -> Vec<u8> {
    let mut inner = pkt(format!("unpack {unpack}\n").as_bytes());
    for (r, res) in results {
        let line = match res {
            Ok(()) => format!("ok {r}\n"),
            Err(why) => format!("ng {r} {}\n", why.replace(['\n', '\r'], " ")),
        };
        inner.extend(pkt(line.as_bytes()));
    }
    inner.extend_from_slice(b"0000");
    if !sideband {
        return inner;
    }
    let mut out = Vec::new();
    for n in notes {
        let mut m = vec![2u8];
        m.extend_from_slice(format!("genie: {n}\n").as_bytes());
        out.extend(pkt(&m));
    }
    let mut data = vec![1u8];
    data.extend(inner);
    out.extend(pkt(&data));
    out.extend_from_slice(b"0000");
    out
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Stream a request body to a file, refusing more than `MAX_PUSH` bytes.
async fn spool(app: &App, body: Body) -> Result<PathBuf, Response> {
    let dir = app.data.join("runtime").join("git-tmp");
    std::fs::create_dir_all(&dir).map_err(|e| text(StatusCode::INTERNAL_SERVER_ERROR, format!("genie: {e}")))?;
    let path = dir.join(format!("push-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
    let mut file = std::fs::File::create(&path).map_err(|e| text(StatusCode::INTERNAL_SERVER_ERROR, format!("genie: {e}")))?;
    let (mut total, mut stream) = (0u64, body.into_data_stream());
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                return Err(text(StatusCode::BAD_REQUEST, format!("genie: {e}")));
            }
        };
        total += chunk.len() as u64;
        if total > MAX_PUSH {
            let _ = std::fs::remove_file(&path);
            return Err(text(StatusCode::PAYLOAD_TOO_LARGE, "genie: the push is too large"));
        }
        if let Err(e) = file.write_all(&chunk) {
            let _ = std::fs::remove_file(&path);
            return Err(text(StatusCode::INTERNAL_SERVER_ERROR, format!("genie: {e}")));
        }
    }
    Ok(path)
}

fn read_head(path: &FsPath) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::new();
    std::fs::File::open(path)?.take(1 << 20).read_to_end(&mut buf)?;
    Ok(buf)
}

async fn receive_pack(
    State(app): State<Arc<App>>,
    Path((project, repo)): Path<(String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let caller = match authenticate(&app, &headers, &project).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let t = match target(&app, &project, &repo, &caller).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    let Some(eff) = t.eff.clone() else { return forbid("people push to the git host directly; the proxy takes pushes of agents".into()) };
    let gzip = headers.get(header::CONTENT_ENCODING).and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("gzip"));
    let spooled = match spool(&app, body).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let body = if gzip {
        match gunzip_file(&spooled).await {
            Ok(p) => p,
            Err(r) => {
                let _ = std::fs::remove_file(&spooled);
                return r;
            }
        }
    } else {
        spooled
    };
    let (project2, repo2) = (project.clone(), t.repo.name.clone());
    let done = app.blocking(move |app| Ok(push(app, &project2, &t, &eff, &body))).await;
    match done {
        Ok((status, bytes)) => {
            if status == StatusCode::OK {
                git_response("application/x-git-receive-pack-result", Body::from(bytes))
            } else {
                text(status, String::from_utf8_lossy(&bytes).to_string())
            }
        }
        Err(e) => text(StatusCode::INTERNAL_SERVER_ERROR, format!("genie: {repo2}: {e}")),
    }
}

async fn gunzip_file(path: &FsPath) -> Result<PathBuf, Response> {
    let out = path.with_extension("raw");
    let src = std::fs::File::open(path).map_err(|e| text(StatusCode::INTERNAL_SERVER_ERROR, format!("genie: {e}")))?;
    let dst = std::fs::File::create(&out).map_err(|e| text(StatusCode::INTERNAL_SERVER_ERROR, format!("genie: {e}")))?;
    let status = tokio::process::Command::new("gzip")
        .arg("-dc")
        .stdin(src)
        .stdout(dst)
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(|e| text(StatusCode::INTERNAL_SERVER_ERROR, format!("genie: gzip: {e}")))?;
    let _ = std::fs::remove_file(path);
    if status.success() { Ok(out) } else { Err(text(StatusCode::BAD_REQUEST, "genie: the push body is not valid gzip")) }
}

/// The blocking part of a push: check, store in the mirror, forward to the host, answer.
fn push(app: &App, project: &str, t: &Target, eff: &Effective, body: &FsPath) -> (StatusCode, Vec<u8>) {
    let result = push_inner(app, project, t, eff, body);
    let _ = std::fs::remove_file(body);
    result
}

fn push_inner(app: &App, project: &str, t: &Target, eff: &Effective, body: &FsPath) -> (StatusCode, Vec<u8>) {
    let bad = |m: &str| (StatusCode::BAD_REQUEST, format!("genie: {m}\n").into_bytes());
    let head = match read_head(body) {
        Ok(h) => h,
        Err(e) => return bad(&e.to_string()),
    };
    let (cmds, caps) = match parse_commands(&head) {
        Ok(x) => x,
        Err(e) => return bad(&e),
    };
    if cmds.is_empty() {
        return bad("no ref updates in the push");
    }
    let sideband = caps.split_whitespace().any(|c| c == "side-band-64k" || c == "side-band");
    let who = t.agent.as_ref().map(|(n, _)| n.clone()).unwrap_or_default();

    // 1. The policy, before anything is stored.
    let verdicts: Vec<(String, Result<(), String>)> =
        cmds.iter().map(|c| (c.refname.clone(), eff.check_push(&c.refname, c.delete()))).collect();
    if verdicts.iter().any(|(_, v)| v.is_err()) {
        // One refused ref refuses the whole push (git pushes are atomic for the agent's purposes).
        let reasons: Vec<String> = verdicts.iter().filter_map(|(_, v)| v.as_ref().err().cloned()).collect();
        let results: Vec<(String, Result<(), String>)> = verdicts
            .into_iter()
            .map(|(r, v)| match v {
                Err(e) => (r, Err(e)),
                Ok(()) => (r, Err("not pushed: another ref of this push was refused".to_string())),
            })
            .collect();
        record_denied(app, project, t, &cmds, &reasons);
        return (StatusCode::OK, report("ok", &results, sideband, &reasons));
    }

    // 2. Store in the mirror (git refuses non-fast-forwards and deletes unless the policy allows them).
    let mirror = match store::refresh(app, &t.host, &t.repo.remote, FRESH) {
        Ok(r) => r.path,
        Err(e) => {
            let results = cmds.iter().map(|c| (c.refname.clone(), Err(format!("the host is not reachable: {e}")))).collect::<Vec<_>>();
            return (StatusCode::OK, report("ok", &results, sideband, &[format!("the host is not reachable: {e}")]));
        }
    };
    let lock = app.git.lock(&mirror.to_string_lossy());
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let allow_force = eff.policy.force_push;
    let stdin = match std::fs::File::open(body) {
        Ok(f) => f,
        Err(e) => return bad(&e.to_string()),
    };
    let out = Command::new("git")
        .args([
            "-c",
            &format!("receive.denyNonFastForwards={}", !allow_force),
            "-c",
            &format!("receive.denyDeletes={}", !eff.policy.delete_branches),
        ])
        .args(["receive-pack", "--stateless-rpc"])
        .arg(&mirror)
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let out = match out {
        Ok(o) => o,
        Err(e) => return bad(&format!("git: {e}")),
    };
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("the pack could not be stored").to_string();
        let results = cmds.iter().map(|c| (c.refname.clone(), Err("not stored".to_string()))).collect::<Vec<_>>();
        return (StatusCode::OK, report(&why, &results, sideband, std::slice::from_ref(&why)));
    }

    // 3. What git stored goes on to the host; what the host refuses is taken back.
    let mut results: Vec<(String, Result<(), String>)> = Vec::new();
    let mut upstream_failed = false;
    for c in &cmds {
        let now = store::ref_sha(&mirror, &c.refname);
        let stored = if c.delete() { now.is_none() } else { now.as_deref() == Some(c.new.as_str()) };
        if !stored {
            results.push((
                c.refname.clone(),
                Err(if c.delete() {
                    "deleting branches is not allowed".into()
                } else {
                    "not a fast-forward: force-push is not allowed".into()
                }),
            ));
            continue;
        }
        let branch = c.branch().unwrap_or_default();
        let sent = if c.delete() {
            store::delete_upstream(&t.host, &mirror, branch)
        } else {
            store::push_upstream(&t.host, &mirror, &c.new, branch, allow_force)
        };
        match sent {
            Ok(()) => results.push((c.refname.clone(), Ok(()))),
            Err(e) => {
                store::restore_ref(&mirror, &c.refname, &c.old);
                upstream_failed = true;
                let line = e
                    .lines()
                    .rev()
                    .find(|l| l.contains("rejected") || l.contains("denied") || l.contains("error") || l.contains("fatal"))
                    .unwrap_or(&e);
                results.push((c.refname.clone(), Err(format!("the git host refused: {}", line.trim()))));
            }
        }
    }
    let ok: Vec<&Command2> = cmds.iter().zip(&results).filter(|(_, (_, r))| r.is_ok()).map(|(c, _)| c).collect();
    if !ok.is_empty() {
        record_pushed(app, project, t, &who, &ok);
    }
    let failures: Vec<String> = results.iter().filter_map(|(r, v)| v.as_ref().err().map(|e| format!("{r}: {e}"))).collect();
    if failures.is_empty() && !upstream_failed {
        (StatusCode::OK, out.stdout)
    } else {
        (StatusCode::OK, report("ok", &results, sideband, &failures))
    }
}

fn record_denied(app: &App, project: &str, t: &Target, cmds: &[Command2], reasons: &[String]) {
    let Some((name, id)) = &t.agent else { return };
    let refs: Vec<&str> = cmds.iter().map(|c| c.refname.as_str()).collect();
    let payload = json!({ "repo": t.repo.name, "op": "push", "refs": refs, "reason": reasons.join("; ") });
    let _ = app.with_tracker(project, |tr| {
        events::append(tr.conn(), events::GIT_DENIED, id.team.as_deref(), name, id.role.as_str(), payload).map(|_| ())
    });
}

fn record_pushed(app: &App, project: &str, t: &Target, who: &str, ok: &[&Command2]) {
    let Some((_, id)) = &t.agent else { return };
    let refs: Vec<_> = ok.iter().map(|c| json!({ "ref": c.refname, "old": c.old, "new": c.new })).collect();
    let task = eff_task(t);
    let payload = json!({ "repo": t.repo.name, "task": task, "refs": refs });
    let _ = app.with_tracker(project, |tr| {
        events::append(tr.conn(), events::GIT_PUSHED, id.team.as_deref(), who, id.role.as_str(), payload).map(|_| ())
    });
    // The task's branch is now published, and the checks of the branch that was pushed are watched
    // from here — that is what makes them visible under a policy that asks for no requests at all.
    if let (Some(task), Some(eff)) = (task, &t.eff) {
        let task_branch = eff.task_branch();
        let allowed = eff.allowed_branches();
        let branch_of = |c: &Command2| c.branch().map(str::to_string);
        // The task's own branch first; without requests the pushed branch itself is the delivery.
        let pushed = ok.iter().filter(|c| !c.delete()).find(|c| branch_of(c).is_some() && branch_of(c) == task_branch).or_else(|| {
            matches!(eff.policy.push, Push::Branches | Push::Direct).then(|| ())?;
            ok.iter().filter(|c| !c.delete()).find(|c| branch_of(c).is_some_and(|b| allowed.iter().any(|a| glob_match(a, &b))))
        });
        if let Some(c) = pushed
            && let Ok(Some(row)) = app.with_server(|db| db.task_repo(project, &task, &t.repo.name))
            && matches!(row.state.as_str(), "pending" | "published")
        {
            let _ = app.with_server(|db| {
                let mut d = Delivery {
                    state: Some("published".into()),
                    head_sha: Some(c.new.clone()),
                    branch: Some(row.branch.clone()),
                    ..Default::default()
                };
                if let Some(branch) = c.branch() {
                    watching(&mut d, branch, &c.new);
                    // The pushed branch is the delivery when it is the task's own, or when nothing
                    // was named yet (without requests the agent picks the branch name itself).
                    if task_branch.as_deref() == Some(branch) || row.branch.is_empty() {
                        d.branch = Some(branch.to_string());
                    }
                }
                db.update_delivery(project, &task, &t.repo.name, d).map(|_| ())
            });
        }
    }
}

fn eff_task(t: &Target) -> Option<String> {
    t.eff.as_ref().and_then(|e| e.task.clone())
}
