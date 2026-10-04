#!/usr/bin/env node
// The genie agent benchmark: a fixed set of tiny tasks on a throwaway training
// project, run by a team, measured before and after a change to the roles or
// prompts. A person starts and stops a round; this script prepares the round,
// shows what waits for the owner, takes a snapshot and compares two snapshots.
// It never answers a question, never waits for a round and never spends by itself.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cpSync, existsSync, globSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { compareRuns, DEFAULT_THRESHOLDS, formatComparison } from "./lib/compare.mjs";
import { runMetrics, taskMetrics, verdict } from "./lib/metrics.mjs";
import { BENCH_ROOT, labelsOf, loadTasks, renderTaskList, validateTasks } from "./lib/tasks.mjs";

const GENIE_ROOT = path.resolve(BENCH_ROOT, "..");
const TRAINING = path.join(BENCH_ROOT, "training");
const RUNS_DIR = process.env.BENCH_RUNS_DIR || path.join(BENCH_ROOT, "runs");
const RESULTS_DIR = process.env.BENCH_RESULTS_DIR || path.join(BENCH_ROOT, "results");
const THRESHOLDS_FILE = path.join(BENCH_ROOT, "thresholds.json");
const GENIE = process.env.GENIE || "genie";
/** The working server: the benchmark never talks to it without `--i-know`. */
const WORKING_PORT = "7420";

const die = (message) => {
  console.error(`bench: ${message}`);
  process.exit(2);
};

const note = (message) => console.log(message);

function parseArgs(argv) {
  const [command, ...rest] = argv;
  const flags = { _: [] };
  const bools = new Set(["start", "dry-run", "json", "i-know", "force", "help"]);
  for (let i = 0; i < rest.length; i += 1) {
    const arg = rest[i];
    if (!arg.startsWith("--")) {
      flags._.push(arg);
      continue;
    }
    const [name, inline] = arg.slice(2).split("=");
    if (inline !== undefined) flags[name] = inline;
    else if (bools.has(name)) flags[name] = true;
    else flags[name] = rest[++i];
  }
  return { command, flags };
}

const usage = `bench/run.mjs — the genie agent benchmark

  list [--json]                         the reference set
  prepare --run <id> [--start] [--dry-run] [--project <slug>] [--repo <dir>]
          [--url <base>]                copy the fixture, register the project, create the tasks
  next --run <id>                       tasks waiting for the owner and the recommended answers
  status --run <id>                     where the round stands now
  collect --run <id> [--note <text>]    snapshot the round into bench/results/<id>.json
  compare <before.json> <after.json> [--json]

Environment: GENIE_URL and GENIE_TOKEN reach the pilot instance, GENIE is the
binary (default "genie"), BENCH_RUNS_DIR / BENCH_RESULTS_DIR move the output.

The working server on port ${WORKING_PORT} is refused unless --i-know is passed.`;

/** The URL of the pilot instance, with the guard against the working server. */
function targetUrl(flags) {
  const url = flags.url || process.env.GENIE_URL || "";
  if (url === "") die("set GENIE_URL or pass --url: the benchmark reaches the pilot instance over its API");
  let parsed;
  try {
    parsed = new URL(url);
  } catch {
    die(`"${url}" is not a URL`);
  }
  const port = parsed.port || (parsed.protocol === "https:" ? "443" : "80");
  if (port === WORKING_PORT && !flags["i-know"]) {
    die(`refusing ${url}: port ${WORKING_PORT} is the working server, not the pilot instance (pass --i-know if you really mean it)`);
  }
  return url.replace(/\/+$/, "");
}

function token() {
  const value = process.env.GENIE_TOKEN || "";
  if (value === "") die("set GENIE_TOKEN to an admin token of the pilot instance (genie user token <login>)");
  return value;
}

