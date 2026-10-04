// The benchmark's own tests: the pure logic of bench/lib, the invariants of the
// reference set, the checks' red-on-the-shipped-fixture property, and the exact
// `genie` calls `prepare` makes (driven by a stub binary on PATH — no server, no
// model, no spend).

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

import { compareRuns, DEFAULT_THRESHOLDS, formatComparison, taskDiff } from "../bench/lib/compare.mjs";
import { answerMinutes, ownerQuestions, refOf, returns, runMetrics, taskMetrics, tokensOf, verdict, wallMinutes } from "../bench/lib/metrics.mjs";
import { loadTasks, renderTaskList, TASKS_FILE, validateTasks } from "../bench/lib/tasks.mjs";

const ROOT = path.resolve(import.meta.dirname, "..");
const RUN = path.join(ROOT, "bench", "run.mjs");
const TRAINING = path.join(ROOT, "bench", "training");
const TASKS = loadTasks();

const runCli = (args: string[], env: Record<string, string> = {}) =>
  spawnSync(process.execPath, [RUN, ...args], { cwd: ROOT, encoding: "utf8", env: { ...process.env, ...env } });

// ── the reference set ────────────────────────────────────────────────────────

test("the reference set is usable and every task has a check, an expect block, a budget and a timeout", () => {
  assert.deepEqual(validateTasks(TASKS), []);
  assert.ok(TASKS.length >= 5 && TASKS.length <= 10, `the set has ${TASKS.length} tasks`);
  for (const task of TASKS) {
    assert.match(task.check, /^node bench\/checks\/r\d+\.mjs <workspace/, `${task.id}: check`);
    assert.ok(existsSync(path.join(ROOT, task.check.split(" ")[1])), `${task.id}: the check script exists`);
    assert.equal(typeof task.expect.done, "boolean", `${task.id}: expect.done`);
    assert.ok(task.budgetUsd > 0 && task.timeoutMin > 0, `${task.id}: limits`);
    assert.ok(task.labels.includes(`bench:${task.id}`), `${task.id}: the ref label`);
  }
  assert.deepEqual(
    TASKS.filter((t) => t.ownerAnswer).map((t) => t.id),
    ["R3", "R6"],
    "only R3 and R6 carry a recommended answer for the owner",
  );
});

test("the set is rejected when a task loses its check, its expect block or its limits", () => {
  const broken = structuredClone(TASKS);
  delete (broken[0] as Record<string, unknown>).check;
  broken[1].budgetUsd = 0;
  broken[2].expect = { done: true };
  const problems = validateTasks(broken);
  assert.ok(problems.some((p) => p.includes("check is missing")), problems.join("; "));
  assert.ok(problems.some((p) => p.includes("budgetUsd")), problems.join("; "));
  assert.ok(problems.some((p) => p.includes("expect.returns")), problems.join("; "));
});

test("the task set on disk is valid JSON with the eight references", () => {
  const raw = JSON.parse(readFileSync(TASKS_FILE, "utf8"));
  assert.deepEqual(raw.tasks.map((t: { id: string }) => t.id), ["R1", "R2", "R3", "R4", "R5", "R6", "R7", "R8"]);
  assert.match(renderTaskList(TASKS), /R1 {2}Итог игнорирует скидку/);
});

test("the shipped fixture is green, so a red check is the task, not the fixture", () => {
  const green = spawnSync(process.execPath, ["--test"], { cwd: TRAINING, encoding: "utf8" });
  assert.equal(green.status, 0, `${green.stdout}\n${green.stderr}`);
});

test("every check fails on the fixture as shipped, so a green check means work was done", () => {
  for (const task of TASKS) {
    const command = task.check.replaceAll("<workspaces>", TRAINING).replaceAll("<workspace>", TRAINING);
    const out = spawnSync(command, { shell: true, cwd: ROOT, encoding: "utf8" });
    assert.notEqual(out.status, 0, `${task.id} passes on the shipped fixture: ${out.stdout}${out.stderr}`);
  }
});

