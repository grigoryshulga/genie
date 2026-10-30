---
title: Agent guide
summary: The workflow every agent follows in this repository, from isolation to cleanup.
type: guide
status: current
tags: [agents, workflow]
---

# Agent guide

Follow steps 1–7 for every repository task. Read these pages when they apply:

- [Coding standards](coding-standards.md): before changing code.
- [The genie CLI guide](../development/genie-cli.md): before tracking work.
- [The testing workflow](../development/testing-workflow.md): to check a UI change on the development stand, update the local tracker, or free disk space.
- [The preview and approval guide](../development/visual-preview.md): before preparing a UI review, configuring T3 Project Actions, or publishing a UI change.
- [The Compose service lifecycle](compose-service-lifecycle.md): when a task needs a Compose-managed service started or updated.

For browser-to-Figma capture, offline plugin work, or native import validation, use the separate private [genie-figma-tools repository](https://github.com/grigoryshulga/genie-figma-tools) and its [agent workflow](https://github.com/grigoryshulga/genie-figma-tools/blob/main/docs/agent-workflow.md). This repository owns the application's tokens and exporter; the toolkit owns capture and import code and tests.

A **UI change** alters what a screen renders or how a control behaves. It covers anything in `web/` or `design-system/`, design tokens, user-facing copy, static assets the application serves, and the documentation site's theme or landing page. Server, CLI, CI, scripts, and Markdown content are not UI changes. A task with a UI change follows the UI path in steps 4–6. Every other task skips step 4.

## 1. Isolate the task

1. Run `git status --short --branch`, `git worktree list`, and `git fetch origin dev`.
2. Create a dedicated worktree and task branch from the fetched `origin/dev`. `dev` is the integration branch: all feature and fix work lands there. `main` receives only release PRs opened from `dev`.
3. Reuse an existing worktree only after you confirm that it belongs to this task and has no unrelated changes.
4. In T3 Code, use its worktree status and handoff tools so the conversation follows the checkout. Otherwise run `git worktree add -b <task-branch> <separate-path> origin/dev`, and run all later commands in that worktree.
5. Preserve other tasks' worktrees and uncommitted changes.
6. Record the branch, worktree path, and starting commit in the task.

A feature branch in the main checkout does not count as isolation.

## 2. Track work in genie

1. Use the repository tracker: container `genie-tracker`, project `genie`, prefix `GNE`, manual mode, web UI `http://127.0.0.1:7422`.
2. Run task commands on the host with `genie --project genie ...`, connected over HTTP as the CLI guide shows. Docker operates the server; the host runs the CLI.
3. Confirm the server and project explicitly. The CLI's default project may belong to another repository.
4. Reuse the assigned task. If none is assigned, search for an existing task first. Create a task only with a description and acceptance criteria.
5. Move the task through its permitted workflow into `in_progress` before you implement.
6. Record milestones as CLI `progress` comments: decisions, validation results, and blockers.
7. Keep acceptance criteria current, and attach verification evidence. The CLI guide lists commands and role restrictions.
8. If the correct project, credentials, or CLI is unavailable, report the exact blocker and request the missing configuration. Keep the work unfinished. A local note does not replace tracking.

## 3. Implement and verify

1. Follow the coding standards. Run the checks that fit the affected paths. The CI workflow and package scripts decide which commands are authoritative.
2. For a UI change, iterate with `npm run dev:stand`. It gives hot reload on varied seeded data, with no image build, CI run, or preview candidate. Add a case to the stand's data when the change needs one.
3. Review the final diff, including generated files. Commit only this task's changes.

## 4. Review a UI change with the owner

Applies only to a UI change. For any other task, record the validation results in genie and go to step 5. The owner may request an integration review of a batch at any time. Build that batch only when the owner requests it.

1. Show the change with the instant fixture preview: `npm run preview:ui`, as the [preview guide](../development/visual-preview.md) describes.
2. Build a fixed integration candidate instead when the owner asks for one, or when the change depends on real backend behavior. Base it on current fetched `origin/dev` or on the explicitly selected task composition. A settled T3 thread, readiness, or worktree creation never starts a build.
3. Give the owner the exact commit, a short review scenario, and the validation evidence. For a candidate, also give its ID, composition, base SHA, health, and private access details. Share them in this conversation and in genie.
4. Stop and wait for the owner's explicit approval of what was shown and permission to publish. Keep the task unfinished. Record `awaiting_visual_review` in a progress comment. This is a review milestone, not a new tracker status or owner acceptance.
5. A missing preview tool or a failed build blocks the gate. Resolve it or report it. The gate stays closed until it is resolved.
6. If the task commit, base, or included task composition changes before publication, show the change again and get a new approval. Publish the approved task commit. Keep other tasks' changes in their own branches.
7. Keep private preview URLs, tailnet hostnames, credentials, and local machine paths out of GitHub descriptions and comments. Public PR evidence uses task IDs, candidate IDs and SHAs, check results, and portable artifacts.

## 5. Publish a pull request

1. Publish once validation passes. For a UI change, publish once the owner has approved it.
2. Load the global `pr` skill and use it to prepare the PR description. If the skill is unavailable, report it and obtain access before you publish.
3. Push the task branch and open a PR against `dev`.
4. Write the PR title, description, and agent-authored PR comments in English. Keep explanations in English even when the conversation and the genie task are in Russian.
5. Include the genie task ID, scope, validation evidence, and material limitations. For a UI change, also include the approved commit and, when a candidate was used, its ID and composition.
6. Record the PR URL in genie. In T3 Code, register the PR with the thread's pull-request linking tool.
7. Move the task to `review` when its workflow requirements are met. Publication alone does not complete the task.

### Releases

Release PRs target `main` from `dev` only. `main` is protected and accepts only PRs from `dev` that also bump the shared version in `Cargo.toml`, `package.json`, `Cargo.lock`, and `package-lock.json`, and add a dated `CHANGELOG.md` section. `.github/scripts/release_gate.py` enforces these rules. Opening, merging, or cutting a release is a separate authorized task. Ordinary work targets `dev` and carries no release metadata.

## 6. Wait for CI before completion

1. Wait for the published PR's latest head commit to finish CI successfully. If a check fails, fix it and wait for the new head. For a UI change, first repeat step 4 for the changed commit, then push.
2. Verify both the head SHA and the check results on the host. Pending, cancelled, failed, missing, or unreadable checks do not establish success. Jobs that the workflow's path filters skip are acceptable when the applicable jobs have passed. A docs-only PR still needs its `Changes` job to pass.
3. In T3 Code, when a PR watcher is available, register the watch. It resumes this task when results arrive. Otherwise, use the host CLI's bounded check-wait facility. An interim message says that CI is pending. It never claims completion.
4. Only after CI succeeds, record in genie the PR URL, the verified head SHA, the check results, and the acceptance evidence.
5. Finish the permitted review and approval transitions before you set `done`. In assisted projects, leave the task awaiting owner-only acceptance when it is required. Workflow gates stay closed until their conditions are met.
6. Before the final handoff in T3 Code, stop the watch. Report the PR link and the successful CI evidence. An open PR with green CI is the repository deliverable. Merging requires separate authorization.

## 7. Clean up after merge into dev

1. When the task's PR is confirmed merged into `dev`, fetch `origin/dev` and verify that the merge commit is present.
2. Follow [merged preview cleanup](../development/visual-preview.md#clean-up-after-merge). It withdraws readiness, stops and removes the task's candidates and their integration worktrees, and removes its clean task worktree. A task without a preview has no candidates, and the same command still removes its worktree. Cleanup is a required post-merge step. Publication, green CI, or a closed unmerged PR does not authorize it.
3. Run cleanup from a stable checkout outside the worktree being removed. Preserve changed worktrees, other tasks, and candidates that contain unmerged work.
4. In T3 Code, check the thread and worktree binding before removal, and release that binding through the app first. If the app cannot release a live binding, retire the candidates with `--keep-worktree`. Record the retained path and the exact limitation in genie. Finish workspace removal after the binding is released.
5. Record in genie the merged PR, the removed candidates and worktrees, and any retained paths. Keep compact candidate tombstones, so old fixed URLs never point at a different build. Keep shared caches, networking identities, tracker deployment worktrees, and data volumes.
6. Run `preview gc` and review its dry-run plan. Apply it with `--execute`, as the [testing workflow](../development/testing-workflow.md#keeping-the-disk-clean) describes. Run it also whenever disk space is short.
