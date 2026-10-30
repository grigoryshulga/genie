// Which queries a journal event makes stale: targeted, with a broad fallback for what is not known.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { observationKeys } from "../web/src/entities/observation/model.ts";
import { type JournalEvent, keysFor, keysForAll, matchesAny, teamKeys, teamState } from "../web/src/shared/api/invalidation.ts";

// The shapes of `keys` in shared/api/client.ts (not imported: it does not load without a bundler).
const keys = {
  meta: ["meta"],
  tasks: ["tasks"],
  teams: ["teams"],
  task: (id: string) => ["task", id],
  team: (id: string) => ["team", id],
};

const ev = (type: string, subject?: string, payload: unknown = {}): JournalEvent => ({ type, subject, payload });
const stale = (events: JournalEvent[], key: readonly unknown[]) => {
  const prefixes = keysForAll(events);
  return prefixes === null || matchesAny(prefixes, key);
};

// The observation events (docs/platform/observation-plane.md §12): the constants of the block in events.rs.
const EVENTS_RS = readFileSync(new URL("../crates/genie-core/src/events.rs", import.meta.url), "utf8");
const OBSERVATION_BLOCK = EVENTS_RS.slice(
  EVENTS_RS.indexOf("// Observation plane"),
  EVENTS_RS.indexOf("#[derive", EVENTS_RS.indexOf("// Observation plane")),
);
const OBSERVATION_EVENTS = [...OBSERVATION_BLOCK.matchAll(/pub const \w+: &str = "([^"]+)";/g)].map((m) => m[1]);

// Every event type the server appends (events.rs, team.rs, knowledge.rs). Adding one there should add it here.
const KNOWN = [
  "task.created", "task.updated", "task.status_changed", "task.commented", "task.criterion_checked", "task.artifact_added",
  "task.blocked", "task.unblocked", "task.team_assigned", "task.deleted", "mail.sent", "mcp.called", "git.pushed", "git.denied",
  "cr.opened", "cr.merged", "cr.closed", "ci.failed", "ci.passed", "ci.stalled", "team.spawned", "team.started", "team.stopped",
  "doc.changed", "doc.proposal", "doc.proposal_decided", "release.published", "chat.posted",
  ...OBSERVATION_EVENTS,
];

test("every known event type has a targeted mapping, an unknown one is broad", () => {
  for (const type of KNOWN) assert.ok(keysFor(ev(type, "T-1")), `${type} is mapped`);
  assert.equal(keysFor(ev("something.new", "T-1")), null);
  assert.equal(keysForAll([ev("task.commented", "T-1"), ev("something.new")]), null, "one unknown event makes the batch broad");
  assert.equal(keysForAll([ev("")]), null, "an unreadable event is broad");
});

test("the mapping covers the keys the entities use", () => {
  const events = [ev("task.status_changed", "T-1")];
  for (const key of [keys.tasks, keys.teams, keys.meta, keys.task("T-1"), keys.team("t")]) assert.ok(stale(events, key), String(key));
});

test("a comment touches that task and the lists, not the others, teams or docs", () => {
  const events = [ev("task.commented", "T-1")];
  assert.ok(stale(events, keys.task("T-1")));
  assert.ok(stale(events, [...keys.task("T-1"), "docs-impact"]));
  assert.ok(stale(events, keys.tasks));
  assert.ok(!stale(events, keys.task("T-2")));
  assert.ok(!stale(events, keys.teams));
  assert.ok(!stale(events, ["docs", "tree"]));
  assert.ok(!stale(events, ["runs", 0]));
  assert.ok(!stale(events, ["notifications"]));
});

test("a status change reaches every task page, the counters and the teams", () => {
  const events = [ev("task.status_changed", "T-1")];
  for (const key of [keys.task("T-2"), keys.tasks, keys.meta, keys.teams, keys.team("t")]) assert.ok(stale(events, key), String(key));
  assert.ok(!stale(events, ["docs", "page", "a.md"]));
});

test("mail and team events touch teams and agents, not docs", () => {
  for (const type of ["mail.sent", "team.started"]) {
    const events = [ev(type, "T-1", { team: "t" })];
    assert.ok(stale(events, keys.teams));
    assert.ok(stale(events, keys.team("t")));
    assert.ok(stale(events, ["peek", "t", "dev"]));
    assert.ok(!stale(events, ["docs", "tree"]));
  }
  assert.ok(!stale([ev("mail.sent", "T-1")], keys.tasks), "mail is not on a task card");
  assert.ok(stale([ev("team.started", "T-1")], keys.tasks), "a card shows its team");
});

test("gateway calls refetch the call list and spend", () => {
  const events = [ev("mcp.called", "T-1")];
  assert.ok(stale(events, ["mcp-calls", "shop"]));
  assert.ok(stale(events, ["usage", "task", "T-1"]));
  assert.ok(!stale(events, keys.tasks));
});