test("every check passes on a copy where the tasks are solved", () => {
  const solved = mkdtempSync(path.join(os.tmpdir(), "bench-solved-"));
  cpSync(TRAINING, solved, { recursive: true });
  overlay(path.join(ROOT, "bench", "solutions"), solved);

  const green = spawnSync(process.execPath, ["--test"], { cwd: solved, encoding: "utf8" });
  assert.equal(green.status, 0, `${green.stdout}\n${green.stderr}`);
  for (const task of TASKS) {
    const command = task.check.replaceAll("<workspaces>", solved).replaceAll("<workspace>", solved);
    const out = spawnSync(command, { shell: true, cwd: ROOT, encoding: "utf8" });
    assert.equal(out.status, 0, `${task.id} fails on the solved copy: ${out.stdout}${out.stderr}`);
  }
});

/** Copy the solution tree over a copy of the fixture, leaving its README out. */
function overlay(from: string, to: string) {
  for (const entry of readdirSync(from, { withFileTypes: true })) {
    if (entry.name === "README.md") continue;
    const src = path.join(from, entry.name);
    const dst = path.join(to, entry.name);
    if (entry.isDirectory()) {
      overlay(src, dst);
      continue;
    }
    mkdirSync(path.dirname(dst), { recursive: true });
    copyFileSync(src, dst);
  }
}

// ── metrics ──────────────────────────────────────────────────────────────────

const history = [
  { at: "2026-10-04T09:00:00.000Z", actor: "genie", role: "orchestrator", event: "created", to: "inbox" },
  { at: "2026-10-04T10:00:00.000Z", actor: "clouseau", role: "analyst", event: "status", from: "refining", to: "needs_owner", note: "which number?" },
  { at: "2026-10-04T12:00:00.000Z", actor: "gshulga", role: "human", event: "status", from: "needs_owner", to: "refining" },
  { at: "2026-10-04T13:00:00.000Z", actor: "johnny5", role: "executor", event: "status", from: "in_progress", to: "review" },
  { at: "2026-10-04T14:00:00.000Z", actor: "picard", role: "reviewer", event: "status", from: "review", to: "changes_requested" },
  { at: "2026-10-04T15:00:00.000Z", actor: "johnny5", role: "executor", event: "status", from: "changes_requested", to: "in_progress" },
  { at: "2026-10-04T16:00:00.000Z", actor: "johnny5", role: "executor", event: "status", from: "in_progress", to: "review" },
  { at: "2026-10-04T17:00:00.000Z", actor: "picard", role: "reviewer", event: "status", from: "review", to: "approved" },
];

const cannedTask = {
  id: "B-3",
  title: "Ускорить экспорт",
  status: "done",
  labels: ["bench", "bench:R3"],
  children: [],
  created: "2026-10-04T09:00:00.000Z",
  updated: "2026-10-04T18:00:00.000Z",
  history,
  comments: [{ at: "2026-10-04T12:00:00.000Z", author: "gshulga", role: "human", kind: "note", text: "take the first 1000" }],
};

test("returns, owner questions and the answer latency come from the status trail", () => {
  assert.equal(returns(cannedTask), 1);
  assert.equal(ownerQuestions(cannedTask), 1);
  assert.equal(answerMinutes(cannedTask), 120, "from 10:00 to the first human word at 12:00");
  assert.equal(wallMinutes(cannedTask), 540);
  assert.equal(refOf(cannedTask), "R3");
});

test("a question nobody answered has no latency, and seconds are not counted as hours", () => {
  const waiting = { ...cannedTask, history: [cannedTask.history[1]], comments: [] };
  assert.equal(answerMinutes(waiting), null);
  const quick = {
    ...cannedTask,
    comments: [{ at: "2026-10-04T10:00:30.000Z", author: "gshulga", role: "human", kind: "note", text: "ok" }],
  };
  assert.equal(answerMinutes(quick), 0.5);
});

