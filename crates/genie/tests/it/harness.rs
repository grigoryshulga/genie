//! What the harness gets from an agent's role, over real processes: the role's
//! skills and the repository's, the MCP config with only the role's connections,
//! the guard's rules — and where one-shot jobs work (`workspace`). The default
//! turn command runs with a recording script in place of pi: it writes down its
//! directory, its environment and its arguments, then reports a result.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use genie::config::{Config, RuntimeConfig};
use genie::state::App;
use genie_core::work::NewJob;
use serde_json::{Value, json};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

struct Live {
    dir: tempfile::TempDir,
    app: Arc<App>,
    repo: PathBuf,
    _stop: tokio::sync::oneshot::Sender<()>,
}

impl Live {
    fn job(&self, role: &str, workspace: &str) -> i64 {
        let job = NewJob {
            project: "shop".into(),
            role: role.into(),
            goal: "Audit the export".into(),
            inputs: json!({}),
            workspace: workspace.into(),
            ..Default::default()
        };
        let id = self.app.with_server(|db| db.create_job(job)).unwrap().id;
        self.app.wake_runtime.notify_one();
        id
    }

    async fn finished(&self, id: i64) -> genie_core::work::Job {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(60) {
            let j = self.app.with_server(|db| db.job(id)).unwrap();
            if j.status == "succeeded" || j.status == "failed" {
                return j;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let turns = self.app.with_server(|db| db.turns("shop", None, 20)).unwrap();
        panic!("job {id} did not finish: {:?}", turns.iter().map(|t| (&t.agent, &t.status, &t.error, &t.log)).collect::<Vec<_>>());
    }

    /// What the stand-in harness recorded for a job: its directory, rules, MCP mode,
    /// the MCP secrets in its environment and its arguments.
    fn recorded(&self, id: i64) -> Recorded {
        let text = std::fs::read_to_string(self.dir.path().join("rec").join(format!("job-{id}"))).unwrap();
        let mut lines = text.lines();
        let cwd = PathBuf::from(lines.next().unwrap());
        let policy = lines.next().unwrap().strip_prefix("policy=").unwrap().to_string();
        let mcp_mode = lines.next().unwrap().strip_prefix("mcpmode=").unwrap().to_string();
        let secrets = lines.next().unwrap().strip_prefix("secrets=").unwrap().to_string();
        let rest: Vec<&str> = lines.collect();
        let args: Vec<String> = rest.join("\n").split("\narg=").map(|a| a.trim_start_matches("arg=").to_string()).collect();
        Recorded { cwd, policy: PathBuf::from(policy), mcp_mode, secrets, args }
    }
}

struct Recorded {
    cwd: PathBuf,
    policy: PathBuf,
    mcp_mode: String,
    /// `<docs token>,<wiki token>` as the agent's environment has them.
    secrets: String,
    args: Vec<String>,
}

impl Recorded {
    /// The values of a repeated flag.
    fn values(&self, flag: &str) -> Vec<&str> {
        self.args.windows(2).filter(|w| w[0] == flag).map(|w| w[1].as_str()).collect()
    }
    fn has(&self, flag: &str) -> bool {
        self.args.iter().any(|a| a == flag)
    }
}

/// `adapter`: pi's settings list pi-mcp-adapter (as `pi install npm:pi-mcp-adapter` leaves them).
async fn live(max_attempts: u32, adapter: bool) -> Live {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path();

    // The project's repository, with a skill of its own.
    let repo = data.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.email", "genie@example.com"]);
    git(&repo, &["config", "user.name", "genie"]);
    write(
        &repo.join(".agents/skills/house-style/SKILL.md"),
        "---\nname: house-style\ndescription: How this repository writes code.\n---\nx\n",
    );
    write(&repo.join("README.md"), "shop\n");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);

    // The server's library: a role, a skill, MCP connections — one behind the gateway,
    // one handed to the harness (`gateway: false`) — whose secrets the server has.
    static SECRETS: std::sync::Once = std::sync::Once::new();
    // SAFETY: set before the servers of this binary start, always to the same values.
    SECRETS.call_once(|| unsafe {
        std::env::set_var("GENIE_TEST_DOCS_TOKEN", "d0cs");
        std::env::set_var("GENIE_TEST_WIKI_TOKEN", "w1ki");
    });
    write(
        &data.join("agents/auditor.md"),
        "---\ntitle: Аудитор\ndescription: Audits changes.\nbase: reviewer\nfiles: write\ndenyCommands: [\"git push*\"]\nskills: [owasp, not-installed]\nmcp: [\"docs:search_*\", wiki]\n---\nYou audit.\n",
    );
    write(
        &data.join("agents/advisor.md"),
        "---\ntitle: Советник\ndescription: Answers questions.\nbase: analyst\nfiles: none\n---\nYou advise.\n",
    );
    write(&data.join("skills/owasp/SKILL.md"), "---\nname: owasp\ndescription: OWASP checks.\n---\nx\n");
    write(&data.join("skills/other/SKILL.md"), "---\nname: other\ndescription: Not for auditors.\n---\nx\n");
    write(
        &data.join("mcp.json"),
        &json!({ "mcpServers": {
            "docs": { "description": "Search the docs", "command": "docs-mcp", "args": ["--home", "${env:HOME}"], "env": { "DOCS_TOKEN": "${env:GENIE_TEST_DOCS_TOKEN}" } },
            "wiki": { "command": "wiki-mcp", "args": ["--home", "${env:HOME}"], "env": { "WIKI_TOKEN": "${env:GENIE_TEST_WIKI_TOKEN}" }, "gateway": false },
            "secret": { "url": "https://secret.example/mcp", "headers": { "Authorization": "Bearer ${env:HOME}" } }
        }})
        .to_string(),
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = Config::load(data).unwrap();
    // Test files live in the data directory, which a sandboxed agent does not see.
    cfg.runtime.sandbox.mode = "off".into();
    cfg.port = listener.local_addr().unwrap().port();
    std::fs::create_dir_all(data.join("rec")).unwrap();
    let script = r#"out="$GENIE_REC/$GENIE_AGENT_NAME"; { pwd -P; echo "policy=${GENIE_POLICY:-}"; echo "mcpmode=${PI_MCP_CONFIG_MODE:-}"; echo "secrets=${GENIE_TEST_DOCS_TOKEN-unset},${GENIE_TEST_WIKI_TOKEN-unset}"; printf 'arg=%s\n' "$@"; } > "$out"; "$GENIE_BIN" agent output '{"summary":"ok"}'"#;
    let mut command = RuntimeConfig::default().command;
    command[0] = vec!["bash".into(), "-c".into(), script.into(), "harness".into()];
    cfg.runtime.command = command;
    cfg.runtime.mode = "turns".into();
    cfg.runtime.max_attempts = max_attempts;
    cfg.runtime.env.insert("GENIE_BIN".into(), env!("CARGO_BIN_EXE_genie").into());
    cfg.runtime.env.insert("GENIE_REC".into(), data.join("rec").to_string_lossy().into_owned());
    let pi_dir = data.join("pi-agent");
    let packages = if adapter { json!(["npm:pi-mcp-adapter"]) } else { json!([]) };
    write(&pi_dir.join("settings.json"), &json!({ "packages": packages }).to_string());
    cfg.runtime.env.insert("PI_CODING_AGENT_DIR".into(), pi_dir.to_string_lossy().into_owned());
    let app = App::open(data, cfg, PathBuf::from("/nonexistent")).unwrap();
    app.create_project("shop", "Shop", Some(&repo.to_string_lossy()), None, Some("SHOP")).unwrap();
    app.with_server(|db| db.set_autonomy("shop", "manual")).unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let a = app.clone();
    tokio::spawn(async move {
        genie::serve_on(a, listener, async {
            let _ = rx.await;
        })
        .await
        .unwrap();
    });
    genie::runtime::start(&app);
    let repo = repo.canonicalize().unwrap();
    Live { dir, app, repo, _stop: tx }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_harness_gets_the_roles_skills_mcp_and_rules_and_jobs_work_where_their_workspace_says() {
    let l = live(3, true).await;
    let data = l.dir.path().canonicalize().unwrap();
    let home = std::env::var("HOME").unwrap_or_default();

    // A read-only job works in the repository without edit and write.
    let id = l.job("auditor", "read-only");
    let j = l.finished(id).await;
    assert_eq!(j.status, "succeeded", "{:?}", j.error);
    let r = l.recorded(id);
    assert_eq!(r.cwd, l.repo);
    assert_eq!(r.values("--exclude-tools"), vec!["edit,write"]);
    // Only the role's skills (the missing one is skipped), then the repository's.
    assert!(r.has("--no-skills"));
    assert_eq!(r.values("--skill"), vec![data.join("skills/owasp").to_string_lossy(), l.repo.join(".agents/skills").to_string_lossy()]);
    // The guard is loaded and finds its rules; the MCP config is the only one pi-mcp-adapter reads.
    assert_eq!(r.values("-e"), vec![data.join("runtime/genie-guard.ts").to_string_lossy()]);
    assert_eq!(std::fs::read_to_string(data.join("runtime/genie-guard.ts")).unwrap(), genie::sessions::GUARD);
    assert_eq!(r.mcp_mode, "exclusive");
    let policy: Value = serde_json::from_str(&std::fs::read_to_string(&r.policy).unwrap()).unwrap();
    assert_eq!(
        policy,
        json!({ "role": "auditor", "files": "read", "denyCommands": ["git push*"], "mcp": { "docs": ["search_*"], "wiki": null } }),
        "a read-only workspace narrows the role's `files`"
    );
    let mcp_file = PathBuf::from(r.values("--mcp-config")[0]);
    let mcp: Value = serde_json::from_str(&std::fs::read_to_string(&mcp_file).unwrap()).unwrap();
    let gateway = format!("http://127.0.0.1:{}/api/mcp-gateway/docs", l.app.cfg.port);
    assert_eq!(
        mcp,
        json!({ "mcpServers": {
            "docs": { "url": gateway, "headers": { "Authorization": "Bearer ${GENIE_TOKEN}" } },
            "wiki": { "command": "wiki-mcp", "args": ["--home", home], "env": { "WIKI_TOKEN": "w1ki" } }
        }}),
        "only granted connections: through the gateway its address and the agent's token, no secrets; \
         handed over directly, the entry with its secrets resolved and genie's own fields dropped"
    );
    assert_eq!(r.secrets, "unset,w1ki", "the gateway's secrets stay with the server; a direct connection's the agent needs");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&mcp_file).unwrap().permissions().mode() & 0o777, 0o600, "the MCP config holds secrets");
    }
    let message = r.args.last().unwrap();
    assert!(message.contains("## Workspace") && message.contains("read-only"), "{message}");