test("delivery events refetch the task's repositories and page", () => {
  for (const type of ["cr.merged", "ci.failed"]) {
    const events = [ev(type, "T-1")];
    assert.ok(stale(events, ["task-repos", "T-1"]));
    assert.ok(stale(events, keys.task("T-1")));
    assert.ok(!stale(events, keys.task("T-2")));
    assert.ok(!stale(events, keys.teams));
  }
  assert.ok(stale([ev("git.pushed", "team-1")], ["task-repos", "T-1"]), "a push names the team, not the task");
});

test("docs events refetch pages, the Changes feed and history", () => {
  for (const type of ["doc.changed", "doc.proposal", "doc.proposal_decided"]) {
    const events = [ev(type)];
    assert.ok(stale(events, ["docs", "page", "a.md"]));
    assert.ok(stale(events, ["docs", "changesets", ""]));
    assert.ok(stale(events, ["docs", "history", "a.md"]));
    assert.ok(!stale(events, keys.teams));
  }
});

test("a batch is the union of its events, without duplicates", () => {
  const prefixes = keysForAll([ev("task.commented", "T-1"), ev("task.commented", "T-1"), ev("task.commented", "T-2")]);
  assert.ok(prefixes);
  assert.equal(new Set(prefixes.map((p) => p.join("/"))).size, prefixes.length);
  assert.ok(matchesAny(prefixes, keys.task("T-1")) && matchesAny(prefixes, keys.task("T-2")));
  assert.deepEqual(keysForAll([]), []);
});

test("a member change refetches the team's own queries, not the task cards", () => {
  // What the team api passes for a mutation that writes a team log line, no journal event.
  assert.deepEqual(teamKeys(), [["teams"], ["team"], ["peek"]]);
  assert.deepEqual(keysFor(ev("mail.sent")), teamKeys(), "mail uses the same keys");
  assert.ok(matchesAny(teamKeys(), keys.team("t")) && matchesAny(teamKeys(), ["peek", "t", "dev"]));
  assert.ok(!matchesAny(teamKeys(), keys.tasks), "a member change is not on a task card");
  // A team appearing or going away does reach the cards.
  assert.deepEqual(keysFor(ev("team.spawned", "T-1")), teamState());
  assert.deepEqual(keysFor(ev("team.stopped", "T-1")), teamState());
  assert.ok(matchesAny(teamState(), keys.tasks) && matchesAny(teamState(), keys.task("T-1")));
});

test("a prefix matches only whole key parts", () => {
  assert.ok(matchesAny([["task"]], ["task", "T-1"]));
  assert.ok(!matchesAny([["task"]], ["tasks"]));
  assert.ok(!matchesAny([["task", "T-1"]], ["task"]));
});

test("the §12 observation events are read from events.rs, all sixteen of them", () => {
  assert.equal(OBSERVATION_EVENTS.length, 16);
  assert.ok(OBSERVATION_EVENTS.includes("outcome.ready_for_evaluation"));
  assert.ok(OBSERVATION_EVENTS.includes("followup_task.created"));
});

const CASE_KEYS = [
  observationKeys.cases({ status: "observing" }),
  observationKeys.case(7),
  observationKeys.evidence(7, { limit: 5 }),
  observationKeys.incidents("open"),
  observationKeys.incidents(),
  observationKeys.task("G-1"),
];

test("every observation event refetches every observation query", () => {
  for (const type of OBSERVATION_EVENTS) {
    const events = [ev(type, "observation:7", { case: 7, key: "O-7" })];
    for (const key of CASE_KEYS) assert.ok(stale(events, key), `${type} refetches ${JSON.stringify(key)}`);
  }
});

test("an observation event leaves tasks and sources alone, except a follow-up task", () => {
  for (const type of OBSERVATION_EVENTS) {
    const events = [ev(type, "observation:7")];
    assert.ok(!stale(events, observationKeys.sources), `${type} does not refetch sources`);
    if (type === "followup_task.created") continue;
    assert.ok(!stale(events, keys.tasks), `${type} does not refetch the task list`);
    assert.ok(!stale(events, keys.task("G-1")), `${type} does not refetch a task page`);
  }
});

test("a follow-up task refetches the observation queries, the task lists and the task pages", () => {
  const events = [ev("followup_task.created", "observation:7", { case: 7, key: "O-7", task: "G-9", incident: null })];
  for (const key of [...CASE_KEYS, keys.tasks, keys.task("G-9"), ["task"]]) assert.ok(stale(events, key), JSON.stringify(key));
  assert.ok(!stale(events, keys.teams), "teams are not part of it");
});

test("a non-observation event does not refetch observation queries", () => {
  for (const type of ["task.created", "task.status_changed", "cr.merged", "mail.sent", "doc.changed"]) {
    for (const key of CASE_KEYS) assert.ok(!stale([ev(type, "G-1")], key), `${type} leaves ${JSON.stringify(key)}`);
  }
});