test("tokens of every kind are counted, and a missing spend block is zero", () => {
  assert.equal(tokensOf({ tokens: { input: 10, output: 4, cacheRead: 100, cacheWrite: 1 } }), 115);
  assert.equal(tokensOf(null), 0);
  assert.equal(tokensOf({}), 0);
});

test("a task without a branch keeps checkPassed null instead of failing it", () => {
  const metrics = taskMetrics({ task: cannedTask, spend: null, check: null });
  assert.equal(metrics.checkPassed, null);
  assert.equal(metrics.costUsd, 0);
  assert.equal(metrics.failedRuns, null, "failed runs are known per project only");
  const withBranch = taskMetrics({ task: cannedTask, spend: { spend: { cost: 1.25, tokens: { input: 2 } } }, check: { passed: false, output: "FAIL" } });
  assert.equal(withBranch.checkPassed, false);
  assert.equal(withBranch.costUsd, 1.25);
});

test("the headline numbers of a round are derived from the tasks and the project's stats", () => {
  const done = taskMetrics({ task: cannedTask, spend: null, check: { passed: true } });
  const other = taskMetrics({ task: { ...cannedTask, id: "B-4", status: "cancelled", history: [], labels: ["bench", "bench:R4"] }, spend: null, check: null });
  const run = runMetrics({ tasks: [done, other], stats: { projects: [{ usage: { spend: { cost: 4.5 } }, runs: 20, runsFailed: 2, proposals: 1 }] } });
  assert.equal(run.started, 2);
  assert.equal(run.done, 1);
  assert.equal(run.doneRate, 0.5);
  assert.equal(run.returns, 1);
  assert.equal(run.returnsPerTask, 0.5);
  assert.equal(run.costUsd, 4.5);
  assert.equal(run.costPerDoneTask, 4.5);
  assert.equal(run.runsFailed, 2);
});

test("the expected profile of a task is compared, not enforced", () => {
  const expect = { done: true, returns: [0, 1], ownerQuestions: [1, 2], splitInto: 2, proposals: [1, 3] };
  assert.equal(verdict(expect, { done: true, returns: 1, ownerQuestions: 1, children: 2, proposals: 1 }).ok, true);
  const off = verdict(expect, { done: false, returns: 4, ownerQuestions: 0, children: 1, proposals: 0 });
  assert.equal(off.ok, false);
  assert.equal(off.problems.length, 5, off.problems.join("; "));
});

// ── thresholds ───────────────────────────────────────────────────────────────

const before = { runId: "before", run: { doneRate: 1, returnsPerTask: 0.5, costPerDoneTask: 1.0 }, tasks: [] };

test("a round that keeps the numbers passes, a worse one regresses", () => {
  const same = compareRuns(before, { runId: "after", run: { doneRate: 1, returnsPerTask: 0.5, costPerDoneTask: 1.0 }, tasks: [] });
  assert.equal(same.ok, true);
  assert.deepEqual(same.regression, []);

  const worse = compareRuns(before, { runId: "worse", run: { doneRate: 0.8, returnsPerTask: 2.0, costPerDoneTask: 1.5 }, tasks: [] });
  assert.equal(worse.ok, false);
  assert.deepEqual(worse.regression, ["doneRate", "returnsPerTask", "costPerDoneTask"]);
});

test("the thresholds' edges are inclusive and the defaults are the documented ones", () => {
  assert.deepEqual(DEFAULT_THRESHOLDS, { doneRate: { maxDrop: 0.1 }, returnsPerTask: { maxIncrease: 1 }, costPerDoneTask: { maxIncreasePct: 25 } });
  const edge = compareRuns(before, { runId: "edge", run: { doneRate: 0.9, returnsPerTask: 1.5, costPerDoneTask: 1.25 }, tasks: [] });
  assert.equal(edge.ok, true, formatComparison(edge));
  const over = compareRuns(before, { runId: "over", run: { doneRate: 0.9, returnsPerTask: 1.5, costPerDoneTask: 1.26 }, tasks: [] });
  assert.deepEqual(over.regression, ["costPerDoneTask"]);
});

