// Generates tasks.md from tasks.json, and creates the matching tracker tasks on request.
//
//   node docs/assets/brand/handoff/build-tasks.mjs            write tasks.md
//   node docs/assets/brand/handoff/build-tasks.mjs --genie    print the genie commands (dry run)
//   node docs/assets/brand/handoff/build-tasks.mjs --create   run them against the confirmed project
//
// tasks.json is the source of truth. --create needs the native `genie` CLI connected to the repository's
// tracker (see docs/development/genie-cli.md); it creates the epic first, then every task in dependency order,
// resolving keys (H06) to the tracker IDs it was given.

import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

const DIR = import.meta.dirname;
const data = JSON.parse(readFileSync(path.join(DIR, "tasks.json"), "utf8"));
const byKey = new Map(data.tasks.map((t) => [t.key, t]));

for (const t of data.tasks) for (const d of t.deps) if (!byKey.has(d)) throw new Error(`${t.key} depends on unknown ${d}`);

// Acceptance criteria every task carries; UI tasks add the visual ones.
const common = (t) => [
  ...(t.ui ? ["Both themes verified at 1440 and 390 widths, with screenshots attached to the task", "No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied"] : []),
  "A PR is published and CI passes for its latest head",
];
const criteria = (t) => [...t.ac, ...common(t)];

// Waves: a task's wave is one more than the latest wave among its dependencies; tasks in a wave can run in parallel.
const wave = new Map();
const waveOf = (k) => {
  if (!wave.has(k)) wave.set(k, byKey.get(k).deps.length ? 1 + Math.max(...byKey.get(k).deps.map(waveOf)) : 0);
  return wave.get(k);
};
data.tasks.forEach((t) => waveOf(t.key));
// Critical path: the dependency chain with the most agent hours, the lower bound on elapsed work with unlimited parallelism.
const best = new Map();
const chain = (k) => {
  if (!best.has(k)) {
    const t = byKey.get(k);
    const prev = t.deps.map(chain).sort((a, b) => b.hours - a.hours)[0];
    best.set(k, { hours: t.hours + (prev?.hours ?? 0), keys: [...(prev?.keys ?? []), k] });
  }
  return best.get(k);
};
const critical = data.tasks.map((t) => chain(t.key)).sort((a, b) => b.hours - a.hours)[0];

const total = data.tasks.reduce((n, t) => n + t.hours, 0);
const phaseHours = (p) => data.tasks.filter((t) => t.phase === p).reduce((n, t) => n + t.hours, 0);
const fmt = (n) => (Number.isInteger(n) ? String(n) : n.toFixed(1));

