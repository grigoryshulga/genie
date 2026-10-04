// Comparing two collected rounds. Pure: the thresholds come from
// `bench/thresholds.json`, the numbers from two result files.

export const DEFAULT_THRESHOLDS = {
  doneRate: { maxDrop: 0.1 },
  returnsPerTask: { maxIncrease: 1 },
  costPerDoneTask: { maxIncreasePct: 25 },
};

/**
 * The headline numbers of two rounds against the thresholds. A regression is a
 * change that is worse than the rule allows — the retrospective gates a role or
 * prompt edit on this verdict.
 */
/** @param {any} before
 * @param {any} after
 * @param {any} [thresholds]
 * @returns {any} */
export function compareRuns(before, after, thresholds = DEFAULT_THRESHOLDS) {
  const b = before?.run ?? {};
  const a = after?.run ?? {};
  const checks = [];

  const drop = thresholds?.doneRate?.maxDrop ?? DEFAULT_THRESHOLDS.doneRate.maxDrop;
  checks.push(
    judge("doneRate", b.doneRate, a.doneRate, (x, y) => y >= x - drop, `not more than ${drop} below the before`),
  );

  const bump = thresholds?.returnsPerTask?.maxIncrease ?? DEFAULT_THRESHOLDS.returnsPerTask.maxIncrease;
  checks.push(
    judge("returnsPerTask", b.returnsPerTask, a.returnsPerTask, (x, y) => y <= x + bump, `not more than ${bump} above the before`),
  );

  const pct = thresholds?.costPerDoneTask?.maxIncreasePct ?? DEFAULT_THRESHOLDS.costPerDoneTask.maxIncreasePct;
  const cost = { name: "costPerDoneTask", before: b.costPerDoneTask ?? null, after: a.costPerDoneTask ?? null, rule: `not more than ${pct}% above the before`, ok: true, note: "" };
  if (b.costPerDoneTask === null || b.costPerDoneTask === undefined) {
    cost.note = "the before round has no cost per done task: not compared";
  } else if (a.costPerDoneTask === null || a.costPerDoneTask === undefined) {
    cost.ok = false;
    cost.note = "the after round has no cost per done task";
  } else {
    cost.ok = a.costPerDoneTask <= b.costPerDoneTask * (1 + pct / 100);
  }
  checks.push(cost);

  const regression = checks.filter((c) => !c.ok).map((c) => c.name);
  return {
    before: before?.runId ?? "before",
    after: after?.runId ?? "after",
    checks,
    tasks: taskDiff(before?.tasks ?? [], after?.tasks ?? []),
    regression,
    ok: regression.length === 0,
  };
}

/** One headline number, judged by `rule`. */
function judge(name, before, after, rule, text) {
  const known = typeof before === "number" && typeof after === "number";
  return { name, before: before ?? null, after: after ?? null, rule: text, ok: known ? rule(before, after) : false, note: known ? "" : "a round is missing this number" };
}

/** The tasks of both rounds side by side, by reference id.
 * @param {any[]} beforeTasks
 * @param {any[]} afterTasks
 * @returns {any[]} */
export function taskDiff(beforeTasks, afterTasks) {
  const byRef = (tasks) => {
    const map = new Map();
    for (const task of tasks) map.set(task.ref ?? task.id, task);
    return map;
  };
  const b = byRef(beforeTasks);
  const a = byRef(afterTasks);
  const refs = [...new Set([...b.keys(), ...a.keys()])].filter(Boolean).sort();
  return refs.map((ref) => {
    const one = b.get(ref) ?? {};
    const two = a.get(ref) ?? {};
    return {
      ref,
      statusBefore: one.status ?? null,
      statusAfter: two.status ?? null,
      returnsBefore: one.returns ?? null,
      returnsAfter: two.returns ?? null,
      costBefore: one.costUsd ?? null,
      costAfter: two.costUsd ?? null,
      checkBefore: one.checkPassed ?? null,
      checkAfter: two.checkPassed ?? null,
    };
  });
}

const show = (x) => (x === null || x === undefined ? "—" : String(x));

/** The comparison as a person reads it.
 * @param {any} result
 * @returns {string} */
export function formatComparison(result) {
  const lines = [`Benchmark: ${result.before} → ${result.after}`];
  lines.push("");
  lines.push("headline                      before      after       rule");
  for (const c of result.checks) {
    lines.push(`${c.name.padEnd(28)}  ${show(c.before).padEnd(10)}  ${show(c.after).padEnd(10)}  ${c.ok ? "ok  " : "FAIL"} ${c.rule}${c.note ? ` (${c.note})` : ""}`);
  }
  lines.push("");
  lines.push("task  status before → after   returns   cost");
  for (const t of result.tasks) {
    const cost = `${show(t.costBefore)} → ${show(t.costAfter)}`;
    lines.push(`${t.ref.padEnd(5)} ${show(t.statusBefore).padEnd(16)}→ ${show(t.statusAfter).padEnd(16)} ${show(t.returnsBefore)} → ${show(t.returnsAfter)}   ${cost}`);
  }
  lines.push("");
  lines.push(result.ok ? "PASS: no regression against the thresholds" : `REGRESSION: ${result.regression.join(", ")}`);
  return lines.join("\n");
}
