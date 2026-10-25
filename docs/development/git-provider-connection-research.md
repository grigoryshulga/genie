# Git provider connection feasibility (GitHub and GitLab)

Researched against first-party documentation on 2026-10-25, with the repository state of this worktree (branch `feat/github-gitlab-integration`, base commit `b7fd17c`). This note separates what the providers document from what Genie would have to build; it does not claim any call, installation or login against live GitHub or GitLab credentials, and no API request was sent to either provider. Provider pages were read through a page-extraction tool, so exact limits, status codes and permission names should be re-read on the linked page before they become code constants. Claims that no primary source confirmed are marked **(not confirmed)**.

## Recommendation

Goal: a user connects a GitHub or GitLab account, selects repositories, and grants them to a Genie project, while the server keeps every credential and agents never see one (the model already in [git-repositories.md](../platform/git-repositories.md)).

- **GitHub: a GitHub App, acting with installation access tokens.** The installer chooses "All repositories" or "Only select repositories", and GitHub documents that choice as the repository-selection mechanism. Installation tokens last one hour, can be narrowed per request to named repositories and a subset of permissions, and do not depend on the installing person staying in the organisation. A user access token is used only during connection, to prove who installed the app and which installations they can see. OAuth Apps and fine-grained PATs are worse fits (see GitHub below).
- **GitLab: OAuth 2.0 authorization code with PKCE (`S256`), scopes `api write_repository`, tokens kept server-side and refreshed.** GitLab has no per-repository grant in OAuth: the token reaches everything the user can reach, so Genie must enforce the repository grant itself. Project or group access tokens give real per-project scoping and a bot identity, but they cannot be created from an OAuth token, so they stay a manual (paste) option, like today's PATs.
- **Self-Managed GitLab: defer the OAuth flow.** Each instance needs its own registered application. The existing pasted-token path keeps working for `git.json` hosts with a custom `url`.
- **Ownership:** a connection belongs to the Genie user who completed it; a project holds a separate grant naming the connection and the selected repositories. Only project owners/admins connect and grant.
- **Writes stay server-side and policy-bound:** merge only by explicit project policy (already modelled); no force push and no branch deletion through a connected provider.
- **Webhooks are an accelerator, not a dependency.** The delivery watcher already polls; Genie often runs on loopback (`bind` defaults to `127.0.0.1` in `crates/genie/src/config.rs`), where providers cannot reach it.

## GitHub

### App type: GitHub App, OAuth App or fine-grained PAT

