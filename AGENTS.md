# Repository workflow

Follow this sequence for every repository task. Read [CODING_STANDARDS.md](CODING_STANDARDS.md) before changing code, and [the genie CLI guide](docs/development/genie-cli.md) before tracking work.

## 1. Isolate the task

- Run `git status --short --branch`, `git worktree list`, and `git fetch origin main`.
- Work in a dedicated worktree and task branch created from the fetched `origin/main`. Reuse an existing worktree only after confirming that it belongs to this task and has no unrelated changes. A feature branch in the main checkout is insufficient isolation.
- In T3 Code, use its worktree status and handoff tools so the conversation follows the checkout. Otherwise create a worktree with `git worktree add -b <task-branch> <separate-path> origin/main` and run subsequent commands there.
- Preserve other tasks' worktrees and uncommitted changes. Record the branch, worktree path, and starting commit in the task.

## 2. Track work in genie

- Use the repository tracker: container `genie-tracker`, project `genie`, prefix `GNE`, manual mode, web UI `http://127.0.0.1:7422`. Invoke `docker exec -i genie-tracker genie --project genie ...` as shown in the CLI guide.
- Confirm the server and project explicitly; the CLI's default project may belong to another repository. Reuse the assigned task, or search for an existing task before creating one with a description and acceptance criteria.
- Move the task through its permitted workflow into `in_progress` before implementation. Record milestones with CLI `progress` comments, including decisions, validation results, and blockers.
- Keep acceptance criteria current and attach verification evidence. Use the CLI guide for commands and role restrictions.
- If the correct project, credentials, or CLI is unavailable, report the exact blocker and request the missing configuration. Keep the work unfinished; a local note does not replace tracking.

## 3. Implement and verify

- Follow the coding standards and run checks appropriate to the affected paths. The CI workflow and package scripts are the authority for commands.
- Review the final diff, including generated files, and commit only this task's changes.

## 4. Publish a pull request

- Load and use the global `pr` skill to prepare the PR description, then publish the task branch and open a PR against `main`. If the skill is unavailable, report it and obtain access before publishing.
- Write the PR title, description, and agent-authored PR comments strictly in English. Keep explanations in English even when the conversation and genie task are in Russian.
- Include the genie task ID, scope, validation evidence, and material limitations. Record the PR URL in genie. In T3 Code, register the PR with the thread's pull-request linking tool.
- Move the task to `review` when its workflow requirements are met. Publication alone does not complete the task.

## 5. Wait for CI before completion

- Wait for the published PR's latest head commit to finish CI successfully. Fix failed checks, push the fixes, and wait again for that new head.
- Verify both the head SHA and check results on the host. Pending, cancelled, failed, missing, or unreadable checks do not establish success. Jobs deliberately skipped by the workflow's path filters are acceptable when the applicable jobs have passed; a docs-only PR still needs its `Changes` job to pass.
- When T3 Code provides a PR watcher, register the watch and let it resume this task when results arrive. An interim message must say that CI is pending; it must not claim completion. Otherwise use the host CLI's bounded check-wait facility.
- Only after successful CI, record the PR URL, verified head SHA, check results, and acceptance evidence in genie. Finish the permitted review/approval transitions before setting `done`. Respect owner-only acceptance in assisted projects; leave the task awaiting that action when required. Never force workflow gates to claim completion.
- Before the final handoff in T3 Code, stop the watch. Report the PR link and successful CI evidence. An open PR with green CI is the repository deliverable; merging requires separate authorization.
