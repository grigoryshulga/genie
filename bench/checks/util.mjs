// Small helpers shared by the benchmark's per-task checks. No dependencies:
// every check is `node bench/checks/rN.mjs <workspace…>`, run by `run.mjs collect`
// against the branch a team produced. Exit 0 = the expected behaviour is there.

import { spawnSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

/** The check failed: say why and exit non-zero. */
export function fail(reason) {
  console.error(`FAIL ${reason}`);
  process.exit(1);
}

/** The check passed. */
export function pass(what) {
  console.log(`PASS ${what}`);
}

/** The workspace paths the runner passed (the task's branch, and its children's). */
export function workspaces() {
  const args = process.argv.slice(2).filter((a) => a !== "");
  if (args.length === 0) fail("no workspace given (the runner passes the task's branch)");
  return args;
}

/** The text of a file below the workspace, or "" when it is not there. */
export function read(ws, rel) {
  try {
    return readFileSync(path.join(ws, rel), "utf8");
  } catch {
    return "";
  }
}

/** Load a module of the workspace; a module that will not load fails the check. */
export async function load(ws, rel) {
  try {
    return await import(pathToFileURL(path.join(ws, rel)).href);
  } catch (e) {
    return fail(`${rel} in ${ws} cannot be loaded: ${e.message}`);
  }
}

/** Load a module of the workspace, or `null` when it is not there. */
export async function tryLoad(ws, rel) {
  try {
    return await import(pathToFileURL(path.join(ws, rel)).href);
  } catch {
    return null;
  }
}

/** Every `.js` file below `dir`, recursively. */
export function jsFiles(dir) {
  const found = [];
  const walk = (d) => {
    let entries = [];
    try {
      entries = readdirSync(d, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const p = path.join(d, entry.name);
      if (entry.isDirectory()) walk(p);
      else if (entry.name.endsWith(".js")) found.push(p);
    }
  };
  walk(dir);
  return found;
}

/** `node --test` in the workspace: a red suite fails the check. */
export function testsPass(ws) {
  const run = spawnSync(process.execPath, ["--test"], { cwd: ws, encoding: "utf8" });
  if (run.status !== 0) fail(`node --test in ${ws} is red:\n${(run.stdout || "") + (run.stderr || "")}`);
}
