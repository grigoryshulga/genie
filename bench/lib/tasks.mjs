// The reference task set: loading it, checking its shape and rendering it for a
// person. Pure — no server, no git, no spend.

import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));

/** The bench directory: the parent of `lib/`. */
export const BENCH_ROOT = path.resolve(here, "..");

/** Where the reference set lives. */
export const TASKS_FILE = path.join(BENCH_ROOT, "reference", "tasks.json");

/** The fields every reference task has to carry. */
const REQUIRED = ["id", "title", "goal", "description", "check", "expect", "budgetUsd", "timeoutMin", "labels", "template"];

/** The reference tasks, as they are on disk.
 * @param {string} [file]
 * @returns {any[]} */
export function loadTasks(file = TASKS_FILE) {
  const data = JSON.parse(readFileSync(file, "utf8"));
  if (!Array.isArray(data.tasks)) throw new Error(`${file}: no "tasks" array`);
  return data.tasks;
}

/**
 * Everything wrong with the set; an empty array means it is usable. The runner
 * refuses to prepare a round with problems, so a broken task never costs money.
 */
/** @param {any[]} tasks
 * @returns {string[]} */
export function validateTasks(tasks) {
  const problems = [];
  const seen = new Set();
  for (const task of tasks) {
    for (const field of REQUIRED) {
      if (task[field] === undefined || task[field] === null || task[field] === "") problems.push(`${task.id ?? "?"}: ${field} is missing`);
    }
    if (seen.has(task.id)) problems.push(`${task.id}: the id is used twice`);
    seen.add(task.id);
    if (!Array.isArray(task.ac) || task.ac.length === 0) problems.push(`${task.id}: no acceptance criteria`);
    if (typeof task.check !== "string" || !task.check.includes("<workspace")) problems.push(`${task.id}: check does not take a <workspace>`);
    if (!Array.isArray(task.labels) || !task.labels.includes("bench")) problems.push(`${task.id}: the labels must include "bench"`);
    const expect = task.expect ?? {};
    if (typeof expect.done !== "boolean") problems.push(`${task.id}: expect.done must be true or false`);
    for (const field of ["returns", "ownerQuestions"]) {
      const pair = expect[field];
      if (!Array.isArray(pair) || pair.length !== 2 || pair[0] > pair[1]) problems.push(`${task.id}: expect.${field} must be a [min, max] pair`);
    }
    if (!(task.budgetUsd > 0)) problems.push(`${task.id}: budgetUsd must be a positive number`);
    if (!(task.timeoutMin > 0)) problems.push(`${task.id}: timeoutMin must be a positive number`);
  }
  if (tasks.length < 5 || tasks.length > 10) problems.push(`the set has ${tasks.length} task(s): 5 to 10 are expected`);
  return problems;
}

/** The labels a task is created with (the set may override them).
 * @param {any} task
 * @returns {string[]} */
export function labelsOf(task) {
  return Array.isArray(task.labels) && task.labels.length > 0 ? task.labels : ["bench", `bench:${task.id}`];
}

function range(pair) {
  if (!Array.isArray(pair)) return "?";
  return pair[0] === pair[1] ? String(pair[0]) : `${pair[0]}..${pair[1]}`;
}

/** The set as a person reads it (`run.mjs list`).
 * @param {any[]} tasks
 * @returns {string} */
export function renderTaskList(tasks) {
  const lines = [];
  for (const task of tasks) {
    const expect = task.expect ?? {};
    const extras = [];
    if (expect.splitInto) extras.push(`split into >=${expect.splitInto}`);
    if (Array.isArray(expect.proposals) && expect.proposals[1] > 0) extras.push(`knowledge proposals ${range(expect.proposals)}`);
    lines.push(`${task.id}  ${task.title}`);
    lines.push(`      goal:    ${task.goal}`);
    lines.push(`      check:   ${task.check}`);
    lines.push(`      expect:  done=${expect.done} returns=${range(expect.returns)} questions=${range(expect.ownerQuestions)}${extras.length ? " " + extras.join(" ") : ""}`);
    lines.push(`      limits:  $${task.budgetUsd}, ${task.timeoutMin} min${task.ownerAnswer ? ", a recommended answer for the owner" : ""}`);
  }
  return lines.join("\n");
}
