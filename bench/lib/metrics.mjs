// Turning the server's answers into the benchmark's numbers. Pure: no server,
// no git, no filesystem — the runner feeds it the payloads, the tests feed it
// canned ones.

/** The status changes in a task's history.
 * @param {any} task
 * @returns {any[]} */
export function transitions(task) {
  return (task.history ?? []).filter((h) => h.event === "status" && h.to);
}

/** How often the work came back from review (`review → changes_requested`).
 * @param {any} task
 * @returns {number} */
export function returns(task) {
  return transitions(task).filter((h) => h.from === "review" && h.to === "changes_requested").length;
}

/** How often an agent asked the owner for a decision (moved to `needs_owner`).
 * @param {any} task
 * @returns {number} */
export function ownerQuestions(task) {
  return transitions(task).filter((h) => h.to === "needs_owner").length;
}

/**
 * How long the owner took to answer the first question, in minutes: from the
 * `needs_owner` change to the first word of a person afterwards. `null` when
 * nobody has answered yet.
 */
/** @param {any} task
 * @returns {number|null} */
export function answerMinutes(task) {
  const asked = (task.history ?? []).find((h) => h.to === "needs_owner");
  if (!asked) return null;
  const at = Date.parse(asked.at);
  if (!Number.isFinite(at)) return null;
  const words = (task.comments ?? []).filter((c) => c.role === "human").map((c) => Date.parse(c.at));
  const moves = (task.history ?? []).filter((h) => h.role === "human").map((h) => Date.parse(h.at));
  const answered = [...words, ...moves].filter((t) => Number.isFinite(t) && t >= at);
  if (answered.length === 0) return null;
  return round1((Math.min(...answered) - at) / 60000);
}

/** Minutes from a task's creation to its last change.
 * @param {any} task
 * @returns {number|null} */
export function wallMinutes(task) {
  const from = Date.parse(task.created);
  const to = Date.parse(task.updated);
  if (!Number.isFinite(from) || !Number.isFinite(to)) return null;
  return round1((to - from) / 60000);
}

/** The tokens of a `spend` block (`input` + `output` + both caches).
 * @param {any} spend
 * @returns {number} */
export function tokensOf(spend) {
  const tokens = spend?.tokens;
  if (!tokens) return 0;
  return (tokens.input ?? 0) + (tokens.output ?? 0) + (tokens.cacheRead ?? 0) + (tokens.cacheWrite ?? 0);
}

/** The reference id of a task, from its `bench:R<n>` label.
 * @param {any} task
 * @returns {string|null} */
export function refOf(task) {
  const label = (task.labels ?? []).find((l) => /^bench:/.test(l));
  return label ? label.slice("bench:".length) : null;
}

/**
 * One task's metrics. `task` is the `genie task show --json` payload, `spend`
 * the matching item of the project's `usage.tasks`, `check` the result of
 * running the task's check against its branch (`null` when there was no branch).
 */
/** @param {{ task: any, spend?: any, check?: any }} input
 * @returns {any} */
export function taskMetrics({ task, spend = null, check = null }) {
  return {
    id: task.id,
    ref: refOf(task),
    title: task.title,
    status: task.status,
    done: task.status === "done",
    closed: task.status === "done" || task.status === "cancelled",
    returns: returns(task),
    ownerQuestions: ownerQuestions(task),
    answerMinutes: answerMinutes(task),
    children: (task.children ?? []).length,
    checkPassed: check === null ? null : Boolean(check.passed),
    checkOutput: check?.output ?? null,
    costUsd: spend?.spend?.cost ?? 0,
    tokens: tokensOf(spend?.spend),
    wallMinutes: wallMinutes(task),
    // The API reports failed runs per project, not per task: the round carries
    // the number, the task cannot name its share (see bench/README.md).
    failedRuns: null,
  };
}

/** The headline numbers of a round.
 * @param {{ tasks: any[], stats?: any }} input
 * @returns {any} */
export function runMetrics({ tasks, stats = null }) {
  const started = tasks.length;
  const done = tasks.filter((t) => t.done).length;
  const closed = tasks.filter((t) => t.closed).length;
  const rounds = tasks.reduce((n, t) => n + t.returns, 0);
  const project = stats?.projects?.[0] ?? null;
  const costUsd = project?.usage?.spend?.cost ?? 0;
  return {
    started,
    done,
    closed,
    open: started - closed,
    doneRate: started === 0 ? 0 : round3(done / started),
    returns: rounds,
    returnsPerTask: started === 0 ? 0 : round3(rounds / started),
    ownerQuestions: tasks.reduce((n, t) => n + t.ownerQuestions, 0),
    costUsd,
    costPerDoneTask: done === 0 ? null : round4(costUsd / done),
    runs: project?.runs ?? null,
    runsFailed: project?.runsFailed ?? null,
    proposals: project?.proposals ?? null,
    proposalsApproved: project?.proposalsApproved ?? null,
    answerHoursMedian: project?.answerHoursMedian ?? null,
  };
}

/**
 * How the actual numbers sit against the expected profile of a task. A
 * deviation is the signal of a round, not a hard failure.
 */
/** @param {any} [expect]
 * @param {any} [actual]
 * @returns {{ ok: boolean, problems: string[] }} */
export function verdict(expect = {}, actual = {}) {
  const problems = [];
  if (typeof expect.done === "boolean" && actual.done !== expect.done) problems.push(`done is ${actual.done}, expected ${expect.done}`);
  if (Array.isArray(expect.returns) && outside(actual.returns, expect.returns)) {
    problems.push(`returns is ${actual.returns}, expected ${expect.returns[0]}..${expect.returns[1]}`);
  }
  if (Array.isArray(expect.ownerQuestions) && outside(actual.ownerQuestions, expect.ownerQuestions)) {
    problems.push(`owner questions is ${actual.ownerQuestions}, expected ${expect.ownerQuestions[0]}..${expect.ownerQuestions[1]}`);
  }
  if (expect.splitInto && (actual.children ?? 0) < expect.splitInto) {
    problems.push(`children is ${actual.children ?? 0}, expected at least ${expect.splitInto}`);
  }
  if (Array.isArray(expect.proposals) && expect.proposals[1] > 0 && (actual.proposals ?? 0) < expect.proposals[0]) {
    problems.push(`knowledge proposals is ${actual.proposals ?? 0}, expected at least ${expect.proposals[0]}`);
  }
  return { ok: problems.length === 0, problems };
}

function outside(value, [min, max]) {
  return typeof value !== "number" || value < min || value > max;
}

function round1(x) {
  return Math.round(x * 10) / 10;
}

function round3(x) {
  return Math.round(x * 1000) / 1000;
}

function round4(x) {
  return Math.round(x * 10000) / 10000;
}
