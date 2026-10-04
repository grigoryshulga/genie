//! The same checks for every provider: what genie asks of a host (identity, repository,
//! open/get/comment/merge of a request, checks) and how the host's failures come back.
//! Run against fake GitHub and GitLab servers (`common/fakehost.rs`) — a new host kind
//! joins by passing this list.

mod common;

use common::fakehost::{self, FakeHost};
use genie::git::hosts;
use genie::git::provider::{Api, ApiError, Ci, CrState, OpenRequest};

fn host(kind: &str, url: &str, token: &str) -> (tempfile::TempDir, hosts::Host) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = serde_json::json!({ "hosts": { "h": { "kind": kind, "url": url } } });
    std::fs::write(dir.path().join("git.json"), cfg.to_string()).unwrap();
    let h = hosts::load(dir.path());
    assert!(h.errors.is_empty(), "{:?}", h.errors);
    // The token is the repository's: what `store::host_of` puts on the host.
    let mut host = h.map["h"].clone();
    host.token = Some(token).filter(|t| !t.is_empty()).map(str::to_string);
    (dir, host)
}

fn req(head: &str, draft: bool) -> OpenRequest {
    OpenRequest { head: head.into(), base: "main".into(), title: format!("Work on {head}"), body: "text".into(), draft }
}

