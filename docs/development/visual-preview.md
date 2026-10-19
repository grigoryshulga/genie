---
title: Visual review before publishing
summary: Prepare combined local previews and obtain owner approval before publishing task branches.
type: guide
status: current
---

# Visual review before publishing

An agent prepares a committed, validated task and a fixed integration candidate, then waits for the owner to approve that candidate and authorize publication. Feature PRs target `dev`; only release PRs from `dev` target `main`. [AGENTS.md](https://github.com/grigoryshulga/genie/blob/main/AGENTS.md) defines the mandatory gates, and [the tracker guide](genie-cli.md) defines task commands.

## Local installation

The preview helper is machine-local, not a tracked repository tool. On this installation it lives at `$(git rev-parse --path-format=absolute --git-common-dir)/genie-preview/preview.py`. All worktrees use that shared copy. The `prepare` action creates a `.genie-preview` symlink in its current worktree and adds `/.genie-preview` to the shared `info/exclude`, preserving existing exclusions. Scripts, tests, dependencies, logs, databases, credentials and build artifacts stay outside commits. No `.gitignore` change is needed.

The helper requires Linux, Python 3.12+, Git, Node/npm and the repository's Rust toolchain. Tailscale is needed only for remote access. On another installation, obtain a trusted copy of the local helper and its tests; a clone does not contain them. If it is missing, report the missing installation rather than silently bypassing preview review.

From a task worktree, define this convenience function for the examples below:

```bash
preview() {
  python3 "$(git rev-parse --path-format=absolute --git-common-dir)/genie-preview/preview.py" "$@"
}
```

## T3 Project Actions

Create two project-scoped Actions. Their commands discover the shared git directory from the thread's current worktree; no machine-specific absolute path is required.

**Prepare preview workspace** command:

```bash
python3 "$(git rev-parse --path-format=absolute --git-common-dir)/genie-preview/preview.py" prepare
```

Enable **Run automatically on worktree creation** and **Wait for it to finish before the agent starts**. Leave **Run when the thread settles** and automatic preview opening disabled. Preparation installs exact npm dependencies only when the lockfile/manifests changed, reuses npm's download cache, and starts the shared preview index. Installed dependencies remain worktree-local.

**Request integration preview** command:

```bash
python3 "$(git rev-parse --path-format=absolute --git-common-dir)/genie-preview/preview.py" request
```

Enable **Run in the thread's worktree when the thread settles** and **Open preview automatically when this action runs**. Leave creation-time execution disabled. Set **Preview URL** to `http://genie.localhost:7480`, or to the remote index URL printed by `preview enable-tailscale` when using T3 from another machine. This Action queues work and returns; compilation runs independently of the thread.

An interruption, question or error can settle a thread. The request Action does nothing unless the worktree has explicitly marked its current clean commit ready. Configure the Actions manually; installing the helper does not modify T3 project settings.

## Mark a task ready

Run applicable checks, inspect the full diff, commit only this task's changes, then:

```bash
preview ready --task GNE-123
preview request
preview status
```

Readiness records the task ID, branch, worktree and exact commit. The helper never commits, pushes, creates PRs, approves tasks, or infers readiness from a branch name. Tracked or untracked changes, a different HEAD, or a removed worktree invalidate readiness. Ignored dependency and build directories are not task changes.

Before resuming edits or removing a task from the next composition:

```bash
preview withdraw
```

Other valid ready tasks remain selected. If any declared task is stale, the combined build fails with a named error instead of dropping that task. Mark it ready again or withdraw it before requesting another build.

## Combined candidates

One worker at a time merges all explicitly ready task commits, sorted by task ID, into a dedicated detached worktree from the current local `origin/dev`. Fetch `origin/dev` before declaring readiness; builds themselves do not fetch or move the base. A conflict blocks the entire declared composition. Agents' worktrees remain unchanged.

The candidate ID is derived from the base SHA and ordered task IDs/SHAs. Duplicate requests reuse the same running candidate. A request received during another build waits for the build lock, then evaluates the newest selection. The worker checks readiness again before publication. The index advances only after successful build and startup checks; failed candidates do not replace the previous working preview.

Each candidate retains its own binary, static web assets, integration worktree and database. Source versions behind a candidate link do not change. Review interactions can change its disposable database; it is not a read-only data snapshot. Build caches are shared only by the serialized builder. Backend artifacts are keyed by backend sources and compiler identity; UI-only changes reuse that backend, while web assets are rebuilt from the exact combined source. Rust uses the development profile for local review, not a release optimization build.

The backend runs with `--no-agents`, its own home/data directories, no inherited tracker credentials or model keys, and a seeded manual project. These previews cover UI/API interaction, not live agent/provider behavior. A private `credentials.txt` next to the candidate manifest contains the generated `preview` account password; read it locally without putting it into task comments or logs. Supply additional representative test data in the disposable preview when a review scenario needs it.

The index exposes `/api/status` with candidate manifests and readiness, not passwords, databases or filesystem browsing. Candidate gateways keep authentication cookies separate even when multiple candidates use different ports on the same Tailscale hostname. Browser API requests and event streams stay on the candidate's origin.

## Local names and Tailscale

Local index: `http://genie.localhost:7480`. Candidate application ports are allocated from 8500 through 8599. Local browsers resolve `.localhost` to loopback; on a remote machine that points to the remote machine itself. Use the Tailscale URL remotely.

Enable remote access once on the build host:

```bash
preview enable-tailscale
```

The command discovers the machine's current MagicDNS name, publishes the loopback index through Tailscale Serve on HTTPS port 8443, and prints both addresses. Each ready candidate gets its own HTTPS port matching its local gateway port. Tailscale access policy must permit the reviewing devices to access those ports. This uses private tailnet access, not Funnel. Existing Serve listeners, including T3 on port 443, are preserved; a conflicting port is an error.

When the tailnet or machine name changes, run the command again before requesting new candidates. The index reloads its host configuration on requests. Use the newly printed remote URL in the T3 Action. Existing candidate manifests retain the address used when they were created; after a DNS rename, restart and verify links rather than assuming an old hostname still resolves.

No service binds to a public or LAN interface. The gateway forwards to the candidate's loopback backend and preserves separate local/Tailscale entry points. Connecting to an index through Tailscale yields Tailscale application links, so remote navigation does not fall back to loopback.

## Owner approval and lifecycle

Before handing off, verify the application URL and its relevant behavior, then record candidate ID, base SHA, all task SHAs, local/remote URLs, check results, limitations and a short review scenario in the task. For a documentation-only task, provide the rendered document or the exact committed diff alongside the preview. Record `awaiting_visual_review` as a progress comment; keep the existing tracker status until a permitted workflow transition applies.

The owner's explicit response must identify the reviewed candidate and authorize publication. Local readiness and T3 thread state are not approval. Changed task commits, base or task composition need a new candidate and approval before pushing. An approval of A+B does not establish that A alone works: if a task is removed, review the new composition, and disclose genuine task dependencies in the PRs.

After approval, publish each task's approved branch to `dev`, link its PR and follow the existing CI/review workflow. Visual approval is separate from owner task acceptance and merge authorization.

```bash
preview stop <candidate-id>
preview restart <candidate-id>
```

Stopping terminates only recorded preview processes after checking their process identity. Restart reuses the same binary, web assets, port and disposable data. Candidate ports and Tailscale routes stay reserved for their original candidate so old links are never silently repointed. Stop unused candidates to release memory. Removing retained artifacts, git worktrees or reserved Serve mappings is an explicit local maintenance operation; the helper never deletes another task's worktree or database.

Local logs and diagnostic manifests live under the shared `genie-preview` directory. A successful health check confirms startup, not correctness of the task's acceptance criteria. Run the repository's applicable checks and manually verify the intended review scenario before requesting approval.