if (process.argv.includes("--genie") || process.argv.includes("--create")) {
  const create = process.argv.includes("--create");
  const ids = new Map();
  const project = process.env.GENIE_PROJECT ?? "genie";
  const q = (s) => `'${String(s).replaceAll("'", `'\\''`)}'`;
  const run = (args, label) => {
    if (!create) {
      console.log(`genie --project ${project} ${args.map((a) => (/^[\w./:=@-]+$/.test(a) ? a : q(a))).join(" ")}\n`);
      return `<${label}>`;
    }
    const r = spawnSync("genie", ["--project", project, ...args, "--json"], { encoding: "utf8" });
    if (r.status !== 0) throw new Error(`${label}: ${r.stderr || r.stdout}`);
    const out = JSON.parse(r.stdout);
    const id = out.id ?? out.task?.id;
    if (!id) throw new Error(`${label}: no task id in ${r.stdout}`);
    console.log(`${label} -> ${id}`);
    return id;
  };
  const withAc = (acs) => acs.flatMap((a) => ["--ac", a]);
  const epicId = run(["task", "create", data.epic.title, "--type", "epic", "--description", data.epic.description, ...withAc(data.epic.ac), "--plan", data.epic.plan], "epic");
  const order = [...data.tasks].sort((a, b) => waveOf(a.key) - waveOf(b.key));
  for (const t of order) {
    const description = `${t.description}\n\nScope: ${t.scope.join("; ")}.\nBrief: docs/assets/brand/handoff/README.md and screens.md. Task key ${t.key}; about ${fmt(t.hours)} agent hours.`;
    const args = ["task", "create", `${t.key}: ${t.title}`, "--description", description, ...withAc(criteria(t)), "--plan", t.plan, "--priority", String(t.priority), "--epic", epicId];
    for (const d of t.deps) args.push("--dep", ids.get(d) ?? `<${d}>`);
    ids.set(t.key, run(args, t.key));
  }
  process.exit(0);
}

const lines = [];
const out = (s = "") => lines.push(s);
out("# Redesign task breakdown");
out();
out("Generated from [tasks.json](tasks.json) by [build-tasks.mjs](build-tasks.mjs); edit the JSON, then run `node docs/assets/brand/handoff/build-tasks.mjs`. Read the [handoff README](README.md) first. Briefs for each screen are in [screens.md](screens.md).");
out();
out(`${data.tasks.length} tasks in one epic, about ${fmt(total)} agent hours of work. With independent lanes running in parallel the longest dependency chain is about ${fmt(critical.hours)} hours (${critical.keys.join(" > ")}). Estimates are rough (plus or minus 50 percent) and measure agent working time, not elapsed days; review, decisions, and CI waits come on top.`);
out();
out("## Creating the tracker tasks");
out();
out("The tracker is the unit of record (see [the CLI guide](../../../development/genie-cli.md)). Preview the commands, then create the epic and tasks against the confirmed project:");
out();
out("```bash");
out("node docs/assets/brand/handoff/build-tasks.mjs --genie     # dry run: prints genie commands");
out("GENIE_PROJECT=genie node docs/assets/brand/handoff/build-tasks.mjs --create");
out("```");
out();
out("Creation uses the caller's rights: people create inbox tasks, agents create drafts. Dependencies are passed with `--dep`, so tasks start only when their predecessors close.");
out();
out("## Plan at a glance");
out();
out("| Key | Phase | Task | Hours | Depends on |");
out("|---|---|---|---|---|");
for (const t of data.tasks) out(`| ${t.key} | ${data.phases[t.phase]} | ${t.title} | ${fmt(t.hours)} | ${t.deps.join(", ") || "none"} |`);
out();
out("| Phase | Hours |");
out("|---|---|");
for (const [p, name] of Object.entries(data.phases)) out(`| ${name} | ${fmt(phaseHours(Number(p)))} |`);
out(`| **Total** | **${fmt(total)}** |`);
out();
out("## Parallel lanes");
out();
out("Tasks in the same wave have no dependency on each other. Work in a wave can proceed in separate worktrees; the screen tasks touch disjoint files once `H01` has split the shared stylesheet.");
out();
const waves = [...new Set(data.tasks.map((t) => waveOf(t.key)))].sort((a, b) => a - b);
for (const w of waves) out(`- **Wave ${w + 1}:** ${data.tasks.filter((t) => waveOf(t.key) === w).map((t) => `${t.key} (${fmt(t.hours)}h)`).join(", ")}`);
out();
out("## Rules for the whole team");
out();
out("- One task, one worktree, one branch from `origin/main` (see [AGENTS.md](../../../../AGENTS.md)); small PRs, English titles and descriptions.");
out("- Do not edit the same file as another task in flight. Shared files (`tokens.css`, `components.css`, `app/styles.css` after the split) change only in the foundation tasks; a screen task that needs a new token or component variant opens a small PR against the foundation first.");
out("- Capture screenshots with the harness (`H00`) before and after every UI change, in both themes and both widths, and attach them to the task.");
out("- The orange rule, the Unbounded size floor, and the no-hard-coded-colours rule are checked in review; see the checklist in [principles.md](../principles.md).");
out("- Behaviour does not change. If a redesign needs a behaviour or contract change, stop, record it on the task, and ask the owner.");
out();
out("## Tasks");
for (const [p, name] of Object.entries(data.phases)) {
  out();
  out(`### Phase ${p}: ${name}`);
  for (const t of data.tasks.filter((x) => String(x.phase) === p)) {
    out();
    out(`#### ${t.key}: ${t.title}`);
    out();
    out(`Hours: ${fmt(t.hours)} · Priority: ${t.priority} · Depends on: ${t.deps.join(", ") || "none"} · Wave ${waveOf(t.key) + 1}`);
    out();
    out(t.description);
    out();
    out(`**Scope:** ${t.scope.join("; ")}`);
    out();
    out("**Acceptance criteria**");
    out();
    for (const a of criteria(t)) out(`- ${a}`);
    out();
    out(`**Plan:** ${t.plan}`);
  }
}
writeFileSync(path.join(DIR, "tasks.md"), `${lines.join("\n")}\n`);
console.log(`wrote tasks.md (${data.tasks.length} tasks, ${fmt(total)} hours, critical path ${fmt(critical.hours)} hours)`);