/** One `genie` call against the pilot instance; `--json` output is parsed. */
function genieCall(args, project, url, secret) {
  const argv = ["--project", project, ...args, "--json"];
  const run = spawnSync(GENIE, argv, { encoding: "utf8", env: { ...process.env, GENIE_URL: url, GENIE_TOKEN: secret } });
  if (run.error) die(`cannot run ${GENIE}: ${run.error.message}`);
  if (run.status !== 0) die(`${GENIE} ${argv.join(" ")} failed (exit ${run.status}):\n${run.stderr || run.stdout}`);
  const text = (run.stdout || "").trim();
  if (text === "") return null;
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

/** One `git` call in a repository. */
function git(args, cwd, { allowFail = false } = {}) {
  const run = spawnSync("git", args, { cwd, encoding: "utf8" });
  if (run.status !== 0 && !allowFail) die(`git ${args.join(" ")} in ${cwd} failed:\n${run.stderr || run.stdout}`);
  return run.status === 0 ? run.stdout : "";
}

const sha256 = (text) => createHash("sha256").update(text).digest("hex");

/** The hash of a directory's files (paths and contents), `.git` left out. */
function hashTree(dir) {
  const files = [];
  const walk = (d) => {
    for (const entry of readdirSync(d, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      if (entry.name === ".git") continue;
      const p = path.join(d, entry.name);
      if (entry.isDirectory()) walk(p);
      else files.push(p);
    }
  };
  walk(dir);
  const hash = createHash("sha256");
  for (const file of files.sort()) {
    hash.update(path.relative(dir, file));
    hash.update("\0");
    hash.update(readFileSync(file));
    hash.update("\0");
  }
  return hash.digest("hex");
}

/** What a round must record so a diff that mixed two changes is visible. */
function environmentHashes() {
  const files = {};
  for (const pattern of ["agents/*.md", "config/teams/*.json", "config/default.json"]) {
    for (const file of globSync(pattern, { cwd: GENIE_ROOT }).sort()) {
      files[file] = sha256(readFileSync(path.join(GENIE_ROOT, file)));
    }
  }
  let roleModels = {};
  try {
    roleModels = JSON.parse(readFileSync(path.join(GENIE_ROOT, "config", "default.json"), "utf8")).roleModels ?? {};
  } catch {
    roleModels = {};
  }
  const version = spawnSync(GENIE, ["--version"], { encoding: "utf8" });
  return { genie: (version.stdout || "").trim(), files, roleModels };
}

const descriptionOf = (task) => `## Цель\n\n${task.goal}\n\n${task.description}`;

const runDirOf = (runId) => path.join(RUNS_DIR, runId);
const runFile = (runId) => path.join(runDirOf(runId), "run.json");

function readRun(runId) {
  const file = runFile(runId);
  if (!existsSync(file)) die(`no round ${runId}: run \`prepare --run ${runId}\` first (looked in ${file})`);
  return JSON.parse(readFileSync(file, "utf8"));
}

function checkRunId(runId) {
  if (!runId || !/^[a-z0-9][a-z0-9-]*$/.test(runId)) die("--run must be a lowercase id of latin letters, digits and dashes");
}

/** The reference task behind a genie task, by its `bench:R<n>` label. */
function definitionOf(tasks, task) {
  const label = (task.labels ?? []).find((l) => /^bench:/.test(l));
  return label ? tasks.find((t) => `bench:${t.id}` === label) ?? null : null;
}

// ----------------------------------------------------------------- commands

function list(flags) {
  const tasks = loadTasks();
  const problems = validateTasks(tasks);
  if (flags.json) {
    note(JSON.stringify(tasks, null, 2));
    return;
  }
  note(renderTaskList(tasks));
  note("");
  note(`${tasks.length} reference task(s); ${problems.length === 0 ? "the set is usable" : "PROBLEMS:"}`);
  for (const problem of problems) note(`  - ${problem}`);
}

function prepare(flags) {
  const runId = flags.run;
  checkRunId(runId);
  const tasks = loadTasks();
  const problems = validateTasks(tasks);
  if (problems.length > 0) die(`the reference set is not usable:\n  ${problems.join("\n  ")}`);

  const dry = Boolean(flags["dry-run"]);
  const url = dry ? flags.url || process.env.GENIE_URL || "<GENIE_URL>" : targetUrl(flags);
  const secret = dry ? "<GENIE_TOKEN>" : token();
  const slug = flags.project || `bench-${runId}`;
  const dir = runDirOf(runId);
  const repo = path.resolve(flags.repo || path.join(dir, "training"));
  const startedAt = new Date().toISOString();

  const plan = [];
  plan.push(["project", "add", slug, "--repo", repo, "--prefix", "B"]);
  for (const task of tasks) {
    plan.push(["task", "create", task.title, "--description", descriptionOf(task), "--type", "task", "--draft", ...task.ac.flatMap((ac) => ["--ac", ac]), ...labelsOf(task).flatMap((label) => ["--label", label])]);
  }

  if (dry) {
    note(`run     ${runId}`);
    note(`project ${slug}`);
    note(`repo    ${repo}`);
    note(`url     ${url}`);
    note("");
    note("would run:");
    const shown = (args) => args.map((a) => (/\s/.test(a) ? JSON.stringify(a) : a)).join(" ");
    for (const args of plan) note(`  ${GENIE} --project ${slug} ${shown(args)} --json`);
    if (flags.start) note(`  ${GENIE} --project ${slug} task status <id> inbox --json   (for each created task)`);
    note("");
    note("recommended answers for the owner:");
    for (const task of tasks.filter((t) => t.ownerAnswer)) note(`  ${task.id}: ${task.ownerAnswer}`);
    return;
  }

  // A parked round is released later with `prepare --run <id> --start`: it is
  // already prepared, only the tasks are still drafts.
  if (existsSync(runFile(runId)) && flags.start && !flags.force) {
    const parked = JSON.parse(readFileSync(runFile(runId), "utf8"));
    const releaseUrl = targetUrl(flags);
    const releaseSecret = token();
    for (const task of parked.tasks) genieCall(["task", "status", task.id, "inbox"], parked.project, releaseUrl, releaseSecret);
    parked.released = true;
    writeFileSync(runFile(runId), `${JSON.stringify(parked, null, 2)}\n`);
    note(`released ${parked.tasks.length} task(s) of round ${runId} to the orchestrator`);
    note(`next: ${GENIE} bench/run.mjs next --run ${runId}`);
    return;
  }
  if (existsSync(dir) && !flags.force) die(`${dir} already holds a round: pick another --run id, or pass --force`);
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  cpSync(TRAINING, repo, { recursive: true });
  git(["init", "-q"], repo);
  git(["config", "user.name", "genie benchmark"], repo);
  git(["config", "user.email", "bench@genie.local"], repo);
  git(["add", "-A"], repo);
  git(["commit", "-q", "-m", `benchmark baseline ${runId}`], repo);
  const baselineSha = git(["rev-parse", "HEAD"], repo).trim();
  const fixtureHash = hashTree(repo);

  genieCall(plan[0], slug, url, secret);
  const created = [];
  for (let i = 0; i < tasks.length; i += 1) {
    const answer = genieCall(plan[i + 1], slug, url, secret);
    const id = typeof answer?.id === "string" ? answer.id : typeof answer?.task === "string" ? answer.task : null;
    if (!id) die(`creating "${tasks[i].title}" answered no task id: ${JSON.stringify(answer)?.slice(0, 300)}`);
    created.push({ id, ref: tasks[i].id, title: tasks[i].title });
    note(`created ${id} (${tasks[i].id}) ${tasks[i].title}`);
  }

  if (flags.start) {
    for (const task of created) genieCall(["task", "status", task.id, "inbox"], slug, url, secret);
    note(`released ${created.length} task(s) to the orchestrator`);
  }

  const run = {
    runId,
    project: slug,
    repo,
    url,
    baselineSha,
    fixtureHash,
    startedAt,
    released: Boolean(flags.start),
    template: tasks[0]?.template ?? "standard",
    tasks: created,
    recommendedAnswers: Object.fromEntries(tasks.filter((t) => t.ownerAnswer).map((t) => [t.id, t.ownerAnswer])),
    hashes: environmentHashes(),
  };
  writeFileSync(runFile(runId), `${JSON.stringify(run, null, 2)}\n`);
  note("");
  note(`round  ${runId} in ${dir}`);
  note(`project ${slug}, baseline ${baselineSha.slice(0, 8)}, fixture ${fixtureHash.slice(0, 8)}`);
  if (!flags.start) {
    note("the tasks are parked as drafts: answer the questions, then `prepare ... --start` to release them");
    note("(running `prepare` again would re-create the round — use `--start` with the same id only if it is prepared and empty)");
  }
  note("");
  note("recommended answers for the owner:");
  for (const [ref, answer] of Object.entries(run.recommendedAnswers)) note(`  ${ref}: ${answer}`);
  note("");
  note(`next: ${GENIE} bench/run.mjs next --run ${runId}`);
}

function next(flags) {
  const run = readRun(flags.run ?? flags._[0]);
  const url = targetUrl(flags);
  const secret = token();
  const states = run.tasks.map((entry) => ({ entry, task: genieCall(["task", "show", entry.id, "--history"], run.project, url, secret) }));
  const waiting = states.filter((s) => s.task.status === "needs_owner");
  if (waiting.length === 0) {
    note("nobody is waiting for the owner.");
    return;
  }
  for (const { entry, task } of waiting) {
    const asked = (task.history ?? []).filter((h) => h.to === "needs_owner").pop();
    const waited = asked ? Math.round(((Date.now() - Date.parse(asked.at)) / 60000) * 10) / 10 : null;
    note(`${entry.id} (${entry.ref}) ${task.title}`);
    note(`  question: ${task.needsOwner?.question ?? asked?.note ?? "—"}`);
    note(`  waiting:  ${waited === null ? "unknown" : `${waited} min`}`);
    const answer = run.recommendedAnswers?.[entry.ref];
    note(`  answer:   ${answer ?? "no recommended answer: answer in your own words"}`);
    note("");
  }
  note(`${waiting.length} question(s) wait for the owner. The script does not answer them.`);
}

function status(flags) {
  const run = readRun(flags.run ?? flags._[0]);
  const url = targetUrl(flags);
  const secret = token();
  const days = Math.max(1, Math.ceil((Date.now() - Date.parse(run.startedAt)) / 86400000) + 1);
  const stats = genieCall(["server", "stats", "--days", String(days)], run.project, url, secret);
  const project = stats?.projects?.[0] ?? null;
  note(`round   ${run.runId} (project ${run.project}), started ${run.startedAt}, released: ${run.released}`);
  note(`repo    ${run.repo} @ ${String(run.baselineSha).slice(0, 8)}`);
  note("");
  note("task    ref   status              team                 returns  questions  cost");
  for (const entry of run.tasks) {
    const task = genieCall(["task", "show", entry.id, "--history"], run.project, url, secret);
    const returns = (task.history ?? []).filter((h) => h.event === "status" && h.from === "review" && h.to === "changes_requested").length;
    const questions = (task.history ?? []).filter((h) => h.to === "needs_owner").length;
    const spend = (project?.usage?.tasks ?? []).find((item) => item.id === entry.id);
    note(
      `${entry.id.padEnd(7)} ${entry.ref.padEnd(5)} ${String(task.status).padEnd(19)} ${String(task.team ?? "—").padEnd(20)} ${String(returns).padEnd(8)} ${String(questions).padEnd(10)} ${spend?.spend?.cost ?? 0}`,
    );
  }
  note("");
  note(`spend so far: $${project?.usage?.spend?.cost ?? 0}; agent runs ${project?.runs ?? "?"} (failed ${project?.runsFailed ?? "?"}); knowledge proposals ${project?.proposals ?? "?"}`);
}

/** The branch a task's team produced, if it is there. */
function branchOf(run, task) {
  const candidates = [task.worktree?.branch, task.team ? `genie/${task.team}` : null, `genie/${task.id}`].filter(Boolean);
  for (const branch of candidates) {
    const found = spawnSync("git", ["rev-parse", "--verify", "--quiet", branch], { cwd: run.repo, encoding: "utf8" });
    if (found.status === 0) return branch;
  }
  return null;
}

/** Run the reference task's check against the branches the round produced. */
function runCheck(definition, run, task, children) {
  if (!definition) return null;
  const branches = [];
  for (const one of [task, ...children]) {
    const branch = branchOf(run, one);
    if (branch) branches.push({ id: one.id, branch });
  }
  if (branches.length === 0) return null;
  const dirs = [];
  try {
    for (const { branch } of branches) {
      const dir = mkdtempSync(path.join(os.tmpdir(), "genie-bench-"));
      git(["worktree", "add", "--quiet", "--detach", dir, branch], run.repo);
      dirs.push(dir);
    }
    const command = definition.check.replaceAll("<workspaces>", dirs.join(" ")).replaceAll("<workspace>", dirs[0]);
    const out = spawnSync(command, { shell: true, cwd: GENIE_ROOT, encoding: "utf8", env: { ...process.env } });
    return { passed: out.status === 0, output: `${out.stdout || ""}${out.stderr || ""}`.trim().slice(-4000) };
  } finally {
    for (const dir of dirs) {
      git(["worktree", "remove", "--force", dir], run.repo, { allowFail: true });
      rmSync(dir, { recursive: true, force: true });
    }
  }
}

function collect(flags) {
  const run = readRun(flags.run ?? flags._[0]);
  const url = targetUrl(flags);
  const secret = token();
  const definitions = loadTasks();
  const days = Math.max(1, Math.ceil((Date.now() - Date.parse(run.startedAt)) / 86400000) + 1);
  const stats = genieCall(["server", "stats", "--days", String(days)], run.project, url, secret);
  const project = stats?.projects?.[0] ?? null;
  const spendById = new Map((project?.usage?.tasks ?? []).map((item) => [item.id, item]));

  const tasks = [];
  for (const entry of run.tasks) {
    const task = genieCall(["task", "show", entry.id, "--history"], run.project, url, secret);
    const children = (task.children ?? []).map((id) => genieCall(["task", "show", id, "--history"], run.project, url, secret));
    const definition = definitionOf(definitions, task) ?? definitions.find((d) => d.id === entry.ref) ?? null;
    const check = runCheck(definition, run, task, children);
    const actual = taskMetrics({ task, spend: spendById.get(task.id) ?? null, check });
    actual.children = children.length || actual.children;
    // Knowledge proposals are counted for the project: the API does not say
    // which task a proposal came from (see bench/README.md).
    actual.proposals = project?.proposals ?? null;
    const verdictOfTask = verdict(definition?.expect ?? {}, actual);
    tasks.push({ ...actual, verdict: verdictOfTask });
    note(`${entry.id} (${entry.ref}) ${task.status}: check ${check === null ? "not run (no branch)" : check.passed ? "passed" : "FAILED"}, ${verdictOfTask.ok ? "as expected" : `off: ${verdictOfTask.problems.join("; ")}`}`);
  }

  const result = {
    runId: run.runId,
    project: run.project,
    repo: run.repo,
    baselineSha: run.baselineSha,
    fixtureHash: run.fixtureHash,
    startedAt: run.startedAt,
    collectedAt: new Date().toISOString(),
    note: flags.note ?? "",
    hashes: run.hashes,
    run: runMetrics({ tasks, stats }),
    tasks,
  };
  mkdirSync(RESULTS_DIR, { recursive: true });
  const file = path.join(RESULTS_DIR, `${run.runId}.json`);
  writeFileSync(file, `${JSON.stringify(result, null, 2)}\n`);
  note("");
  note(`snapshot ${file}`);
  note(`done ${result.run.done}/${result.run.started} (${Math.round(result.run.doneRate * 100)}%), returns ${result.run.returns}, spend $${result.run.costUsd}`);
}

function compare(flags) {
  const [beforeFile, afterFile] = flags._;
  if (!beforeFile || !afterFile) die("compare needs two result files: compare <before.json> <after.json>");
  const before = JSON.parse(readFileSync(beforeFile, "utf8"));
  const after = JSON.parse(readFileSync(afterFile, "utf8"));
  let thresholds = DEFAULT_THRESHOLDS;
  try {
    thresholds = JSON.parse(readFileSync(THRESHOLDS_FILE, "utf8"));
  } catch {
    thresholds = DEFAULT_THRESHOLDS;
  }
  const result = compareRuns(before, after, thresholds);
  if (flags.json) note(JSON.stringify(result, null, 2));
  else note(formatComparison(result));
  if (!result.ok) process.exit(1);
}

// ---------------------------------------------------------------------- main

const { command, flags } = parseArgs(process.argv.slice(2));

if (flags.help || command === undefined || command === "help") {
  note(usage);
  process.exit(0);
}

switch (command) {
  case "list":
    list(flags);
    break;
  case "prepare":
    prepare(flags);
    break;
  case "next":
    next(flags);
    break;
  case "status":
    status(flags);
    break;
  case "collect":
    collect(flags);
    break;
  case "compare":
    compare(flags);
    break;
  default:
    die(`unknown command "${command}"\n\n${usage}`);
}