test("a round without a cost per done task is noted, not counted as a regression", () => {
  const noCost = compareRuns({ runId: "before", run: { doneRate: 1, returnsPerTask: 0, costPerDoneTask: null }, tasks: [] }, { runId: "after", run: { doneRate: 1, returnsPerTask: 0, costPerDoneTask: null }, tasks: [] });
  assert.equal(noCost.ok, true);
  assert.match(noCost.checks.find((c: { name: string; note?: string }) => c.name === "costPerDoneTask")?.note ?? "", /not compared/);

  const lost = compareRuns({ runId: "before", run: { doneRate: 1, returnsPerTask: 0, costPerDoneTask: 1 }, tasks: [] }, { runId: "after", run: { doneRate: 1, returnsPerTask: 0, costPerDoneTask: null }, tasks: [] });
  assert.deepEqual(lost.regression, ["costPerDoneTask"]);
});

test("the per-task diff lines the two rounds up by reference", () => {
  const beforeTasks = [{ ref: "R1", id: "B-1", status: "done", returns: 0, costUsd: 0.4, checkPassed: true }];
  const afterTasks = [
    { ref: "R1", id: "B-9", status: "cancelled", returns: 2, costUsd: 0.9, checkPassed: false },
    { ref: "R8", id: "B-10", status: "done", returns: 0, costUsd: 1.1, checkPassed: true },
  ];
  const diff = taskDiff(beforeTasks, afterTasks);
  assert.deepEqual(diff.map((d) => d.ref), ["R1", "R8"]);
  assert.equal(diff[0].statusBefore, "done");
  assert.equal(diff[0].statusAfter, "cancelled");
  assert.equal(diff[1].statusBefore, null);
  assert.equal(diff[1].checkAfter, true);
});

test("comparing writes a person-readable table and exits non-zero on a regression", () => {
  const good = path.join(os.tmpdir(), `bench-good-${process.pid}.json`);
  const bad = path.join(os.tmpdir(), `bench-bad-${process.pid}.json`);
  writeFileSync(good, JSON.stringify(before));
  writeFileSync(bad, JSON.stringify({ runId: "bad", run: { doneRate: 0.5, returnsPerTask: 3, costPerDoneTask: 2 }, tasks: [] }));

  const pass = runCli(["compare", good, good]);
  assert.equal(pass.status, 0, pass.stderr);
  assert.match(pass.stdout, /PASS: no regression/);

  const fail = runCli(["compare", good, bad]);
  assert.equal(fail.status, 1, "a regression is an exit code, so a script can gate on it");
  assert.match(fail.stdout, /REGRESSION: doneRate, returnsPerTask, costPerDoneTask/);
});

// ── the runner without a server ──────────────────────────────────────────────

const STUB = `#!/usr/bin/env node
const fs = require("node:fs");
const args = process.argv.slice(2);
fs.appendFileSync(process.env.STUB_LOG, JSON.stringify(args) + "\\n");
if (args.includes("create")) {
  const calls = fs
    .readFileSync(process.env.STUB_LOG, "utf8")
    .trim()
    .split("\\n")
    .map((line) => JSON.parse(line));
  const n = calls.filter((a) => a.includes("create")).length;
  process.stdout.write(JSON.stringify({ id: "B-" + n, slug: "bench-smoke" }));
} else if (args.includes("add")) {
  process.stdout.write(JSON.stringify({ slug: "bench-smoke", prefix: "B" }));
} else {
  process.stdout.write(JSON.stringify({ ok: true, tasks: [] }));
}
`;

type Call = string[];

/** The arguments that follow every `flag` of one call. */
const valuesAfter = (call: Call, flag: string): string[] =>
  call.map((arg, i) => (arg === flag ? call[i + 1] : null)).filter((v): v is string => v !== null);

