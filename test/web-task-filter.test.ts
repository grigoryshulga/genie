// The task list's search and filters: what the address holds and which tasks pass.

import assert from "node:assert/strict";
import { test } from "node:test";
import { isFiltered, matchesFilter, OPEN, parseFilter, readinessOf, sortTasks, type TaskSummary, writeFilter } from "../web/src/entities/task/model.ts";

const now = Date.parse("2026-10-04T12:00:00Z");
const task = (id: string, patch: Partial<TaskSummary> = {}): TaskSummary =>
  ({
    id,
    title: `Задача ${id}`,
    type: "task",
    status: "in_progress",
    priority: 2,
    labels: [],
    acceptanceDone: 0,
    acceptanceTotal: 0,
    deps: [],
    openDeps: [],
    children: 0,
    childrenClosed: 0,
    comments: 0,
    artifacts: 0,
    created: "2026-10-01T12:00:00Z",
    updated: "2026-10-04T11:00:00Z",
    ...patch,
  }) as TaskSummary;
const filter = (query: string) => parseFilter(new URLSearchParams(query));

test("an empty address is the open tasks, and defaults stay out of it", () => {
  const f = filter("");
  assert.deepEqual(f.statuses, OPEN);
  assert.equal(isFiltered(f), false);
  assert.equal(writeFilter(f, new URLSearchParams("task=G-1")).toString(), "task=G-1", "the open task is kept");
  const g = filter("status=needs_owner&ready=partial&time=7d&by=created&prio=0,1&epic=G-2&who=me&q=цена&sort=updated&group=none");
  assert.equal(isFiltered(g), true);
  assert.deepEqual(parseFilter(writeFilter(g, new URLSearchParams())), g, "a filter survives the round trip");
  assert.deepEqual(filter("status=bogus").statuses, OPEN, "unknown statuses fall back to the open ones");
});

test("tasks pass by status, text, readiness, time, priority, epic and person", () => {
  const t = task("G-7", { labels: ["склад"], acceptanceDone: 1, acceptanceTotal: 3, assignee: "anna", parent: "G-2", priority: 1 });
  assert.equal(matchesFilter(t, filter(""), "anna", now), true);
  assert.equal(matchesFilter(t, filter("status=done"), "anna", now), false);
  assert.equal(matchesFilter(t, filter("q=СКЛАД"), "anna", now), true, "labels are searched, in any case");
  assert.equal(matchesFilter(t, filter("q=g-7"), "anna", now), true, "and the number");
  assert.equal(matchesFilter(t, filter("q=отчёт"), "anna", now), false);
  assert.equal(matchesFilter(t, filter("ready=partial"), "anna", now), true);
  assert.equal(matchesFilter(t, filter("ready=full"), "anna", now), false);
  assert.equal(matchesFilter(t, filter("time=today"), "anna", now), true, "updated this morning");
  assert.equal(matchesFilter(t, filter("time=today&by=created"), "anna", now), false, "created three days ago");
  assert.equal(matchesFilter(t, filter("time=stale7"), "anna", now), false);
  assert.equal(matchesFilter(task("G-1", { updated: "2026-09-01T00:00:00Z" }), filter("time=stale30"), "anna", now), true);
  assert.equal(matchesFilter(t, filter("prio=0,1"), "anna", now), true);
  assert.equal(matchesFilter(t, filter("prio=3"), "anna", now), false);
  assert.equal(matchesFilter(t, filter("epic=G-2"), "anna", now), true);
  assert.equal(matchesFilter(t, filter("epic=none"), "anna", now), false);
  assert.equal(matchesFilter(t, filter("who=me"), "anna", now), true);
  assert.equal(matchesFilter(t, filter("who=me"), undefined, now), false, "nobody is responsible in the local mode");
  assert.equal(matchesFilter(t, filter("who=none"), "anna", now), false);
  assert.equal(matchesFilter(t, filter("who=boris"), "anna", now), false);
});

test("epics stay out of the list unless someone has to act on them", () => {
  assert.equal(matchesFilter(task("G-2", { type: "epic", status: "in_progress" }), filter(""), undefined, now), false);
  assert.equal(matchesFilter(task("G-2", { type: "epic", status: "needs_owner" }), filter(""), undefined, now), true);
});

test("readiness goes by the acceptance criteria", () => {
  assert.equal(readinessOf({ acceptanceDone: 0, acceptanceTotal: 0 }), "none");
  assert.equal(readinessOf({ acceptanceDone: 0, acceptanceTotal: 2 }), "zero");
  assert.equal(readinessOf({ acceptanceDone: 1, acceptanceTotal: 2 }), "partial");
  assert.equal(readinessOf({ acceptanceDone: 2, acceptanceTotal: 2 }), "full");
});

test("orders: priority first by default, then by dates or the number", () => {
  const list = [
    task("G-10", { priority: 2, created: "2026-10-02T00:00:00Z", updated: "2026-10-02T00:00:00Z" }),
    task("G-9", { priority: 1, created: "2026-10-03T00:00:00Z", updated: "2026-10-01T00:00:00Z" }),
    task("G-11", { priority: 2, created: "2026-10-01T00:00:00Z", updated: "2026-10-03T00:00:00Z" }),
  ];
  const ids = (sort: Parameters<typeof sortTasks>[1]) => sortTasks(list, sort).map((t) => t.id);
  assert.deepEqual(ids("priority"), ["G-9", "G-11", "G-10"]);
  assert.deepEqual(ids("updated"), ["G-11", "G-10", "G-9"]);
  assert.deepEqual(ids("created"), ["G-9", "G-10", "G-11"]);
  assert.deepEqual(ids("id"), ["G-9", "G-10", "G-11"]);
});
