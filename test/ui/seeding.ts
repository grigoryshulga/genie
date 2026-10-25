// Writes the browser tests' data into a genie data directory through the CLI. The CLI acts as the
// machine's operator, which is what lets a seed reach a status such as needs_owner that the web refuses
// to a person. Shared by the browser-test harness (run.ts) and the development stand (stand.ts).

import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { OWNER, type SeedTask } from "./seed.ts";

export interface Doc {
  path: string;
  text: string;
  /** Written to the vault directly: the CLI refuses invalid frontmatter. */
  raw?: boolean;
}

// The command line acts as the machine's operator, named after $USER: the owner's login keeps the
// task history the same on every machine.
export function genie(bin: string, args: string[], input: string | undefined, dataDir: string): string {
  const env = { ...process.env, USER: OWNER.login };
  return execFileSync(bin, ["--data", dataDir, ...args], { input, env, encoding: "utf8", stdio: ["pipe", "pipe", "pipe"] });
}

/**
 * Creates one task in `dataDir` and walks it through its statuses, the way the operator would. Returns
 * the id the server assigned; `ids` collects them for `parent`.
 */
export function seedTask(bin: string, dataDir: string, project: string, t: SeedTask, ids: string[]): string {
  const run = (args: string[], input?: string) => genie(bin, ["--project", project, ...args], input, dataDir);
  const args = ["task", "create", t.title, "--json"];
  if (t.description) args.push("--description", t.description);
  for (const ac of t.acceptance ?? []) args.push("--ac", ac);
  if (t.priority !== undefined) args.push("--priority", String(t.priority));
  if (t.type) args.push("--type", t.type);
  for (const label of t.labels ?? []) args.push("--label", label);
  if (t.parent !== undefined) args.push("--parent", ids[t.parent]);
  const id: string = JSON.parse(run(args)).id;
  ids.push(id);
  for (const status of t.statuses ?? []) {
    const owner = status === "needs_owner" ? ["--action", t.action ?? "ask-free-form", ...(t.options ?? []).flatMap((o) => ["--option", o])] : [];
    const note = status === "needs_owner" && t.note ? ["--note", t.note] : [];
    run(["task", "status", status, "--task", id, ...note, ...owner]);
  }
  for (const n of t.checked ?? []) run(["task", "check", String(n + 1), "--task", id]);
  for (const c of t.comments ?? []) run(["task", "comment", c.text, "--task", id, "--kind", c.kind ?? "note"]);
  for (const a of t.artifacts ?? []) run(["task", "artifact", "--task", id, "--name", a.name, "--kind", a.kind ?? "other", "--text", a.text]);
  if (t.blocked) run(["task", "block", t.blocked, "--task", id]);
  return id;
}

export function seedDocs(bin: string, dataDir: string, project: string, docs: Doc[]) {
  for (const doc of docs) {
    if (!doc.raw) genie(bin, ["--project", project, "docs", "write", doc.path, "--text", doc.text], undefined, dataDir);
    else {
      mkdirSync(dirname(join(dataDir, "vault", doc.path)), { recursive: true });
      writeFileSync(join(dataDir, "vault", doc.path), doc.text);
    }
  }
}