- GitHub states a general preference: "In general, GitHub Apps are preferred over OAuth apps." The one documented exception is enterprise-level resources, which a GitHub App cannot yet be given permissions against. [Differences between GitHub Apps and OAuth apps](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/differences-between-github-apps-and-oauth-apps)
- GitHub Apps use fine-grained permissions and the installer decides which repositories the app can reach; an OAuth App can reach every repository the authorizing user can, and its tokens last until revoked unless expiry is opted into. [Deciding when to build a GitHub App](https://docs.github.com/en/apps/creating-github-apps/about-creating-github-apps/deciding-when-to-build-a-github-app)
- The OAuth `repo` scope "grants full access to public and private repositories including read and write access to code"; there is no per-repository scope. [Scopes for OAuth apps](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/scopes-for-oauth-apps)
- Fine-grained PATs can be limited to chosen repositories and expire, but they are created by hand by the user, are limited to one resource owner, may need organisation-owner approval, and the page lists the Checks API among current gaps. Genie's watcher reads check runs (`provider/github.rs`, `ci`), so a fine-grained PAT may not work for CI display. GitHub advises a GitHub App for organisation-level or long-lived integrations. [Managing personal access tokens](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens)
- OAuth Apps "can't use granular permissions", share the user's 5,000 requests/hour, and need webhooks configured per repository; GitHub Apps get app-level webhooks and installation rate limits that scale. [Differences](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/differences-between-github-apps-and-oauth-apps)

### Installation flow and repository selection

- Installing an app that needs repository permissions offers **All repositories** or **Only select repositories**. Repositories the app itself creates are granted automatically. [Installing your own GitHub App](https://docs.github.com/en/apps/using-github-apps/installing-your-own-github-app)
- The account owner can change repository access later under the app's "Configure" page, and can suspend or uninstall it there. [Reviewing and modifying installed GitHub Apps](https://docs.github.com/en/apps/using-github-apps/reviewing-and-modifying-installed-github-apps)
- Install URL: `https://github.com/apps/<app-name>/installations/new`. An optional `state` query value is preserved across installation and can link the installation to a user. [Sharing your GitHub App](https://docs.github.com/en/apps/sharing-github-apps/sharing-your-github-app)
- After installation GitHub redirects to the app's **Setup URL** with an `installation_id`. GitHub warns that "bad actors can hit this URL with a spoofed `installation_id`" and says to generate a user access token for the installer and check that the installation is associated with that user. Genie must not trust the query value alone. [About the setup URL](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/about-the-setup-url)
- "Request user authorization (OAuth) during installation" is an app setting; when enabled, no Setup URL can be entered. Whether the callback then receives the installation ID **(not confirmed)**. [Registering a GitHub App](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/registering-a-github-app)
- Who may install on an organisation (owner, or request-to-install by members) is not stated on the pages read **(not confirmed)**.
- The app is registered per operator: owner account, callback URL(s) (up to ten), webhook secret, "Only on this account" or "Any account". A manifest flow can create the registration and return the App ID, private key, client secret and webhook secret; the code must be exchanged within one hour. This matches the existing "operator registers the provider application" stance in [social-auth-research.md](social-auth-research.md). [Registering a GitHub App](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/registering-a-github-app), [App manifest flow](https://docs.github.com/en/apps/creating-github-apps/setting-up-a-github-app/creating-a-github-app-from-a-manifest)
- Whether one Genie-operated app could serve all self-hosted Genie deployments is a product decision, not a GitHub limitation; this note assumes each deployment registers its own app.

### Which token does the work

- Installation token: acts as the app, limited by the app's permissions. User access token: limited to what both the app and the user can do, and attributed to the user. GitHub also says an app that acts for a user "should always use a user access token", while apps may "act independently of a user". [About creating GitHub Apps](https://docs.github.com/en/apps/creating-github-apps/about-creating-github-apps/about-creating-github-apps), [Best practices](https://docs.github.com/en/apps/creating-github-apps/about-creating-github-apps/best-practices-for-creating-a-github-app)
- **Recommendation:** installation token for everything Genie does in the background (mirror fetch, push, PR, checks, merge). Genie's watcher and agents run when no user is present, and user tokens expire after eight hours. Use the user token only to verify the installer and list installations/repositories.
- Git over HTTPS needs the Contents permission and uses `x-access-token` as the user name; the installation token expires after one hour. The existing code already uses that user name (`Host::basic_user` in `crates/genie/src/git/hosts.rs`). [Authenticating as an installation](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/authenticating-as-a-github-app-installation)
- Minting: `POST /app/installations/{id}/access_tokens`, authenticated with an app JWT. `repositories` or `repository_ids` (up to 500) narrow the repositories and `permissions` narrows permissions; omitted means everything the installation holds. A token cannot exceed the installation's repositories or the app's permissions. [Generating an installation access token](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-an-installation-access-token-for-a-github-app)
- So the answer to "can tokens be scoped per request" is yes, by repository and by permission. Genie should mint a token for exactly the repository of the operation, cache it until shortly before expiry, and never persist it.
- Best practice for stored credentials: keep the private key in a vault or secret store, encrypt tokens on the back end, keep refresh tokens apart from access tokens. [Best practices](https://docs.github.com/en/apps/creating-github-apps/about-creating-github-apps/best-practices-for-creating-a-github-app)

### User authorization (identity and listing only)

- Web flow at `https://github.com/login/oauth/authorize` with `state`, `code_challenge` and `code_challenge_method=S256` (`plain` is not supported); exchange at `https://github.com/login/oauth/access_token` with `code_verifier`. The returned `scope` is always empty because GitHub App user tokens are bounded by app and user permissions. [Generating a user access token](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app)
- Lifetime: access token eight hours (`ghu_`), refresh token six months (`ghr_`). Each refresh returns a new refresh token and invalidates the old pair, so the newest must be stored atomically. Expiry is on by default and GitHub recommends keeping it. [Refreshing user access tokens](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/refreshing-user-access-tokens)
- Because Genie needs the user token only at connection time (and optionally for periodic re-verification), it can discard it and avoid storing a refresh token at all, as `social_auth.rs` does for login.

### Listing repositories

- `GET /user/installations` lists installations the user can access; `GET /user/installations/{installation_id}/repositories` lists repositories of one installation visible to the user (user token). `GET /installation/repositories` lists what an installation token can reach. All paginate with `per_page` up to 100. [Installations API](https://docs.github.com/en/rest/apps/installations)
- `GET /app/installations` (app JWT) lists the app's installations and returns permissions, `repository_selection` and `suspended_*` fields. [Apps API](https://docs.github.com/en/rest/apps/apps)
- Genie's picker should call the user-scoped endpoints, so a user sees only repositories both they and the app can reach, and re-check with `GET /user/installations` regularly (GitHub's own recommendation).

### Permissions Genie needs

Repository permissions, from GitHub's per-endpoint table. [Permissions required for GitHub Apps](https://docs.github.com/en/rest/overview/permissions-required-for-github-apps)

| Need (Genie operation) | Permission | Level |
| --- | --- | --- |
| Repository facts (`GET /repos/...`) | Metadata | read |
| Mirror fetch and task-branch push | Contents | write |
| Open a PR, list PRs, comment, read reviews | Pull requests | write |
| Merge a PR (table lists Contents for the merge endpoint) | Contents (plus Pull requests) | write |
| Combined commit status | Commit statuses | read |
| Check runs for a ref | Checks | read |
| Rerun failed jobs (`rerun_failed`) and read workflow runs | Actions | write (read suffices for display) |
| Read branch protection for `repo check` | Administration | read (optional) |
| Push changes to `.github/workflows` | Workflows | write |

- Recommendation: do not request Workflows or Administration at all in the first version. Without Workflows, a push that edits workflow files is refused by GitHub, which is a useful guard for agent work; without Administration, the existing code already degrades to "protected branches unknown" (`provider/github.rs`, `repo`). Request Actions write only if `genie pr rerun` is kept for GitHub. "Workflows" is needed to edit workflow files per [Choosing permissions](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/choosing-permissions-for-a-github-app).
- Missing permissions return 403 and the `X-Accepted-GitHub-Permissions` header names what the endpoint needs (same page). That makes a "permission missing" health state cheap to detect.
- Changing app permissions later requires each installing owner to approve; until then "their installation will continue to use the old permissions" (same page). Genie should read `permissions` from the installation record rather than assume them.
- Metadata read appears in the table for repository facts; whether it is mandatory for every app is not stated on the pages read **(not confirmed)**.

### Webhooks: changes and revocation

- `installation` (`created`, `deleted`, `suspend`, `unsuspend`, `new_permissions_accepted`), `installation_repositories` (`added`, `removed`, with `repository_selection`) and `github_app_authorization` (a user revoked authorization) are delivered to the app by default and cannot be unsubscribed. Revoking authorization "does not uninstall the GitHub App". [Webhook events and payloads](https://docs.github.com/en/webhooks/webhook-events-and-payloads)
- `check_run` and `check_suite` need Checks read; they can replace polling of check runs. (same page)
- Verify the signature of every delivery against the webhook secret. [Best practices](https://docs.github.com/en/apps/creating-github-apps/about-creating-github-apps/best-practices-for-creating-a-github-app)
- Delivery needs a reachable HTTPS URL. For a loopback deployment, rely on polling plus a periodic `GET /app/installations`; the doc recommends webhooks over polling for rate limits, not as the only mechanism.

### Suspension, uninstall, revocation

- A suspended installation is blocked from the account's resources and webhook events; an uninstall removes access in that account; GitHub recommends suspending for a temporary cut-off. `GET /app/installations/{id}` exposes `suspended_by` and `suspended_at`. [Apps API](https://docs.github.com/en/rest/apps/apps), [Reviewing and modifying installed apps](https://docs.github.com/en/apps/using-github-apps/reviewing-and-modifying-installed-github-apps)
- The app keeps working if the installer leaves the organisation. [Differences](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/differences-between-github-apps-and-oauth-apps)
- Exact error codes returned by token minting for a deleted or suspended installation are not stated on the pages read **(not confirmed)**; treat any 4xx from minting as "connection unhealthy" and record the body.
- A user's authorization is separate from installation: de-authorizing deactivates that user's tokens but not the installation. [Reviewing and modifying installed apps](https://docs.github.com/en/apps/using-github-apps/reviewing-and-modifying-installed-github-apps)

### Rate limits

- Installation tokens: 5,000/hour baseline, +50/hour per repository and per organisation user beyond 20, capped at 12,500; 15,000 on Enterprise Cloud organisations. User tokens share the user's 5,000/hour. [Rate limits for the REST API](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api)
- Secondary limits include 100 concurrent requests, 900 points/minute per endpoint (5 points for writes) and roughly 80 content-creating requests/minute. Respect `retry-after` and `x-ratelimit-reset`. (same page) The existing client honours `retry-after` and `x-ratelimit-remaining` for reads (`provider/mod.rs`, `Api::send`).

### Branch protection, required checks, merge

- Protected branches can require status checks (matched by name and source) and approving reviews; by default admins are not bound; push can be restricted to named users, teams or apps; force pushes and deletion are blocked by default on protected branches. [About protected branches](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches)
- Whether an installation token is bound by these rules without extra configuration is not stated outright **(not confirmed)**; plan for "bound unless the owner adds the app to a bypass list", and keep Genie's own check (`mergeable`, `approvals`, `require_ci` in `delivery.rs::do_merge`) as a second layer.
- `PUT /pulls/{n}/merge` takes `merge_method` (`merge`, `squash`, `rebase`) and a `sha` that the head must match: 405 means the merge cannot be performed, 409 means `sha` did not match, 422 is validation. GitHub recommends the asynchronous merge API for merge queues, which the endpoint does not support. [Pull requests API](https://docs.github.com/en/rest/pulls/pulls)
- Check runs can be written only by GitHub Apps; reading them works for apps and users. [Using the REST API with checks](https://docs.github.com/en/rest/guides/using-the-rest-api-to-interact-with-checks)

### Idempotent PR creation

- Listing PRs accepts `head` in `user:ref-name` or `organization:ref-name` form. [Pull requests API](https://docs.github.com/en/rest/pulls/pulls)
- The REST reference for creating a PR documents only a generic 422 ("Validation failed, or the endpoint has been spammed"); an "already exists" error for a duplicate head is **not documented** there. Genie's current code relies on matching the substring "already exists" in a rejected response (`provider/github.rs`, `open`), which is therefore an observed behaviour, not a documented contract.
- Recommendation: list open PRs for `owner:head` first and create only when none exists; on any 422 after create, list again before reporting failure.

## GitLab

### Credential types

- **OAuth 2.0 (authorization code with PKCE).** `code_challenge` with `S256` at `/oauth/authorize`, then `code_verifier` at `/oauth/token`; GitLab recommends PKCE for client and server apps but does not state it is mandatory. Without PKCE the client secret goes in the request or a Basic header, never both (400). [OAuth 2.0 API](https://docs.gitlab.com/api/oauth2/), [RFC 7636](https://datatracker.ietf.org/doc/html/rfc7636)
- **Applications** are the OAuth client registrations: user-owned, group-owned, or instance-wide (Self-Managed admins; can be "trusted" to skip the consent step). Each is registered on one instance and yields an Application ID and a secret that can be renewed (renewing breaks the old one). [Configure GitLab as an OAuth provider](https://docs.gitlab.com/integration/oauth_provider/)
- **Personal access tokens** are tied to one user; by default expiry is at most 365 days and administrators can change the maximum on 17.6 and later. [Personal access tokens](https://docs.gitlab.com/user/profile/personal_access_tokens/)
- **Project and group access tokens** create a non-removable bot user; creation needs Maintainer or Owner; role and scopes are chosen per token; default expiry is 365 days. Available on every Self-Managed licence, but on GitLab.com they require Premium or Ultimate. Bot users do not consume licence seats. [Project access tokens](https://docs.gitlab.com/user/project/settings/project_access_tokens/)
- The create endpoint states "You must use a personal access token with this endpoint", so an OAuth token cannot mint project tokens. [Project access tokens API](https://docs.gitlab.com/api/project_access_tokens/)

### Scopes and how coarse they are

- `api` is "complete read/write access to the API, including all groups and projects"; `read_api` read-only; `read_repository` read via Git-over-HTTP or the Repository Files API; `write_repository` read-write via Git-over-HTTP. There is no per-project scope in OAuth. [OAuth provider scopes](https://docs.gitlab.com/integration/oauth_provider/)
- For project and group tokens, `write_repository` "does not support API authentication", so opening a merge request needs `api`. By the same logic an OAuth grant that must open and merge MRs has to ask for `api` as well as being able to push (`api` for the API calls; whether `api` alone also authorizes Git-over-HTTP push is **not confirmed** from the pages read, so request `api write_repository`). [Access token scopes](https://docs.gitlab.com/security/tokens/access_token_scopes/)
- Consequence: the stored OAuth token can touch every project the user can. The repository grant must therefore be enforced inside Genie, at the point where `host_of` builds the host for an operation (`crates/genie/src/git/store.rs`), by resolving only the project IDs granted to that Genie project.
- Per-project isolation is possible only with project or group access tokens (role-limited bot); Genie cannot automate their creation from OAuth.
- Git over HTTPS accepts the OAuth token as the password with user name `oauth2` per GitLab's OAuth documentation (read through a search extract; confirm the page wording). [OAuth 2.0 API](https://docs.gitlab.com/api/oauth2/), [Clone a repository](https://docs.gitlab.com/topics/git/clone/)

### Token lifetime, refresh, revocation

- Example responses show `expires_in: 7200`; the administrator can set the OAuth access-token lifetime (default 7,200 s, minimum 300 s) on Self-Managed. [OAuth 2.0 API](https://docs.gitlab.com/api/oauth2/), [Account and limit settings](https://docs.gitlab.com/administration/settings/account_and_limit_settings/)
- A refresh invalidates the existing access token and refresh token and returns new ones; refresh tokens work after the access token has expired. Persist the new pair atomically before using it. [OAuth 2.0 API](https://docs.gitlab.com/api/oauth2/)
- Refresh-token lifetime: not stated on the page read **(not confirmed)**.
- Revoke with `POST /oauth/revoke` (`client_id`, `client_secret`, `token`); inspect with `GET /oauth/token/info`. [OAuth 2.0 API](https://docs.gitlab.com/api/oauth2/)
- A missing, invalid, expired or revoked token gives 401. [REST API troubleshooting](https://docs.gitlab.com/api/rest/troubleshooting/)
- No GitLab webhook event for OAuth revocation appears in the event list I could read (push, merge request, pipeline, job, member and others; the `Resource Access Token Hook` fires before a project or group access token expires, seven days by default). Treat 401 on use or refresh as the revocation signal. [Webhook events](https://docs.gitlab.com/user/project/integrations/webhook_events/) (the middle of the list was truncated; **not fully confirmed**)

### Listing projects

- `GET /projects?membership=true&min_access_level=30` returns projects where the user is at least Developer (10 Guest, 20 Reporter, 30 Developer, 40 Maintainer, 50 Owner); `simple=true` trims fields; offset pagination is limited to 50,000 projects, keyset beyond. Responses carry `permissions.project_access` and `permissions.group_access`, which the existing `gitlab::repo` already reads for `can_push`. [Projects API](https://docs.gitlab.com/api/projects/)

### Protected branches, approvals, pipelines

- The default branch is protected by default; "Allowed to push and merge" lets listed roles merge via MR without "Allowed to merge"; force push is a separate toggle; protected branches cannot be deleted with local Git, only through UI/API by Maintainers or Owners. [Protected branches](https://docs.gitlab.com/user/project/repository/branches/protected/)
- "Pipelines must succeed" requires the existence of a successful pipeline; skipped pipelines block by default. Required approvals and unresolved threads also block merge. [Auto-merge and merge checks](https://docs.gitlab.com/user/project/merge_requests/auto_merge/)
- Merge: `PUT /projects/:id/merge_requests/:iid/merge` accepts `sha`, `squash`, `auto_merge` (`merge_when_pipeline_succeeds` is deprecated since 17.11) and `should_remove_source_branch`. 401 means no permission, 405 cannot merge, 409 `sha` mismatch, 422 failed. `source_branch`, `target_branch` and `state` filter the list endpoint. [Merge requests API](https://docs.gitlab.com/api/merge_requests/)
- The existing `gitlab::merge` sends `sha` and `squash` but not `auto_merge`, which is right for Genie's explicit-policy model; it also leaves `should_remove_source_branch` unset, which fits the no-deletion rule.

### Idempotent MR creation

- Create requires `source_branch`, `target_branch` and `title`. The reference does **not** document a duplicate-MR response; the general meaning of 409 is "a conflicting resource already exists". The 409 in the owner's brief is therefore a plausible duplicate response, not a documented one for this endpoint. [Merge requests API](https://docs.gitlab.com/api/merge_requests/), [REST troubleshooting](https://docs.gitlab.com/api/rest/troubleshooting/)
- The current code matches the message text "already exists" (`provider/gitlab.rs`, `open`; the unit test uses "Another open merge request already exists"). Same recommendation as GitHub: list `state=opened&source_branch=` first, create only if empty.

### Rate limits and webhooks

- GitLab.com: 2,000 authenticated API requests/minute per user in effect today; plan-based lower burst limits are announced but not yet in effect. `RateLimit-*` headers are on all responses, `Retry-After` on 429. [GitLab.com rate limits](https://docs.gitlab.com/user/gitlab_com/rate_limits/)
- Webhooks: project webhooks exist on all tiers; group webhooks need Premium. A signing token (HMAC-SHA256) is preferred over the plain `X-Gitlab-Token` secret; webhooks are disabled after four consecutive failures (temporary) and permanently after 40. [Webhooks](https://docs.gitlab.com/user/project/integrations/webhooks/)
- Creating webhooks needs Maintainer (per Vercel's integration docs; GitLab's own statement **not confirmed**). OAuth gives no per-repository webhook automation without that role, so polling remains the baseline.

### GitLab.com versus Self-Managed

- An OAuth application is registered per instance and its endpoints depend on the instance URL; Genie needs one registration per instance, so a global "GitLab" button cannot cover custom hosts. [OAuth provider](https://docs.gitlab.com/integration/oauth_provider/)
- Administrators can disable user-owned OAuth applications (group-owned are unaffected), change the token lifetime and cap access-token lifetimes; dynamic client registration can be turned off. [Account and limit settings](https://docs.gitlab.com/administration/settings/account_and_limit_settings/)
- Version-dependent behaviour already seen: `auto_merge` replaced `merge_when_pipeline_succeeds` in 17.11; `detailed_merge_status` supersedes `merge_status` (the code reads both, `provider/gitlab.rs::parse`); the maximum PAT lifetime is configurable from 17.6. The release that introduced PKCE is **not confirmed**. The current code handles missing approvals endpoints on older or Free editions (404/403 means zero approvals).
- Self-Managed rate limits are administrator-set; not researched.
- **First version:** say plainly that GitLab OAuth connection is offered for GitLab.com only, and that Self-Managed instances keep the pasted-token flow, with a documented follow-up for a per-instance OAuth application (operator pastes Application ID and secret per `git.json` host, and the host needs a reachable `redirect_uri`). `ca_cert`/`http_proxy` host fields already exist for private CAs.

## Vercel reference

Used only as a UX reference, from Vercel's own docs.

- GitHub: the user installs the Vercel GitHub App; the import screen lists repositories from the connected providers, with a namespace dropdown and search. Repository scope is the App installation's all-or-selected choice, and a missing repository is fixed by "Configure GitHub App" from the import screen, then returning and searching again. [Deploying Git repositories](https://vercel.com/docs/git), [Missing GitHub repository guide](https://vercel.com/kb/guide/unable-to-find-github-repository)
- Per-project binding is separate from the account connection: a Vercel project is connected to, changed to or disconnected from one repository in Project Settings. [Vercel for GitHub](https://vercel.com/docs/git/vercel-for-github)
- Role gating is explicit: for a personal GitHub account the repository Owner; for organisations an Owner or a Member with repository access; outside collaborators cannot connect. [Vercel for GitHub](https://vercel.com/docs/git/vercel-for-github)
- Vercel publishes the permission table with a reason per permission (Contents, Pull requests, Checks, Commit statuses, Metadata, Actions read, Workflows and others). Genie should show the same kind of table at connection time. [Vercel for GitHub](https://vercel.com/docs/git/vercel-for-github)
- GitLab: Vercel asks for the `api` scope, and requires Maintainer on the project (and on its group) so it can configure a webhook. [Vercel for GitLab](https://vercel.com/docs/git/vercel-for-gitlab) This confirms that GitLab's connection is coarse even for an established product.
- Self-Managed GitLab and GitHub Enterprise Server are served by running Vercel CLI inside the customer's pipelines rather than by the built-in integration. [Vercel for GitLab](https://vercel.com/docs/git/vercel-for-gitlab), [Deploying Git repositories](https://vercel.com/docs/git)
- Not documented in the pages read: how Vercel stores provider tokens. Do not infer.

## Current state versus requirements

Read from this worktree; file paths are relative to the repository root.

| Requirement | Exists in code | Missing |
| --- | --- | --- |
| Credentials server-side only, agents never hold them | PATs sealed per (project, repo) in `repo_secrets` (`crates/genie-core/src/secrets.rs`); `Host::git_env` injects the token as a header (`git/hosts.rs`); clones point at the proxy with a Genie credential helper (`git/store.rs::configure`); `WELL_KNOWN_SECRET_VARS` stripped | A connection entity (provider, account, installation, expiry, refresh token) and a token resolver. `host_of` (`store.rs`) is the natural seam: it already fills `host.token` per operation, so a minted or refreshed token can go there |
| User-level secret storage | `user_secrets` table exists (`secrets.rs`) | No provider-connection use of it; no refresh-token rotation handling |
| Connect account and OAuth plumbing | Browser-bound state, PKCE `S256`, one-use flow registry, operator client config via env (`http/social_auth.rs`, `config.rs` `SocialClient`); the access token is used once and discarded | GitHub App registration/config (App ID, private key, webhook secret), installation callback, GitLab application config, token storage |
| List and select repositories | None: a repository is added by typing host and `owner/repo` plus a PAT (`ops/repos.rs`, `http/repos.rs`) | Picker backed by `GET /user/installations/...` and `GET /projects?membership=true`; stored immutable provider repository ID |
| Project grant separate from connection | Task-level repo grant (`set_task_repos`, `genie repos use`) and policy narrowing | Project-level grant naming a connection and its repositories; revoke-on-disconnect behaviour |
| Admin-only connection | `repos add/set`, token changes and host listing require `access.admin()` (`http/repos.rs`); those commands are not listed for agents (asserted in `ops/repos.rs` test `the_roles_that_deliver...`) | Same gate for connect/grant endpoints; owner-versus-admin split if wanted |
| Manager-only PR/MR tools, agents without integration tools | Agents get `pr open/merge/rerun` only through role capabilities `StatusSubmit`/`StatusRework` (`ops/repos.rs`, `genie-core/src/model.rs::class_capabilities`): Executor, Orchestrator and Human have them; Analyst, Reviewer, Tester and Documenter do not. Naming a task's repositories is limited to people and the orchestrator (`http/repos.rs`). Repository listing/connection tools are not exposed to agents | Executors can currently open and (by policy) merge PRs, so "only the manager pushes and opens PRs" is **not** enforced. If the owner means the orchestrator only, a policy/role switch is missing; note that agents push through the proxy, not through provider tools |
| Audit log of git operations | Journal events `git.pushed`, `git.denied`, `cr.opened`, `cr.merged`, `cr.closed`, `ci.*` (`genie-core/src/events.rs`, `http/git.rs::record*`, `git/delivery.rs`) | Push events are written only for agent callers (`let Some(..) = &t.agent else { return }` in `http/git.rs`); no event for connect, disconnect, token set/changed, grant changes, server-side mirror fetches or merges by `Auto` policy beyond `cr.merged`. No audit for who used which connection |
| Idempotent PR/MR creation | `Api::open` creates and, on a rejected response containing "already exists", looks up the open request for the head and returns it (`provider/github.rs`, `provider/gitlab.rs`); `delivery::open_request` records `cr.opened` only when the number changes (`first`) | Lookup-first (documented `head`/`source_branch` filters); no reliance on an undocumented message; closed or merged earlier PR for the same head is not considered; the POST is not retried on timeout (only GETs repeat) |
| Delivery policy: merge only by policy | `Merge::{Human,AgentAfterApproval,Auto}`, `require_ci`, `approvals`, method, `sha` guard, `mergeable`/draft/changes-requested checks (`git/policy.rs`, `delivery::do_merge`) | Awareness of provider required-checks rules (GitHub app-matched checks, GitLab "pipelines must succeed") beyond relaying a 405/422; no `auto_merge` use (intentional) |
| No force push, no branch deletion | Defaults `force_push=false`, `delete_branches=false` (`policy.rs`); proxy rejects them (`check_push`); mirror sets `receive.denyNonFastForwards`/`denyDeletes` (`store.rs::ensure_mirror`) | The policy can still switch both on, and `push_upstream(.., force)` / `delete_upstream` exist; a hard exclusion for connected-provider repositories is not enforced. Task branches are not protected on the host, so host rules would not stop a force push by a server bug |
| Revoke and health detection | `genie repos check` reports reachability, token rights, unreadable sealed token, protected branches, with optional push probe (`git/check.rs`); typed `ApiError::Auth` (`provider/mod.rs`) | Check runs only when an administrator asks. The poller logs a failed `watch_one` to stderr at most once per 10 minutes (`delivery.rs::spawn_poller`); there is no stored health state, no event, no UI badge, and no handling of `Auth` as "connection revoked" |
| Webhooks | None for providers; `delivery.rs` header notes webhooks "would replace the polling"; `http/mod.rs` webhooks serve automations only | Signed provider webhook endpoint; installation, repository and revocation event handling |
| Rate limits | Reads retried on 403/429 and 5xx up to three times, waits up to 5 s, honours `retry-after`/`x-ratelimit-remaining`; per-host `poll_secs` | No per-installation budgeting; GitLab `RateLimit-*` headers unused; no cache of one-hour tokens |
| Self-Managed GitLab / GHES | `git.json` host `url`, `api_url`, `ca_cert`, `http_proxy`, clone templates (`git/hosts.rs`) | Per-host OAuth application config; version detection exists only as `whoami` version string |

## Open decisions and recommendations

| Decision | Recommendation | Why, and what to verify first |
| --- | --- | --- |
| GitHub mechanism | GitHub App with installation tokens; user token only for verification and listing | Per-repository selection and per-request token narrowing are documented for installations; OAuth Apps and fine-grained PATs lack one or the other. Verify with a real test app before committing |
| GitLab mechanism | OAuth authorization code + PKCE, scopes `api write_repository`; keep PAT/project-token paste as an advanced option | OAuth scopes cannot be per repository; the Genie-side grant is the only boundary, so enforce it in `host_of` and in the picker |
| Self-Managed GitLab | Defer OAuth; document that pasted tokens (ideally project access tokens) work today | Per-instance application and version differences; PKCE introduction version unknown |
| Connection ownership | Owned by the Genie user who completed it; a project holds a separate grant (connection + repositories) | GitHub installations outlive the installer, GitLab OAuth tokens die with the user's authorization; decide what happens to grants when the owner is removed (suggest: grants keep working for GitHub installations until revoked, are suspended for GitLab) |
| Who connects | Project owner or admin only, matching today's `access.admin()` gate | Agents get no connection or listing tools; the repository picker is a UI/HTTP surface for people |
| Who opens PRs | Decide explicitly: orchestrator only, or executors as today | The owner's phrase "manager pushes and opens PR" is stricter than current code; a role capability change is a small slice, not a research question |
| Merge | Default human merge; `agent_after_approval` and `auto` remain explicit per-repository policy; rely on provider protections as a second layer | Already modelled; add provider-rule awareness only for clearer error text |
| Force push and deletion | Remove the toggles for connected-provider repositories (server never pushes with `+` or deletes) | Provider protection covers only protected branches; Genie owns the task branches |
| Webhooks | Optional; polling stays the baseline | Loopback deployments cannot receive them; add signature verification when enabled |
| Shared vs per-deployment GitHub App | Per-deployment, operator-registered (manifest flow optional) | Matches the social-auth operator model; a hosted shared app is a product decision with a different security profile |

## Suggested implementation slices

First end-to-end scenario: connect, select a repository, bind it to a project, prepare the working copy, the manager pushes and opens a PR/MR, Genie shows checks.

1. **Connection store and token resolver.** New sealed tables for `provider_connections` (user, provider, host, external account, installation ID, status, expiry) and `project_repo_grants`; a `resolve_token(connection, repo)` used by `store::host_of`. GitHub: mint, cache and refresh one-hour installation tokens narrowed to the repository ID. No changes for agents or the proxy. Acceptance: existing PAT repositories behave identically; unit tests with a fake token server.
2. **GitHub App configuration and install flow.** Operator config (App ID, private key reference by env/secret, client ID/secret, webhook secret). Endpoints: start (install URL with Genie `state` bound to the browser, as in `social_auth.rs`), callback (exchange code with PKCE, `GET /user/installations`, require the returned `installation_id` to be in that list), store connection. Acceptance: forged `installation_id` is rejected.
3. **Repository picker and grant.** List through `GET /user/installations/{id}/repositories` (paged), show the permissions table, create the project grant and a `project_repos` row carrying the provider repository ID; sync the mirror. Acceptance: a repository outside the installation is not offered; revoking the grant removes agent access.
4. **Delivery hardening.** Lookup-first PR/MR creation for both providers; hard no-force/no-delete for connected repositories; journal events for connect, grant, revoke and server-side merge; gate PR tools to the role the owner chooses.
5. **Health.** A stored connection status (`ok`, `needs_attention`, `revoked`, `suspended`, `permission_missing`) updated by token-mint and API `Auth` errors, by a periodic `GET /user/installations`/`GET /app/installations` check, and (when reachable) by `installation`, `installation_repositories` and `github_app_authorization` webhooks; surfaced in Settings and as an event.
6. **Checks display.** Reuse the current watcher; add `check_run`/`pull_request` webhooks only as a poll accelerator.
7. **GitLab.com OAuth.** Same connection model: PKCE flow, `GET /projects?membership=true&min_access_level=30` picker, atomic refresh-token rotation, 401-as-revoked, Genie-side project-ID enforcement. Self-Managed stays on pasted tokens.

## Sources

GitHub
- [Differences between GitHub Apps and OAuth apps](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/differences-between-github-apps-and-oauth-apps)
- [Deciding when to build a GitHub App](https://docs.github.com/en/apps/creating-github-apps/about-creating-github-apps/deciding-when-to-build-a-github-app)
- [About creating GitHub Apps](https://docs.github.com/en/apps/creating-github-apps/about-creating-github-apps/about-creating-github-apps)
- [Best practices for creating a GitHub App](https://docs.github.com/en/apps/creating-github-apps/about-creating-github-apps/best-practices-for-creating-a-github-app)
- [Registering a GitHub App](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/registering-a-github-app)
- [About the setup URL](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/about-the-setup-url)
- [Choosing permissions for a GitHub App](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/choosing-permissions-for-a-github-app)
- [Creating a GitHub App from a manifest](https://docs.github.com/en/apps/creating-github-apps/setting-up-a-github-app/creating-a-github-app-from-a-manifest)
- [Sharing your GitHub App](https://docs.github.com/en/apps/sharing-github-apps/sharing-your-github-app)
- [Installing your own GitHub App](https://docs.github.com/en/apps/using-github-apps/installing-your-own-github-app)
- [Reviewing and modifying installed GitHub Apps](https://docs.github.com/en/apps/using-github-apps/reviewing-and-modifying-installed-github-apps)
- [Authorizing GitHub Apps](https://docs.github.com/en/apps/using-github-apps/authorizing-github-apps)
- [Generating an installation access token](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-an-installation-access-token-for-a-github-app)
- [Authenticating as an installation](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/authenticating-as-a-github-app-installation)
- [Generating a user access token](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app)
- [Refreshing user access tokens](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/refreshing-user-access-tokens)
- [Scopes for OAuth apps](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/scopes-for-oauth-apps)
- [Managing personal access tokens](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens)
- [Permissions required for GitHub Apps](https://docs.github.com/en/rest/overview/permissions-required-for-github-apps)
- [Installations REST API](https://docs.github.com/en/rest/apps/installations)
- [Apps REST API](https://docs.github.com/en/rest/apps/apps)
- [Pull requests REST API](https://docs.github.com/en/rest/pulls/pulls)
- [Using the REST API with checks](https://docs.github.com/en/rest/guides/using-the-rest-api-to-interact-with-checks)
- [Rate limits for the REST API](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api)
- [About protected branches](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches)
- [Webhook events and payloads](https://docs.github.com/en/webhooks/webhook-events-and-payloads)

GitLab
- [OAuth 2.0 identity provider API](https://docs.gitlab.com/api/oauth2/)
- [Configure GitLab as an OAuth 2.0 provider](https://docs.gitlab.com/integration/oauth_provider/)
- [Personal access tokens](https://docs.gitlab.com/user/profile/personal_access_tokens/)
- [Project access tokens](https://docs.gitlab.com/user/project/settings/project_access_tokens/)
- [Project access tokens API](https://docs.gitlab.com/api/project_access_tokens/)
- [Access token scopes](https://docs.gitlab.com/security/tokens/access_token_scopes/)
- [Projects API](https://docs.gitlab.com/api/projects/)
- [Merge requests API](https://docs.gitlab.com/api/merge_requests/)
- [Protected branches](https://docs.gitlab.com/user/project/repository/branches/protected/)
- [Auto-merge and merge checks](https://docs.gitlab.com/user/project/merge_requests/auto_merge/)
- [Webhooks](https://docs.gitlab.com/user/project/integrations/webhooks/) and [webhook events](https://docs.gitlab.com/user/project/integrations/webhook_events/)
- [REST API troubleshooting](https://docs.gitlab.com/api/rest/troubleshooting/) and [REST API authentication](https://docs.gitlab.com/api/rest/authentication/)
- [GitLab.com rate limits](https://docs.gitlab.com/user/gitlab_com/rate_limits/)
- [Account and limit settings](https://docs.gitlab.com/administration/settings/account_and_limit_settings/)
- [Clone a Git repository](https://docs.gitlab.com/topics/git/clone/)

Vercel
- [Deploying Git repositories](https://vercel.com/docs/git), [Vercel for GitHub](https://vercel.com/docs/git/vercel-for-github), [Vercel for GitLab](https://vercel.com/docs/git/vercel-for-gitlab), [Missing GitHub repository guide](https://vercel.com/kb/guide/unable-to-find-github-repository)

Standards
- [RFC 7636: Proof Key for Code Exchange](https://datatracker.ietf.org/doc/html/rfc7636)
