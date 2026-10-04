# The agent benchmark

A fixed set of small tasks on a throwaway training project, worked by a genie
team, measured **before and after** a change to a role, a prompt, a skill or a
team template. It answers the first gap of `docs/platform/proposals.md` ("Нет проверки
качества агентов"): did the tasks reach «Готово», how often did the work come
back from review, what did it cost.

The weekly retrospective (and a later CI) gates an edit on `compare`: a change is
applied only when the second round is no worse than the first.

## What is measured

Per round (headline, from `genie server stats --json`):

| number | meaning |
|---|---|
| `doneRate` | tasks that reached `done` / tasks started |
| `returns` | `review → changes_requested` transitions across the set (and per task) |
| `costUsd`, `costPerDoneTask` | the models' spend, and the spend per done task |

Per task: `done`, `returns`, `ownerQuestions`, `answerMinutes`, `checkPassed`,
`costUsd`, `tokens`, `wallMinutes`, the number of `children` (splitting) and a
verdict against the profile the task is expected to produce.

Two things are known only per project, not per task — the API does not name the
task a failed agent run or a knowledge proposal belongs to:

- `failedRuns` is recorded at the round level (`run.runsFailed`); a task's
  `failedRuns` is `null`;
- `proposals` is the project's count, and R7's verdict is checked against it.

## The reference set (`bench/reference/tasks.json`)

| id | what it tests | the move a healthy round makes |
|---|---|---|
| R1 | a small, well-specified bug (the total ignores a discount) | straight to done, no questions |
| R2 | a feature that must come with a test | done, at most one return |
| R3 | an underspecified task ("make the export faster") | the analyst **asks the owner** before working |
| R4 | a constraint that lives only in the project's docs | the agent **finds `docs/logging.md`** and follows it |
| R5 | a regression trap (a quantity of `0` is a quantity) | the reviewer catches the naive fix and returns the work |
| R6 | a false constraint in the task text (CommonJS in an ESM package) | the agent **asks** instead of rewriting the package |
| R7 | a user-visible change plus documentation | done, a knowledge page is proposed |
| R8 | three unrelated changes in one task | split into subtasks instead of one blob |

Every task carries genie acceptance criteria, an objective `check`, the expected
profile (`expect`), a `budgetUsd`, a `timeoutMin` and — for R3 and R6 — a
recommended answer for the owner.

`<check>` is run by `collect` in a read-only checkout of the branch the team
produced:

- `check: "node bench/checks/rN.mjs <workspace>"` — the task's own branch;
- `check: "node bench/checks/r8.mjs <workspaces>"` — the task's branch and its
  children's, because R8 is expected to be split.

A task that never got a branch has `checkPassed: null` (not a failure).

## The training project (`bench/training/`)

A tiny zero-dependency ESM library ("shelf"): inventory lines, a store, totals, a
report, a CSV export, logging. Tests are plain `node --test`. The fixture is
green as shipped; each task's check fails on it, so a passing check means work was
done, and a red one means the work is missing or the fixture was broken.

Rounds are copies: `prepare` copies the fixture, `git init`s it and makes one
baseline commit, so two rounds start from a byte-identical tree.

## Running a round

A **person** starts and stops a round; the script never answers a question and
never waits for one. It talks to the pilot instance (G-131) over the API with an
admin token — never to the working server on port 7420, which is refused unless
`--i-know` is passed — and never reads the server's data directory.

```sh
export GENIE_URL=http://127.0.0.1:7500   # the pilot instance, not 7420
export GENIE_TOKEN=$(genie user token gshulga)  # an admin of the pilot instance

node bench/run.mjs list                          # the reference set
node bench/run.mjs prepare --run 2026-10-05      # copy the fixture, register bench-2026-10-05, park the tasks
node bench/run.mjs prepare --run 2026-10-05 --start   # release them to the orchestrator
node bench/run.mjs next --run 2026-10-05         # who waits for the owner, and the recommended answer
node bench/run.mjs status --run 2026-10-05       # the one-screen state of the round
node bench/run.mjs collect --run 2026-10-05 --note "before: current roles"
node bench/run.mjs compare bench/results/before.json bench/results/after.json
```

`prepare` prints the round, the tasks and the recommended answers for R3/R6;
`--dry-run` prints the plan and touches nothing (no server, no token needed).
Releasing a round twice is refused — a released round is collected, not released
again; `--force` is the escape hatch for exactly that case (it moves finished
tasks back to the inbox). A round is never **re-created in place**: the record of
a finished round is evidence, so a new round gets a new id — and a refused
`project add` can therefore never destroy `run.json`.

`collect` is a **snapshot**: tasks still open are recorded as not done, and the
person stops the round before running it. Stopping means **accepting the
finished tasks** (moving them to `done` in the web or with
`genie task accept <id> --note "…"`):
with the default `assisted` autonomy agents leave tasks in `review` or
`approved`, and a `collect` taken then has `doneRate` 0 in both rounds and
`compare` says nothing. Close the round, then collect.

Between two rounds only the thing under test may change. `run.json` records a
hash of `agents/*.md`, `config/teams/*.json`, `config/default.json` (with the
role models) and the `genie` version, plus the fixture hash — so a diff that
mixed two changes is visible. Those hashes are the **runner's own checkout**
(`preparedFrom` in `run.json` says which): prepare and collect from the
instance's checkout with the instance's binary, otherwise the guarantee holds
for the wrong tree. What the server actually runs is not exposed by the API —
if a round ever looks mixed, compare `preparedFrom` with the instance's install.

## Thresholds (`bench/thresholds.json`)

`compare` prints a before/after table and exits non-zero when a rule is broken:

| rule | default |
|---|---|
| `doneRate` | not more than `0.1` below the before round |
| `returnsPerTask` | not more than `1` above |
| `costPerDoneTask` | not more than `25%` above |

A round without a cost per done task is noted and skipped, not counted as a
regression.

## Verifying the benchmark itself (no server, no spend)

```sh
npm test                                  # test/bench.test.ts
node bench/run.mjs prepare --run prova --dry-run
```

The tests cover the pure logic (`bench/lib`) on canned payloads, the invariants
of the reference set, the exact `genie` calls `prepare` makes (a stub `genie` on
`PATH`, plus a grammar probe against the real binary when it is on `PATH`), the
refusal of port 7420, the `compare` exit codes, and both directions of the
checks: **every check fails on the shipped fixture** and **every check passes on
a copy where the tasks are solved** (`bench/solutions/`, overlaid onto a copy of
the fixture in a temp directory). Anything a check cannot satisfy, or a check
that starts passing by itself, fails `npm test`.

A real round on the pilot instance is the evidence of the round itself, not of
this code.

## Adding a reference task

1. add the shape to `bench/training/` (and keep `node --test` there green);
2. add `bench/checks/rN.mjs`, using `bench/checks/util.mjs`; make it fail on the
   shipped fixture;
3. solve the task in `bench/solutions/` (same paths; `README.md` there explains
   it) so the check's other direction stays covered;
4. add the entry to `bench/reference/tasks.json` with its `check`, `expect`,
   budget and timeout;
5. `npm test` — the set's invariants, the red-on-fixture and the
   green-on-solved tests cover it.

The benchmark does not grow the `genie` CLI: a number the API does not return
becomes a task of its own, not a new command here.
