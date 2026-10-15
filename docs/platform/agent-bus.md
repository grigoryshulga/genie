---
title: Agent runtime
summary: Mail delivery, live pi sessions, interruption, supervision, and runtime settings.
type: reference
status: current
---

# Agent runtime

Team members and project orchestrators run as long-lived `pi --mode rpc` processes. The Rust supervisor controls them; the `genie-bus` pi extension delivers mail between model steps. One-shot automation jobs and non-RPC harnesses run as individual turns.

## Mail delivery

A step consists of a model response and its tool calls. The extension fetches mail after a step and before the agent settles. Busy agents receive mail at the next boundary; idle agents wake immediately.

| Level | Delivery |
| --- | --- |
| `interrupt` | Abort the current step and deliver first. Only people and the orchestrator can interrupt. |
| `high` | Next step boundary, ahead of normal mail. |
| `normal` | Next step boundary. |
| `low` | Same boundary, at the end of the batch. |

A long-running tool delays ordinary mail until it finishes. Use interruption when waiting is unacceptable.

Mail is leased until the model is about to see it. A crashed process returns unacknowledged mail to the queue; a stuck lease expires after two minutes. The conversation records delivered IDs so recovery does not insert the same message again.

## Batches and questions

A delivery is limited to `runtime.deliveryBudget` characters, default 8000. Messages longer than 2000 characters are shortened with a command to read the rest. Kickoff, interrupts, human messages, blockers, and questions take priority. The orchestrator receives a digest of routine team correspondence.

```bash
genie mail send <recipient> "Build is passing" --topic build
genie mail ask <recipient> "Which endpoint should I use?" --timeout 180
genie mail reply <id> "Use /api/orders"
genie mail read <id>
genie team peek <member> --deep
```

A topic replaces an undelivered message from the same sender to the same recipient on that topic. A timed-out question can still receive a later reply through mail.

## Control

The team screen shows current tools, status, pending mail, and conversation history. People can pause, resume, restart, interrupt, and change a member's model.

```bash
genie team board
genie team interrupt <team> <member> "Stop the build and inspect the failing test"
genie team pause <team> <member>
genie team resume <team> <member>
genie team turns
```

`GET /api/agents` exposes the board and delivery latency. Deep conversation reads use `GET /api/agents/<team>/<member>/peek?deep=1&limit=12` with a maximum of 60 messages.

## Supervision

- Failed turns and crashed processes retry with backoff. After `maxAttempts` consecutive failures, the member stays in `error`, keeps its mail, and notifies the orchestrator.
- A step with no activity beyond `turnTimeoutSecs` is interrupted. Five repeated identical tool calls trigger a possible-loop report.
- An active team that is idle with no pending mail or human/CI wait triggers a stall report after `stallSecs`. The report is deduplicated across server restarts.
- Idle sessions stop after `idleStopSecs` and restart on mail. At the session limit, the longest-idle session yields capacity.
- On server restart, orphan processes stop, outstanding leases recover, and interrupted agents receive a continuation message.

Logs are under `<data>/runtime/<project>/<agent>/`. See [operations](operations.md#health-and-logs).

## Runtime settings

Configure `runtime` in `<data>/config.json`:

| Key | Default | Meaning |
| --- | --- | --- |
| `mode` | `auto` | `sessions`, `turns`, or sessions with the standard command in `auto`. |
| `maxSessions` | 12 | Concurrent live sessions. |
| `idleStopSecs` | 300 | Idle time before a session stops. |
| `maxConcurrent` | 4 | Concurrent turns; capacity is reserved for the orchestrator. |
| `maxAttempts` | 3 | Consecutive failed attempts before error. |
| `turnTimeoutSecs` | 1800 | Step/turn watchdog timeout. |
| `askTimeoutSecs` | 180 | Default wait for an agent reply. |
| `deliveryBudget` | 8000 | Characters per mail delivery. |
| `nodeHeapMb` | 2048 | V8 heap cap for npm pi; does not apply to Pi-Bolt. |

Custom `command` and `sessionCommand` argument groups support role prompts, model, skills, and guard placeholders. Preserve the guard and `--no-approve` when replacing a pi command, so repository resources cannot bypass Genie configuration. Use `mode: turns` for a harness without the live-session protocol.

## Interactive orchestrator

```bash
genie orchestrate --project shop
genie orchestrate --project shop -- --continue
```

This launches your terminal pi with the project's orchestrator prompt and a leased orchestrator token. The server orchestrator waits while you hold the console. Mail arrives between steps and wakes idle sessions. `/genie` toggles the task board; `/genie agents` shows agents.

The lease renews while pi runs, releases on exit, and expires after a lost connection. There is no need to change project autonomy. The server then resumes handling the mailbox.