    // A worktree job gets its own worktree on its own branch and may write.
    let id = l.job("auditor", "worktree");
    let j = l.finished(id).await;
    assert_eq!(j.status, "succeeded", "{:?}", j.error);
    let r = l.recorded(id);
    let worktree = l.repo.parent().unwrap().join(format!("repo.worktrees/job-{id}"));
    assert_eq!(r.cwd, worktree);
    assert_eq!(git(&worktree, &["branch", "--show-current"]), format!("genie/job-{id}"));
    assert!(!r.has("--exclude-tools"));
    assert!(r.values("--skill").contains(&worktree.join(".agents/skills").to_string_lossy().as_ref()), "{:?}", r.args);
    let policy: Value = serde_json::from_str(&std::fs::read_to_string(&r.policy).unwrap()).unwrap();
    assert_eq!(policy["files"], "write");
    assert!(r.args.last().unwrap().contains(&format!("genie/job-{id}")));

    // A scratch job works in an empty directory of its own: no repository skills there.
    let id = l.job("auditor", "scratch");
    let j = l.finished(id).await;
    assert_eq!(j.status, "succeeded", "{:?}", j.error);
    let r = l.recorded(id);
    assert_eq!(r.cwd, data.join(format!("workspaces/shop/job-{id}")));
    assert_eq!(r.values("--skill"), vec![data.join("skills/owasp").to_string_lossy()]);