function stubEnv() {
  const tmp = mkdtempSync(path.join(os.tmpdir(), "bench-stub-"));
  const bin = path.join(tmp, "bin");
  mkdirSync(bin);
  const stub = path.join(bin, "genie");
  writeFileSync(stub, STUB);
  chmodSync(stub, 0o755);
  const log = path.join(tmp, "calls.log");
  const runs = path.join(tmp, "runs");
  return { tmp, log, runs, env: { PATH: `${bin}${path.delimiter}${process.env.PATH}`, STUB_LOG: log, BENCH_RUNS_DIR: runs, GENIE_URL: "http://127.0.0.1:7500", GENIE_TOKEN: "stub-token" } };
}

test("prepare registers the project, creates the eight parked tasks and releases them on --start", () => {
  const { log, runs, env } = stubEnv();
  const out = runCli(["prepare", "--run", "smoke", "--start"], env);
  assert.equal(out.status, 0, out.stderr || out.stdout);

  const calls: Call[] = readFileSync(log, "utf8")
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line))
    .filter((call: Call) => !call.includes("--version"));

  const repo = path.join(runs, "smoke", "training");
  assert.deepEqual(calls[0], ["--project", "bench-smoke", "project", "add", "bench-smoke", "--repo", repo, "--prefix", "B", "--json"]);
  assert.equal(calls.filter((c) => c.includes("create")).length, 8, "one task per reference task");

  const creates = calls.filter((c) => c.includes("create"));
  creates.forEach((call, i) => {
    assert.deepEqual(call.slice(0, 2), ["--project", "bench-smoke"]);
    assert.deepEqual(call.slice(2, 5), ["task", "create", TASKS[i].title], `${TASKS[i].id}: the reference title`);
    assert.ok(call.includes("--draft"), `${TASKS[i].id}: parked as a draft`);
    assert.deepEqual(valuesAfter(call, "--type"), ["task"]);
    assert.deepEqual(valuesAfter(call, "--label"), ["bench", `bench:${TASKS[i].id}`], `${TASKS[i].id}: labels`);
    assert.equal(valuesAfter(call, "--ac").length, TASKS[i].ac.length, `${TASKS[i].id}: one --ac per criterion`);
    assert.ok(valuesAfter(call, "--description")[0].includes(TASKS[i].goal), `${TASKS[i].id}: the description carries the goal`);
  });

  const released = calls.filter((c) => c.includes("status") && c.includes("inbox"));
  assert.equal(released.length, 8, "every task is released to the orchestrator");
  for (let i = 1; i <= 8; i += 1) {
    assert.deepEqual(released[i - 1], ["--project", "bench-smoke", "task", "status", "inbox", "--task", `B-${i}`, "--json"]);
  }

  const run = JSON.parse(readFileSync(path.join(runs, "smoke", "run.json"), "utf8"));
  assert.equal(run.runId, "smoke");
  assert.equal(run.project, "bench-smoke");
  assert.equal(run.released, true);
  assert.equal(run.tasks.length, 8);
  assert.match(run.fixtureHash, /^[0-9a-f]{64}$/);
  assert.match(run.baselineSha, /^[0-9a-f]{40}$/);
  assert.equal(run.preparedFrom.checkout, ROOT, "the checkout the hashes come from is recorded");
  assert.ok(Object.keys(run.hashes.files).some((f) => f.startsWith("agents/")), "the role prompts are hashed");
  assert.ok(Object.keys(run.hashes.files).some((f) => f.startsWith("config/teams/")), "the team templates are hashed");
  assert.deepEqual(Object.keys(run.recommendedAnswers), ["R3", "R6"]);
  assert.equal(existsSync(path.join(repo, ".git")), true, "the fixture is a git repository with a baseline commit");
  assert.equal(existsSync(path.join(repo, "src", "report.js")), true, "the fixture is complete");
});

