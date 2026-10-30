// Writes the browser tests' data into a genie data directory through the CLI. The CLI acts as the
// machine's operator, which is what lets a seed reach a status such as needs_owner that the web refuses
// to a person. Shared by the browser-test harness (run.ts) and the development stand (stand.ts).

import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
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

/** A section policy that makes published human changesets wait for their owners. */
export function seedChangesetPolicy(dataDir: string, space: string, owners: string[]) {
  const path = join(dataDir, "vault/.genie/vault.json");
  let config: { spaces?: Record<string, { policy?: unknown; owners?: string[] }> } = {};
  try {
    config = JSON.parse(readFileSync(path, "utf8"));
  } catch {
    // The vault writes its default config on the first change; start from the same default.
    config.spaces = {};
  }
  config.spaces ??= {};
  const existing = config.spaces[space] ?? {};
  config.spaces[space] = { ...existing, owners, policy: { humans: "review", agents: "review" } };
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(config, null, 2)}\n`);
}

/**
 * Seeds the documentation changesets the screens need: one unreviewed publication,
 * one open draft and one merge that failed. It runs against the server's HTTP API
 * after the first start, because changesets exist only once the vault routes do.
 */
export async function seedChangesets(apiPort: number, project: string): Promise<string> {
  const base = `http://127.0.0.1:${apiPort}`;

  async function login(login: string, password: string): Promise<string> {
    const res = await fetch(`${base}/api/auth/login`, {
      method: "POST",
      headers: { "content-type": "application/json", "x-genie": "1" },
      body: JSON.stringify({ login, password }),
    });
    if (!res.ok) throw new Error(`login ${login}: ${res.status}`);
    const cookie = res.headers.get("set-cookie")?.split(";")[0];
    if (!cookie) throw new Error(`login ${login}: no session cookie`);
    return cookie;
  }

  async function call(cookie: string, method: string, path: string, body?: unknown): Promise<Record<string, unknown>> {
    const res = await fetch(`${base}${path}`, {
      method,
      headers: { "content-type": "application/json", "x-genie": "1", "x-genie-project": project, cookie },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const data = (await res.json().catch(() => ({}))) as Record<string, unknown> & { error?: string };
    if (!res.ok) throw new Error(`${method} ${path}: ${res.status} ${data.error ?? ""}`.trim());
    return data;
  }

  const owner = await login(OWNER.login, OWNER.password);
  const ivan = await login("ivan", "stand-password");

  // A draft the owner publishes at once: the review policy leaves it unreviewed in the feed.
  const first = (await call(owner, "POST", "/api/docs/edit", {
    path: "demo/guides/run.md",
    ops: [{ op: "replace-section", heading: "Шаги", body: "1. Собрать бинарник.\n2. Запустить его с каталогом данных.\n3. Проверить журнал сервера." }],
    note: "Добавлена проверка после запуска",
  })) as { changeset: { id: number } };

  // A second author edits the same section from the same base: publication must conflict.
  const second = (await call(ivan, "POST", "/api/docs/edit", {
    path: "demo/guides/run.md",
    ops: [{ op: "replace-section", heading: "Шаги", body: "1. Собрать бинарник.\n2. Запустить его с каталогом данных.\n3. Убедиться, что порт свободен." }],
    note: "Порт часто занят старым процессом",
  })) as { changeset: { id: number } };

  await call(owner, "POST", `/api/docs/changesets/${first.changeset.id}/publish`, {});
  const published = (await call(ivan, "POST", `/api/docs/changesets/${second.changeset.id}/publish`, {})) as {
    status: string;
    merge?: { id: number };
  };

  let mergeNote = "no merge";
  if (published.status === "merging" && published.merge) {
    try {
      await call(owner, "POST", `/api/docs/merges/${published.merge.id}/resolve`, { fail: true });
      mergeNote = `merge #${published.merge.id} failed`;
    } catch {
      mergeNote = `merge #${published.merge.id} pending`;
    }
  }

  // An open draft, published by its author later from the page's bar.
  await call(owner, "POST", "/api/docs/edit", {
    path: "demo/guides/deploy.md",
    ops: [{ op: "append", heading: "Команды", text: "\nПеред выкаткой проверьте журнал деплоя." }],
    note: "Черновик к статье о деплое",
  });

  return `changesets seeded: #${first.changeset.id} unreviewed, #${second.changeset.id} ${mergeNote}`;
}
