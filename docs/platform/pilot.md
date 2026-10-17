---
title: Run a pilot
summary: Introduce Genie to a small team and evaluate reliability and useful output.
type: runbook
status: current
---

# Run a pilot

Start with one real project and a small team. Use `assisted` autonomy, independent review, and human merging. The pilot should show that people can use Genie daily and that its output meets their acceptance criteria.

## Prepare

- Run `genie doctor` as the service user and resolve its failures.
- Set models, provider keys, Git identity, repository policies, and a default integration instruction.
- Verify the project's dependency install, build, and tests inside the agent environment.
- Create user accounts, project invitations, and a responsible person for each task.
- Configure a knowledge space and its publication policy.
- Set team concurrency, task/epic/daily budgets, backup retention, and a restore location.
- Test one notification channel and an owner-question reply.

Use a separate data directory and port for experiments. The supplied `deploy/pilot.config.json` defines a separate instance. The optional `deploy/pilot.sh` helper requires Bitwarden's `bws-run` launcher and a LiteLLM key; read its header and environment variables before running it. Do not run destructive drills against the team's live data.

## First work

Choose a small, well-understood task with a verifiable result. Follow it from inbox through clarification, implementation, review, merge, and human closure. Check that:

- Agents receive the right repository and knowledge context.
- Review findings return to the executor.
- Questions reach the responsible person and answers reach the team.
- CI failures are visible and prevent an unsupported completion.
- The result is integrated and its documentation updated.

Only then add more tasks and increase concurrency.

## Recovery drills

The deployment directory includes:

| Script | Purpose |
| --- | --- |
| `deploy/rollback-drill.sh` | Upgrade and restore a temporary data copy using old and new binaries. |
| `deploy/crash-drill.sh` | Kill an agent/server during work and check mail and session recovery with a real pi/model setup. |
| `deploy/load-drill.sh` | Exercise teams, mail, automations, and web traffic with a fake model; report latency. |

Read each script's `--help` for required paths and credentials. Use temporary data, verify a backup restore, and confirm that no orphan agent processes remain after a restart.

## Evaluate

Record a baseline and review it weekly:

```bash
genie stats --days 7 --project shop --json
```

Track completed tasks, lead time, review returns, owner questions and response time, failed agent runs, model costs, and recovery incidents. Inspect actual accepted changes as well as counts. Set numerical success thresholds with the team before the pilot; do not infer quality from the number of tasks closed.

Feed recurring failures into [the roadmap](roadmap.md) before widening autonomy.
