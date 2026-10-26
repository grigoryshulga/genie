---
title: Repositories and delivery
summary: Connect Git hosts, scope task repositories, and enforce push, review, and merge policies.
type: reference
status: current
---

# Repositories and delivery

A project can connect several repositories on GitHub, GitLab, self-hosted GitLab, or a plain Git server. The server holds hosting credentials; agents use an authenticated Git proxy. A legacy project registered with `project add --repo` continues to use its local repository without this proxy.

## Connect a host and repository

Create `<data>/git.json` without tokens:

```json
{
  "hosts": {
    "github": { "kind": "github", "url": "https://github.com" },
    "gitlab": { "kind": "gitlab", "url": "https://git.company.example" }
  }
}
```

Then, from a shell where `PAT` contains the repository token:

```bash
genie repos hosts --check
printf '%s\n' "$PAT" | genie repos add api github acme/shop-api --mount services/api --project shop --token-stdin
genie repos check api --project shop
```

The server fetches a mirror and discovers the default branch. The web equivalent is **Settings → Project settings → Repositories**. `genie repos check api --probe-push --project shop` additionally creates and deletes a test branch to verify push access.

## Connect a GitHub account

Instead of pasting a token per repository, a person can connect an account through the deployment's GitHub App and pick repositories from it. A project reaches only the repositories attached to it; the connection itself belongs to the person who made it.

### Register the app (operator, once)

Register a GitHub App for this deployment and describe it in `config.json`. The file names the secrets; it never holds them:

```json
{
  "publicUrl": "https://genie.example.com",
  "gitApps": {
    "github": {
      "host": "github",
      "appId": 123456,
      "slug": "genie-acme",
      "clientId": "Iv1.0123456789abcdef",
      "clientSecretEnv": "GENIE_GITHUB_APP_SECRET",
      "privateKeyFile": "/etc/genie/github-app.pem"
    }
  }
}
```

`host` is a `github` host of `git.json`. In the app's settings use `<publicUrl>/api/git/github/callback` as both the callback URL and the setup URL, leave "Request user authorization during installation" off, keep user token expiry on, and grant these repository permissions: Contents (read and write), Pull requests (read and write), Checks (read), Commit statuses (read), Metadata (read), and Actions (read and write) only if `genie pr rerun` should work. Do not grant Workflows or Administration: without Workflows GitHub refuses a push that edits workflow files. Keep the private key readable by the server only.

### Connect, pick and attach

An administrator of a project opens **Settings → Project settings → Repositories → Connected accounts** and chooses **Connect GitHub**.

1. The person installs the app on an account and chooses all or selected repositories. GitHub sends them back with an `installation_id`, which anyone can forge, so it is only a claim.
2. GitHub asks them to authorize (PKCE `S256`, bound to the browser that started the flow). The server accepts the installation only if GitHub lists it among that person's installations.
3. **Choose repositories** lists the repositories that both the person and the installation reach. **Add** attaches one to the project; its path, default branch and id come from GitHub, not from the request.

The user token behind the list lives in server memory for at most 15 minutes and is never stored; **Confirm on GitHub** renews it. Installation tokens are minted for one repository when an operation needs it, cached in memory until shortly before they expire, and never stored, logged or given to agents.

A repository attached from a connection takes no token of its own and never allows force push or deleting branches, whatever its policy says.

### Who may do what

- Only an administrator of a project connects an account or attaches its repositories; the connection owner or a server administrator can disconnect it.
- On a connected repository the task manager (the orchestrator, or a person) opens, reruns and merges requests. The agents of a task push their branch through the proxy and nothing more. Repositories reached with a token keep their current rules.
- Disconnecting stops every operation at once; repositories, deliveries and links to requests stay. Connecting the same installation again revives it.
- If the owner of a connection is deleted or disabled, operations stay blocked (`owner_removed`) until a server administrator reassigns the connection (`POST /api/git/connections/<id>/reassign` with `{"login": "…"}`).