test("the release call follows the CLI's grammar, checked against the real binary when it is there", (t) => {
  const wrongGrammar = /unexpected argument|Usage: genie task status/;

  const probe = spawnSync("genie", ["--version"], { encoding: "utf8" });
  if (probe.error) {
    t.skip("genie is not on PATH: the grammar is asserted against the stub only");
    return;
  }
  // An empty data directory: the command must fail on the missing server, not on
  // the arguments. No GENIE_URL/GENIE_TOKEN, so no running server is reached.
  const env = { ...process.env, GENIE_URL: "", GENIE_TOKEN: "" };
  const data = mkdtempSync(path.join(os.tmpdir(), "bench-data-"));
  const right = spawnSync("genie", ["--data", data, "task", "status", "inbox", "--task", "B-1"], { encoding: "utf8", env });
  assert.doesNotMatch(`${right.stdout}${right.stderr}`, wrongGrammar, "`task status <STATUS> --task <id>` must parse");
  const wrong = spawnSync("genie", ["--data", data, "task", "status", "B-1", "inbox"], { encoding: "utf8", env });
  assert.match(`${wrong.stdout}${wrong.stderr}`, wrongGrammar, "the old argument order really is refused by the CLI");
});

test("releasing a round twice is refused instead of moving done tasks back to the inbox", () => {
  const { log, env } = stubEnv();
  assert.equal(runCli(["prepare", "--run", "twice", "--start"], env).status, 0);
  const before = readFileSync(log, "utf8").trim().split("\n").length;

  const again = runCli(["prepare", "--run", "twice", "--start"], env);
  assert.equal(again.status, 2);
  assert.match(again.stderr, /already released/);
  assert.equal(readFileSync(log, "utf8").trim().split("\n").length, before, "no task was moved");
});

test("prepare without --start parks the tasks in draft and writes the round", () => {
  const { log, runs, env } = stubEnv();
  const out = runCli(["prepare", "--run", "parked"], env);
  assert.equal(out.status, 0, out.stderr || out.stdout);
  const calls: Call[] = readFileSync(log, "utf8").trim().split("\n").map((line) => JSON.parse(line));
  assert.equal(calls.filter((c) => c.includes("inbox")).length, 0, "nothing is released without --start");
  const run = JSON.parse(readFileSync(path.join(runs, "parked", "run.json"), "utf8"));
  assert.equal(run.released, false);
});

test("prepare --dry-run meets no server and changes nothing", () => {
  const { runs, env } = stubEnv();
  const out = runCli(["prepare", "--run", "planned", "--dry-run", "--start"], { ...env, STUB_LOG: path.join(os.tmpdir(), `unused-${process.pid}.log`) });
  assert.equal(out.status, 0, out.stderr);
  assert.match(out.stdout, /would run:/);
  assert.match(out.stdout, /task create/);
  assert.match(out.stdout, /recommended answers for the owner:/);
  assert.match(out.stdout, /R3:/);
  assert.equal(existsSync(path.join(runs, "planned")), false, "a dry run writes nothing");
});

test("the working server on port 7420 is refused without --i-know", () => {
  const { env } = stubEnv();
  const out = runCli(["prepare", "--run", "danger"], { ...env, GENIE_URL: "http://127.0.0.1:7420" });
  assert.equal(out.status, 2);
  assert.match(out.stderr, /refusing http:\/\/127\.0\.0\.1:7420/);
  assert.match(out.stderr, /--i-know/);
});

test("the runner asks for a token and a URL instead of guessing the working server", () => {
  const { env } = stubEnv();
  const noUrl = runCli(["prepare", "--run", "x"], { ...env, GENIE_URL: "" });
  assert.equal(noUrl.status, 2);
  assert.match(noUrl.stderr, /set GENIE_URL or pass --url/);
  const noToken = runCli(["prepare", "--run", "x"], { ...env, GENIE_TOKEN: "" });
  assert.equal(noToken.status, 2);
  assert.match(noToken.stderr, /set GENIE_TOKEN/);
});
