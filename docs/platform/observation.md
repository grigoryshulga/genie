---
title: Observation
summary: Check whether delivered work did what it was meant to do, with guardrails, measurements, incidents, and follow-up tasks.
type: guide
status: current
---

# Observation

Observation looks at delivered work after it ships. An **outcome case** links one or more done tasks to a contract. Genie reads measurements from the project's metrics or error sources, opens an **incident** when a guardrail is crossed, and concludes the case as `confirmed`, `negative`, or `inconclusive`.

Observation is separate from the task lifecycle. It never moves a task, blocks delivery, or changes project memory. The server runs it on a schedule, so no agent stays alive while a case waits. Follow-up work goes through the normal task services; see [follow-up tasks](#follow-up-tasks). The design is in the [Phase 1 specification](observation-plane.md); Russian UI labels are in [the glossary](https://github.com/grigoryshulga/genie/blob/main/GLOSSARY.md#observation-plane).

## Concepts

| Term | Meaning |
| --- | --- |
| Case (`O-<n>`) | One observation of one or more tasks, with a policy and a contract. |
| Policy | `guardrails` watches limits that must not be crossed. `verify` also checks expected changes. |
| Contract | Versioned expectations: hypotheses, guardrails, and windows. Written before any evidence is evaluated. |
| Hypothesis | An expected change or bound, such as "p95 latency falls by 20%". |
| Guardrail | A limit that must not be crossed, such as "error rate rises by 0.1 percentage points at most". |
| Anchor | The moment observation started. Windows are measured from it. |
| Evidence record | One measurement with its source, window, value, and trust level. |
| Trust level | `measured` (collected from a source), `reported` (entered by a person), or `agent` (reported by an agent). The evaluator ignores `agent` records. |
| Incident (`I-<n>`) | An open guardrail violation of a case. |
| Evaluation | A verdict over the case's current evidence. A final evaluation concludes the case. |

Each case shows its key, status, result, and confidence in the Observations page. Its keys (`O-1`, `I-1`) also work on the command line, where `1` is accepted as well.

## Lifecycle

A case moves through these states:

1. **Draft** (`proposed`): created with its tasks, and a contract if you have one.
2. **Waiting** (`waiting`): armed. Arming needs a contract, at least one task, and sources that exist with their secrets set. The case lists any remaining problems, so you can see why it cannot be armed.
3. **Observing** (`observing`): started. The start happens automatically when every linked task is done, when you start it manually, or, for a case that waits for a deployment, when the deployment is recorded.
4. **Evaluating** (`evaluating`): a short state while the scheduler evaluates the case.
5. **Concluded** (`concluded`) with a result, or **cancelled** (`cancelled`) with a reason that becomes its conclusion.

A cancelled task does not count as done. Start the case manually if its work still matters.

The full transition table and who may trigger each one are in §6 and §14 of the [specification](observation-plane.md).

## Contract format

A contract is JSON. This example is for a `verify` case: it checks a hypothesis and watches a guardrail. A `guardrails` case has guardrails and no hypotheses.

```json
{
  "hypotheses": [
    {
      "id": "checkout_latency_p95",
      "source": "prom",
      "query": "histogram_quantile(0.95, sum(rate(http_request_duration_seconds_bucket{route=\"/checkout\"}[$__range])) by (le))",
      "samplesQuery": "sum(increase(http_requests_total{route=\"/checkout\"}[$__range]))",
      "expect": { "change": "-20%" }
    }
  ],
  "guardrails": [
    {
      "id": "error_rate",
      "source": "prom",
      "query": "100 * sum(rate(http_requests_total{code=~\"5..\"}[$__range])) / sum(rate(http_requests_total[$__range]))",
      "maxChange": "+0.1pp"
    }
  ],
  "observation": {
    "start": "tasks_done",
    "baselineWindow": "7d",
    "minimumWindow": "24h",
    "preferredWindow": "72h",
    "minimumSamples": 10000
  },
  "followup": "manual"
}
```

Field names are camelCase. Unknown fields are rejected, so a typo fails at once rather than being ignored.

- **Criteria.** Each hypothesis and guardrail has an `id` (lowercase letters, digits, and `_`), a `source` (a configured source name, or `manual` for reported values), and a `query` in the source's language. For Prometheus-compatible sources, `$__range` is replaced by the measured window length, for example `86400s`. A `samplesQuery` says how many samples a value covers.
- **Expectations.** A hypothesis sets `expect` to exactly one of `change`, `min`, or `max`. A guardrail sets at least one of `maxChange`, `minChange`, `max`, or `min`.
- **Windows.** `start` is `tasks_done` (default), `deployment`, or `manual`. `minimumWindow` is required; nothing concludes before it, unless a person requests a final evaluation. `preferredWindow` (default: the minimum) is when a final evaluation runs on its own. `baselineWindow` (default `7d`) is the period before the anchor used for change comparisons. `minimumSamples` is the sample count a hypothesis needs before its value counts.
- **Schedule.** `guardrailInterval` (default `15m`, at least one minute) sets how often guardrails are measured. `evaluationInterval` (default `6h`) sets how often the case is evaluated. `guardrailWindow` (default `1h`, or the preferred window if shorter) is how far back each guardrail check looks.
- **Durations** are `30s`, `15m`, `24h`, or `2d`.
- **Follow-up.** `followup` is `manual` (default) or `auto`; see [follow-up tasks](#follow-up-tasks).

### Change grammar

Changes in `expect.change`, `maxChange`, and `minChange` are signed:

| Text | Meaning |
| --- | --- |
| `-20%` | Relative change: the value is 20% below its baseline. |
| `+0.1pp` | Absolute change: the value is 0.1 percentage points above its baseline. |
| `+5` | Absolute change in the metric's own units. |

The sign is required. Zero is rejected for hypotheses, which must expect a change, but allowed for guardrails, where `+0` means "must not grow".

**Note on `pp`.** `pp` means the metric is already a percentage. The absolute delta is the value minus the baseline, in the metric's own units. For an error rate expressed as a percentage, `maxChange: "+0.1pp"` is violated when the rate rises by more than 0.1 points. `pp` does not scale the metric. For a metric that is a fraction between 0 and 1, write the threshold in its own units, such as `+0.001`.

Changes need a baseline. A baseline is measured once per criterion and contract version, over the `baselineWindow` before the anchor.

The validation rules, including policy requirements, id uniqueness, query length, and the limits of 20 criteria and 90-day windows, are in §5.3 of the [specification](observation-plane.md). An invalid contract is rejected with every problem listed.

## Sources

A source is a metrics or error service the server queries. Two kinds exist:

- **Prometheus-compatible** (`prometheus`): Prometheus, Thanos, VictoriaMetrics, or Mimir. Queries are PromQL instant queries; the value must be a single series, so aggregate with `sum()` or similar.
- **Sentry-compatible** (`sentry`): Sentry's events API. Queries use Sentry's search syntax. `organization` is required; `field` defaults to `count()`, and `dataset` to `errors`.

Set a source up in **Settings → Project settings → Observation**, or on the command line. Only project administrators manage sources and their secrets. Each source has:

- a name (lowercase letters, digits, `-`, and `_`; `manual` is reserved);
- a base URL, `http://` or `https://`, without credentials, query, or fragment;
- an authentication mode: `none`, `bearer` (a token), or `basic` (`user:password`);
- a timeout from 1 to 120 seconds (default 30).

The secret is stored sealed on the server and never shown again; the UI and API show only a hint. Credentials in a URL are refused, so use the secret field instead.

**The server makes these requests from its own network.** Any URL the server can reach can be configured as a source by an administrator, so configure only metrics and error services that the server should query. Check each source with a test query before arming a case.

```bash
genie observe sources
genie observe source-add prom --kind prometheus --url https://prometheus.internal --auth bearer
printf '%s' "$PROM_TOKEN" | genie observe source-secret prom --value -
genie observe source-test prom --query 'sum(rate(http_requests_total[$__range]))' --window 1h
```

Removing a source also removes its secret. A case that still names the source records a collection error (`unknown source`) on each pass until its contract changes or the source returns. Arming refuses a case whose contract names an unknown source, or a source with `bearer` or `basic` auth that has no secret.

## Evidence and measurements

The scheduler measures non-manual criteria over their windows: a baseline before the anchor, guardrails over the recent `guardrailWindow`, and hypotheses from the anchor to now. A failed measurement is stored as an evidence record with an error, so the history shows what was tried.

People and the project manager can add measurements by hand:

```bash
genie observe report O-1 --criterion checkout_latency_p95 --value 0.42 --baseline 0.55 --samples 12000 --note "Load test, staging"
```

Manual records are `reported` when a person adds them and `agent` when the project manager does. Only `reported` and `measured` records count in evaluation. A reported guardrail violation opens its incident at once, like a measured one. Evidence is kept per contract version, so a revised contract does not reuse records from an older one.

## Guardrails and incidents

Each guardrail is checked at its interval. A violation opens an incident immediately, unless one is already open for the same guardrail; a repeated violation adds evidence but no second incident. Incidents are listed in the case and in `genie observe incidents`.

The case does not conclude `negative` the moment an incident opens. Conclusion waits for an evaluation at or after the minimum window, which gives people time to judge a false positive.

An incident has three outcomes:

- **Open.** The violation counts in evaluation.
- **Resolved** (`genie observe resolve I-1 --note ...`). Someone fixed or accepted the cause. The violation still counts in the evaluation, because it happened.
- **Dismissed** (`genie observe dismiss I-1 --note ...`). Only a person may dismiss, and a note is required. Dismissal records that the violation was a **false positive**, such as a broken measurement or a known one-off. The guardrail then counts as `ok` in evaluation, even when its measurement breached the limit. A new violating measurement opens a new incident.

Dismissal is not a way to close a real regression. Resolve the incident and create a follow-up task instead.

## Evaluation

An evaluation looks only at the current contract version's evidence and records a verdict. The rules, in order:

1. **Before the minimum window**, nothing concludes. A violation is still recorded as an incident, but the case stays observing. A person's final request is the exception.
2. **Any violated guardrail** gives `negative` at evaluation.
3. **Policy `guardrails`.** A final evaluation gives `confirmed` when every guardrail is `ok`, and `inconclusive` otherwise. Before the preferred window the case keeps observing.
4. **Policy `verify`.** `confirmed` as soon as every hypothesis is met and every guardrail is `ok`. Otherwise, a final evaluation gives `negative` when a hypothesis is not met, and `inconclusive` in every other case, such as a criterion with no usable evidence.

A criterion is judged on its latest usable record. A hypothesis with too few samples, no baseline, or no evidence is `insufficient`, and so the case keeps observing. A final evaluation never returns `insufficient`.

Confidence is `high` when the records behind a conclusion are all measured, and `medium` when any is reported. `inconclusive` is `low`. Each conclusion ends with a deterministic English summary that always states `attribution: not assessed`. Observation finds correlation, not cause: a conclusion does not say the task caused the result.

The details of each rule are in §8 and §9 of the [specification](observation-plane.md).

## Follow-up tasks

A follow-up is an ordinary task created through the task services, with the caller's identity, label `observation`, and a description linking the case, its tasks, and the incident evidence. Its type is `bug` when it answers an incident, and `task` otherwise.

```bash
genie observe followup O-1 --incident I-1 --title "Fix checkout 5xx regression"
```

With `followup: auto` in the contract, the server creates one follow-up for each new incident, once, as the observation actor. Repeated violations of an open incident create no more tasks. A `negative` or `inconclusive` conclusion never creates a task automatically: the notification goes to the project manager, who decides what follows.

## Who can do what

People with write access and the project manager agent manage cases. Task managers and team members can read observation data but cannot change it. Dismissal and the final evaluation are reserved for people.

| Action | People (write) | Project manager | Task manager, member, job |
| --- | --- | --- | --- |
| Create, update, arm, start, cancel a case; set its contract before start | Yes | Yes | No |
| Revise a contract after start (with a reason) | Yes | No | No |
| Record a deployment; request an evaluation | Yes | Yes | No |
| Request a final evaluation | Yes | No | No |
| Report a measurement | Yes (`reported`) | Yes (`agent`, not counted) | No |
| Resolve an incident; create a follow-up | Yes | Yes | No |
| Dismiss an incident | Yes | No | No |
| Manage sources and secrets | Project administrators only | No | No |

The server enforces every row. The full matrix is in §14 of the [specification](observation-plane.md).

## Command line

Run `genie observe --help` for the full list; [CLI and MCP](../reference/cli.md#observe) gives the access level of each command. A typical run:

```bash
genie observe create --title "Checkout latency" --policy verify --task G-7 --contract-file checkout.json
genie observe arm O-1
genie observe show O-1                # status, arm problems, evaluations, incidents, recent evidence
genie observe evidence O-1 --criterion checkout_latency_p95
genie observe incidents --status open
genie observe evaluate O-1            # request an evaluation now; --final concludes it (people only)
```

Changing a contract after observation started needs a reason, and only a person may do it:

```bash
genie observe contract O-1 --file checkout-v2.json --reason "Route renamed; the query follows the new label"
```

Deployments are recorded by hand. They start a case that waits for one, and later deployments are kept for the record only:

```bash
genie observe deploy O-1 --ref v1.4.2 --environment production
```

Add `--json` to any command for the API response.

## Automations and journal events

Every observation event has the subject `observation:<id>`, not a task, so the automation engine finds no task for it and `task` is `null` in the templates (and in `event.task`). Payload fields are top-level template fields under their own names, with one exception: `task` is always the engine's task context, so a payload `task` is hidden at the top level. `followup_task.created`, for example, carries the created follow-up task in its `task` field, which is read as `event.payload.task`. Read payload fields under `event.payload` when in doubt. The event names are listed in [automations](automations.md#triggers). Payload details are in §12 of the [specification](observation-plane.md).

For example, to notify the project's administrators when an incident opens:

```json
{
  "name": "Observation incident",
  "on": { "event": "incident.created" },
  "limits": { "concurrency": 1, "maxRunsPerHour": 10 },
  "steps": [
    {
      "id": "tell",
      "notify": {
        "to": ["project.admins"],
        "title": "{{ incidentKey }} opened",
        "text": "Review the incident in Genie."
      }
    }
  ]
}
```

Built-in notifications already cover incidents and conclusions, so add a rule only for extra delivery or a different audience.

## Notifications

Genie sends these notifications through the project's notification channels, each linking to the case page:

- **Incident** (`observation.incident`): the project's administrators and the case's author, when the author is a person. The project manager also receives mail. Repeated notices for the same incident are deduplicated.
- **Conclusion** (`observation.concluded`): the same people, and the project manager receives mail with the summary and the follow-up command.
- **Failing collection** (`observation.failing`): the project's administrators, after three consecutive passes with a collection error. A case notifies once per streak, and again after a new one.

A failing source is not hammered: a failed case waits 10 seconds before the next attempt. The last error appears on the case, and the failure count resets after a successful pass.

## Limits

- At most 20 hypotheses and 20 guardrails per contract; queries up to 4000 characters.
- Windows up to 90 days; a guardrail interval of at least one minute.
- Source responses up to 1 MiB; timeouts up to 120 seconds. Sources are polled one case at a time, so a slow source delays the others.
- A guardrail measured every 15 minutes records about 96 measurements a day. Phase 1 has no evidence retention or downsampling.
- A deployment must be no more than 30 days old when recorded.
- Titles are at most 200 characters.
- Phase 1 has no deployment provider integration, no product analytics, and no attribution. Deployments are recorded explicitly, and sources are polled rather than pushed.

The complete list of limits and their rationale is in §19 of the [specification](observation-plane.md).