### Health and audit

A connection is `ok`, `needs_attention`, `permission_missing`, `suspended`, `revoked` or `owner_removed`. Failed token mints and a check every 30 minutes (and **Check** on demand) update it; a suspended, removed, revoked or ownerless connection is refused before any call to GitHub. Connecting, granting a repository, disconnecting and reassigning are recorded in the configuration history (`config_changes`, item `git.connection:<id>`) and, for the affected projects, in the journal as `git.connection`.

## Host fields

Host keys use `snake_case`; unknown fields are rejected.

| Field | Meaning |
| --- | --- |
| `kind`, `url` | `github`, `gitlab`, or `plain`, and the host's base URL. Plain hosts have no PR/MR or CI integration. |
| `api_url` | Override the inferred API endpoint. |
| `transport` | Server-side `https` (default) or `ssh`. Agents still use the HTTP proxy. |
| `ssh_key`, `known_hosts`, `ssh_command` | SSH identity, host verification file, and optional command override. |
| `ca_cert`, `http_proxy` | Corporate CA and proxy for Git/API access. |
| `insecure_skip_verify` | Explicitly disable certificate verification; reported by `doctor`. |
| `clone_urls` | Custom HTTPS/SSH templates with `{url}`, `{host}`, and `{remote}`. |
| `identity` | Agent commit author `name` and `email`. |
| `poll_secs` | Hosting poll interval, 10–3600 seconds; default 60. |

## Repository tokens

Project administrators provide tokens per project/repository (or [connect a GitHub account](#connect-a-github-account) instead), through the web or `repos add|set --token-stdin`. The same remote can use different tokens in different projects. Tokens are not accepted as CLI arguments or MCP inputs.

Tokens are encrypted in `server.db` with `<data>/secrets.key` and bound to their project/repository. API responses expose only a suffix and update time. `git.json` fields `token` and `token_file` are rejected.

For hosted projects, Genie strips hosting-token environment variables from agents. Mirror access and remote credentials stay on the server. Keep the encryption key in a trusted backup or be prepared to re-enter credentials after restoration.

Use a repository-scoped bot token and protect the default branch on the host. Server-side policy and host protection are independent checks.

## Task repository scope

Agents can read repositories allowed by role and policy. Writing also requires an explicit task grant:

```bash
genie repos use api:write web:read
```

A single-repository project selects it automatically. With several repositories, a team launch requires the task's repository selection. Project-level read-only access and role `git`/`files` settings cannot be widened by the task.

Bare mirrors live under `<data>/repos/<host>/<remote>.git`. Team clones live under `<data>/workspaces/<project>/<team>/`, with each repository at its configured mount path.

## Delivery policy

The default policy allows pushes to the task branch, opens a PR/MR, and leaves merging to a person. The proxy rejects default-branch pushes, force pushes, and branch deletion under that policy.

Repository presets in Settings cover read-only access, human merge, agent merge after approval, automatic merge, and direct push. Inspect the resulting policy before enabling broader access. The task's branch name defaults to `genie/{task}`.

```bash
genie pr open
genie pr rerun
```

Use `genie pr --help` for request inspection and merge actions available to the caller.

## Workflow gates and CI

- **Review:** when policy requires a request, a published branch needs an open PR/MR. Failed checks on the observed commit block agent submission. Pending or stalled checks do not block this transition.
- **Done:** an open, unmerged request blocks agent completion. Assisted projects still require a person to close the task.
- **Merge:** repository `require_ci` and merge policy determine whether the server or agent may merge. A human merge through the host is observed afterward.

The watcher records request state, branch/request CI, and the merge commit's CI on the target branch after a merge. Failed checks include available log excerpts; stalled CI generates a separate signal. A hosting outage falls back to recorded state instead of silently treating failed checks as passing.

Requests closed without merging mark delivery abandoned. The orchestrator decides whether to resume or cancel the work.