    // A role that does not work with files gets an empty directory whatever the workspace.
    let id = l.job("advisor", "read-only");
    assert_eq!(l.finished(id).await.status, "succeeded");
    let r = l.recorded(id);
    assert_eq!(r.cwd, data.join(format!("workspaces/shop/job-{id}")));
    assert_eq!(r.values("--exclude-tools"), vec!["edit,write"]);

    // A role without a skill list gets the harness's own skills (no --no-skills) and the
    // repository's; without MCP grants its MCP config is empty.
    let id = l.job("analyst", "read-only");
    assert_eq!(l.finished(id).await.status, "succeeded");
    let r = l.recorded(id);
    assert!(!r.has("--no-skills"));
    assert_eq!(r.values("--skill"), vec![l.repo.join(".agents/skills").to_string_lossy()]);
    let mcp: Value = serde_json::from_str(&std::fs::read_to_string(r.values("--mcp-config")[0]).unwrap()).unwrap();
    assert_eq!(mcp, json!({ "mcpServers": {} }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_pi_mcp_adapter_no_mcp_config_and_a_job_that_cannot_start_fails() {
    let l = live(1, false).await;
    // pi refuses --mcp-config without the adapter: the flag and the file (secrets) are left out.
    let id = l.job("auditor", "scratch");
    assert_eq!(l.finished(id).await.status, "succeeded");
    let r = l.recorded(id);
    assert!(!r.has("--mcp-config"), "{:?}", r.args);
    assert_eq!(r.mcp_mode, "");
    assert!(!l.dir.path().join(format!("runtime/shop/job_{id}/mcp.json")).exists());
    assert!(r.policy.exists(), "the guard's rules are there all the same");

    // The role was removed from the configuration after the job was queued.
    let id = l.job("ghost", "none");
    let j = l.finished(id).await;
    assert_eq!(j.status, "failed");
    assert!(j.error.as_deref().unwrap_or_default().contains("role ghost is no longer in the agent configuration"), "{:?}", j.error);
    let turns = l.app.with_server(|db| db.turns("shop", Some(&format!("job/{id}")), 10)).unwrap();
    assert!(turns.iter().all(|t| t.status == "failed"), "{:?}", turns.iter().map(|t| &t.status).collect::<Vec<_>>());
}