async fn scenario(kind: &'static str) {
    let fake: FakeHost = fakehost::spawn(kind, "secret").await;
    let (_d, h) = host(kind, &fake.url, "secret");
    let api = Api::new(&h).unwrap();

    // Who the token is; the repository and what protects it.
    let who = api.whoami().await.unwrap();
    assert_eq!(who.login, "genie-bot");
    assert_eq!(who.version.is_some(), kind == "gitlab");
    let info = api.repo("acme/api").await.unwrap();
    assert_eq!(info.default_branch, "main");
    assert_eq!(info.can_push, Some(true));
    assert_eq!(info.protected_branches, Some(vec!["main".to_string()]));
    assert!(matches!(api.repo("acme/none").await, Err(ApiError::NotFound(_))), "{kind}");

    // Opening is idempotent: the same branch gives the same request.
    let cr = api.open("acme/api", &req("genie/S-1", false)).await.unwrap();
    assert_eq!((cr.number, cr.state, cr.head.as_str(), cr.base.as_str()), (1, CrState::Open, "genie/S-1", "main"), "{kind}");
    assert!(cr.title.contains("genie/S-1") && !cr.draft && cr.head_sha.is_some());
    let again = api.open("acme/api", &req("genie/S-1", false)).await.unwrap();
    assert_eq!(again.number, 1, "{kind}: a second open finds the first");
    let draft = api.open("acme/api", &req("genie/S-2", true)).await.unwrap();
    assert!(draft.draft, "{kind}");

    // Approvals and reviews.
    let got = api.get("acme/api", 1).await.unwrap();
    assert_eq!((got.approvals, got.changes_requested, got.mergeable), (0, false, Some(true)));
    {
        let mut f = fake.lock();
        f.approvals = 2;
        f.changes_requested = kind == "github";
    }
    let got = api.get("acme/api", 1).await.unwrap();
    assert_eq!(got.approvals, 2, "{kind}");
    assert_eq!(got.changes_requested, kind == "github");
    assert!(matches!(api.get("acme/api", 99).await, Err(ApiError::NotFound(_))));

    // Comments (system notes are not shown).
    api.comment("acme/api", 1, "looks good").await.unwrap();
    let list = api.comments("acme/api", 1).await.unwrap();
    assert!(list.iter().any(|c| c.body == "looks good" && c.author == "genie-bot"), "{kind}: {list:?}");
    assert!(list.iter().all(|c| !c.body.contains("assigned to")), "{kind}: system notes are left out");

    // The checks of one commit (the watched ref's sha, not necessarily a request's head).
    for (state, want) in [("none", Ci::None), ("pending", Ci::Pending), ("passed", Ci::Passed), ("failed", Ci::Failed)] {
        fake.lock().ci = state.into();
        assert_eq!(api.ci("acme/api", got.head_sha.as_deref()).await.unwrap(), want, "{kind}: {state}");
    }
    // A commit the host does not know is not an error: it simply has no checks.
    fake.lock().ci = "passed".into();
    assert_eq!(api.ci("acme/api", None).await.unwrap(), Ci::None, "{kind}");

    // A rerun restarts the failed run of the commit: a GitHub Actions run, a GitLab pipeline. It
    // flips the checks back to running, and a commit with nothing failed has nothing to rerun.
    fake.lock().ci = "failed".into();
    assert_eq!(api.rerun_failed("acme/api", got.head_sha.as_deref()).await.unwrap(), 1, "{kind}");
    assert_eq!(fake.lock().reruns, 1, "{kind}");
    assert_eq!(api.ci("acme/api", got.head_sha.as_deref()).await.unwrap(), Ci::Pending, "{kind}: the restarted run is running");
    fake.lock().ci = "none".into();
    assert!(matches!(api.rerun_failed("acme/api", got.head_sha.as_deref()).await, Err(ApiError::Unsupported(_))), "{kind}");
    assert_eq!(fake.lock().reruns, 1, "{kind}: nothing failed, nothing restarted");
    assert!(matches!(api.rerun_failed("acme/api", None).await, Err(ApiError::Unsupported(_))), "{kind}");

    // A merge the host refuses says why; then it goes through, and the merge commit is named.
    fake.lock().refuse_merge = Some("Pull Request is not mergeable".into());
    assert!(
        matches!(api.merge("acme/api", 1, Some("squash"), got.head_sha.as_deref()).await, Err(ApiError::Rejected(m)) if m.contains("not mergeable")),
        "{kind}"
    );
    assert_eq!(got.merge_sha, None, "an open request has no merge commit yet: {kind}");
    api.merge("acme/api", 1, Some("squash"), got.head_sha.as_deref()).await.unwrap();
    let merged = api.get("acme/api", 1).await.unwrap();
    assert_eq!(merged.state, CrState::Merged, "{kind}");
    assert_eq!(merged.merge_sha.as_deref(), Some("2222222222222222222222222222222222222222"), "{kind}");
    if kind == "gitlab" {
        assert!(matches!(api.merge("acme/api", 2, Some("rebase"), None).await, Err(ApiError::Unsupported(_))));
    }

    // Failures of the host and of the token.
    let (_d2, bad) = host(kind, &fake.url, "wrong");
    assert!(matches!(Api::new(&bad).unwrap().whoami().await, Err(ApiError::Auth(_))), "{kind}");
    fake.lock().rate_limited = 1;
    let before = fake.lock().calls.len();
    api.whoami().await.unwrap();
    assert!(fake.lock().calls.len() >= before + 2, "{kind}: a read is repeated after a rate limit");
    fake.lock().broken = 2;
    api.repo("acme/api").await.unwrap();
    fake.lock().broken = 5;
    assert!(matches!(api.whoami().await, Err(ApiError::Unavailable(_))), "{kind}: three tries, then it gives up");
    fake.lock().broken = 0;
}

#[tokio::test]
async fn github_provider() {
    scenario("github").await;
}

#[tokio::test]
async fn gitlab_provider() {
    scenario("gitlab").await;
}

#[tokio::test]
async fn a_plain_host_has_no_request_api_and_a_host_without_token_cannot_call_it() {
    let (_d, plain) = host("plain", "https://git.example", "");
    assert!(matches!(Api::new(&plain), Err(ApiError::Unsupported(_))));
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("git.json"), r#"{"hosts": {"h": {"kind": "gitlab", "url": "https://git.example"}}}"#).unwrap();
    let h = hosts::load(dir.path()).map["h"].clone();
    assert!(h.token.is_none(), "a host holds no token: its repositories bring their own");
    assert!(matches!(Api::new(&h), Err(ApiError::Auth(_))));
}
