# Repository workflow

For browser-to-Figma capture, offline plugin work or native import validation, use the separate private [genie-figma-tools repository](https://github.com/grigoryshulga/genie-figma-tools) and its [agent workflow](https://github.com/grigoryshulga/genie-figma-tools/blob/main/docs/agent-workflow.md). This repository owns the application's tokens and exporter; the toolkit owns capture/import code and tests.

Follow steps 1–7 for every repository task. Read these first when they apply:

- [CODING_STANDARDS.md](CODING_STANDARDS.md) before changing code.
- [The genie CLI guide](docs/development/genie-cli.md) before tracking work.
- [The testing workflow](docs/development/testing-workflow.md) to check a UI change on the development stand, update the local tracker, or free disk space.
- [The preview and approval guide](docs/development/visual-preview.md) before preparing a UI review, configuring T3 Project Actions, or publishing a UI change.
- [The Compose service lifecycle](docs/agents/compose-service-lifecycle.md) when a task needs a Compose-managed service started or updated.

A **UI change** alters what a screen renders or how a control behaves: anything in `web/` or `design-system/`, design tokens, user-facing copy, static assets the application serves, and the documentation site's theme or landing page. Server, CLI, CI, scripts and Markdown content are not UI changes. A task that includes a UI change follows the UI path in steps 4–6; every other task skips the preview.

## 1. Isolate the task

- Run `git status --short --branch`, `git worktree list`, and `git fetch origin dev`.
- Work in a dedicated worktree and task branch created from the fetched `origin/dev`. `dev` is the integration branch: all feature and fix work lands there. `main` receives only release PRs opened from `dev`, so branch task work from `origin/dev`. Reuse an existing worktree only after confirming that it belongs to this task and has no unrelated changes. A feature branch in the main checkout is insufficient isolation.
- In T3 Code, use its worktree status and handoff tools so the conversation follows the checkout. Otherwise create a worktree with `git worktree add -b <task-branch> <separate-path> origin/dev` and run subsequent commands there.
- Preserve other tasks' worktrees and uncommitted changes. Record the branch, worktree path, and starting commit in the task.

## 2. Track work in genie

- Use the repository tracker: container `genie-tracker`, project `genie`, prefix `GNE`, manual mode, web UI `http://127.0.0.1:7422`. Use the native host CLI: `genie --project genie ...`, connected over HTTP as shown in the CLI guide. Docker operates the server; task commands run on the host.
- Confirm the server and project explicitly; the CLI's default project may belong to another repository. Reuse the assigned task, or search for an existing task before creating one with a description and acceptance criteria.
- Move the task through its permitted workflow into `in_progress` before implementation. Record milestones with CLI `progress` comments, including decisions, validation results, and blockers.
- Keep acceptance criteria current and attach verification evidence. Use the CLI guide for commands and role restrictions.
- If the correct project, credentials, or CLI is unavailable, report the exact blocker and request the missing configuration. Keep the work unfinished; a local note does not replace tracking.

## 3. Implement and verify

- Follow the coding standards and run checks appropriate to the affected paths. The CI workflow and package scripts are the authority for commands.
- Iterate on a UI change with `npm run dev:stand`: hot reload on varied seeded data, with no image build, CI run or preview candidate. Add a case to the stand's data when the change needs one.
- Review the final diff, including generated files, and commit only this task's changes.

## 4. Review a UI change with the owner

Applies to a UI change. For any other task, record the validation results in genie and continue with step 5.

- Mark the exact commit ready and request an integration preview as described in the preview guide. A settled T3 thread triggers a request only when current readiness exists; it is not evidence of task completion or owner approval.
- Verify that the candidate is ready and healthy. Give the owner its fixed candidate ID, local and Tailscale links when configured, base SHA, included task SHAs, validation evidence, and a short review scenario. Record this evidence in genie.
- Stop and await the owner's explicit approval of that candidate and permission to publish the task. Keep the task unfinished and record `awaiting_visual_review` in a progress comment; this is a review milestone, not a new tracker status or owner acceptance. A missing preview tool or failed build is a blocker to resolve or report, and the gate stays closed until it is resolved.
- Rebuild and obtain approval of a new candidate when the task commit, base, or included task composition changes before publication. Publish the approved task commit, keeping other tasks' changes in their own branches.

## 5. Publish a pull request

- Publish once validation passes; for a UI change, once the owner has approved the candidate. Load and use the global `pr` skill to prepare the PR description, then push the task branch and open a PR against `dev`. Release PRs target `main` from `dev` only. If the skill is unavailable, report it and obtain access before publishing.
- Treat `main` as release-only. The `main` branch is protected and accepts PRs exclusively from `dev` that also bump the shared version in `Cargo.toml`, `package.json`, `Cargo.lock`, `package-lock.json` and add a dated `CHANGELOG.md` section, as enforced by `.github/scripts/release_gate.py`. Opening, merging, or cutting a release is a separate authorized task; ordinary work targets `dev` and carries no release metadata.
- Write the PR title, description, and agent-authored PR comments strictly in English. Keep explanations in English even when the conversation and genie task are in Russian.
- Include the genie task ID, scope, validation evidence, and material limitations; for a UI change, also the approved candidate ID and composition. Record the PR URL in genie. In T3 Code, register the PR with the thread's pull-request linking tool.
- Move the task to `review` when its workflow requirements are met. Publication alone does not complete the task.

## 6. Wait for CI before completion

- Wait for the published PR's latest head commit to finish CI successfully. Fix failed checks and wait again for the new head; a UI change first repeats step 4 for the changed candidate before pushing.
- Verify both the head SHA and check results on the host. Pending, cancelled, failed, missing, or unreadable checks do not establish success. Jobs deliberately skipped by the workflow's path filters are acceptable when the applicable jobs have passed; a docs-only PR still needs its `Changes` job to pass.
- When T3 Code provides a PR watcher, register the watch and let it resume this task when results arrive. An interim message must say that CI is pending; it must not claim completion. Otherwise use the host CLI's bounded check-wait facility.
- Only after successful CI, record the PR URL, verified head SHA, check results, and acceptance evidence in genie. Finish the permitted review/approval transitions before setting `done`. Respect owner-only acceptance in assisted projects; leave the task awaiting that action when required. Workflow gates stay closed until their conditions are met.
- Before the final handoff in T3 Code, stop the watch. Report the PR link and successful CI evidence. An open PR with green CI is the repository deliverable; merging requires separate authorization.

## 7. Clean up after merge into dev

- When the task's PR is confirmed merged into `dev`, fetch `origin/dev` and verify the merge commit is present. Then follow [merged preview cleanup](docs/development/visual-preview.md#clean-up-after-merge): it withdraws readiness, stops and removes this task's candidates and their integration worktrees, and removes its clean task worktree. A task without a preview has no candidates, and the same command still removes its worktree. Cleanup is a required post-merge step; publication, green CI or a closed unmerged PR does not authorize it.
- Run cleanup from a stable checkout outside the worktree being removed. Preserve changed worktrees, other tasks and candidates containing unmerged work. In T3 Code, check the thread/worktree binding before removal; release that binding through the app first. If the app cannot release a live binding, retire the candidates with `--keep-worktree`, record the retained path and exact limitation in genie, and finish workspace removal after it is released.
- Record the merged PR, removed candidates/worktrees and any retained paths in genie. Retain compact candidate tombstones so old fixed URLs never point at a different build. Keep shared caches, networking identities, tracker deployment worktrees and data volumes.
- Then run `preview gc` and review its dry-run plan; apply it with `--execute` ([testing workflow](docs/development/testing-workflow.md#keeping-the-disk-clean)). Run it as well whenever disk space is short.
