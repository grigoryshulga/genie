// Runs the browser tests against a real genie server.
//
// The server runs on the host from a built binary, on a disposable data directory seeded through
// the CLI. Two servers start from the same seed written twice into two directories: the screenshots
// get data no flow has changed, and a failed screenshot does not hold back the flows. Playwright runs inside the
// official image pinned to the installed @playwright/test version, so screenshots render with the
// same fonts and browser locally and in CI. The container shares the host network and reaches the
// servers on 127.0.0.1.
//
// Usage: node test/ui/run.ts [playwright test arguments], e.g. --update-snapshots or --project flows.
// `--capture` (npm run ui:capture) instead seeds all eleven statuses and writes a PNG of every route
// in each theme at desktop and phone sizes to test-results/captures; it compares nothing.
// GENIE_BIN selects the server binary (default: target/debug/genie); web/dist must be built.

import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { createWriteStream, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { CAPTURE_DOCS, CAPTURE_TASKS, FLOW_DOC, FLOW_TASKS, OWNER, PROJECT, SEED_TASKS, type SeedTask } from "./seed.ts";

const root = resolve(import.meta.dirname, "../..");
const bin = resolve(root, process.env.GENIE_BIN ?? "target/debug/genie");
const web = join(root, "web/dist");
const results = join(root, "test-results");
const capture = process.argv.includes("--capture");
const passthrough = process.argv.slice(2).filter((a) => a !== "--capture");

function fail(message: string): never {
  console.error(`ui tests: ${message}`);
  process.exit(1);
}

if (!existsSync(bin)) fail(`no genie binary at ${bin}: run \`cargo build -p genie\` or set GENIE_BIN`);
if (!existsSync(join(web, "index.html"))) fail("no web/dist: run `npm run build:web`");
const version = JSON.parse(readFileSync(join(root, "node_modules/@playwright/test/package.json"), "utf8")).version;
const image = `mcr.microsoft.com/playwright:v${version}-noble`;

const scratch = mkdtempSync(join(tmpdir(), "genie-ui-"));
mkdirSync(results, { recursive: true });
const servers: ChildProcess[] = [];

function cleanup() {
  for (const server of servers) server.kill();
  rmSync(scratch, { recursive: true, force: true });
}
process.on("exit", cleanup);
for (const signal of ["SIGINT", "SIGTERM"] as const) process.on(signal, () => process.exit(130));

// The command line acts as the machine's operator, named after $USER: the owner's login keeps the
// task history the same on every machine.
function genie(args: string[], input: string | undefined, dataDir: string): string {
  const env = { ...process.env, USER: OWNER.login };
  return execFileSync(bin, ["--data", dataDir, ...args], { input, env, encoding: "utf8", stdio: ["pipe", "pipe", "pipe"] });
}

/**
 * Creates one task in `dataDir` and walks it through its statuses, the way the operator would. The
 * CLI is the machine's operator, which is what lets a seed reach a status such as needs_owner that the
 * web refuses to a person. Returns the id the server assigned; `ids` collects them for `parent`.
 */
function seedTask(dataDir: string, t: SeedTask, ids: string[]): string {
  const args = ["--project", PROJECT.id, "task", "create", t.title, "--json"];
  if (t.description) args.push("--description", t.description);
  for (const ac of t.acceptance ?? []) args.push("--ac", ac);
  if (t.priority !== undefined) args.push("--priority", String(t.priority));
  if (t.type) args.push("--type", t.type);
  for (const label of t.labels ?? []) args.push("--label", label);
  if (t.parent !== undefined) args.push("--parent", ids[t.parent]);
  const id: string = JSON.parse(genie(args, undefined, dataDir)).id;
  ids.push(id);
  for (const status of t.statuses ?? []) {
    const owner = status === "needs_owner" ? ["--action", t.action ?? "ask-free-form", ...(t.options ?? []).flatMap((o) => ["--option", o])] : [];
    const note = status === "needs_owner" && t.note ? ["--note", t.note] : [];
    genie(["--project", PROJECT.id, "task", "status", status, "--task", id, ...note, ...owner], undefined, dataDir);
  }
  return id;
}

// The seed the two servers share, plus the tasks only one of them may have. Each server gets its own
// freshly written copy: a data directory records its project's paths absolutely, so a copied directory
// would keep writing into the one it was copied from, and a task made for the flows server would turn
// up in the screenshots and in their committed baselines.
function seed(dataDir: string, extra: SeedTask[] = []) {
  genie(["user", "add", OWNER.login, "--name", OWNER.name, "--admin", "--password-stdin"], `${OWNER.password}\n`, dataDir);
  genie(["project", "add", PROJECT.id, "--name", PROJECT.name, "--prefix", PROJECT.prefix], undefined, dataDir);
  genie(["member", PROJECT.id, OWNER.login, "owner"], undefined, dataDir);
  genie(["--project", PROJECT.id, "docs", "write", FLOW_DOC.path, "--text", FLOW_DOC.text], undefined, dataDir);
  if (capture) {
    for (const doc of CAPTURE_DOCS) {
      if (!doc.raw) genie(["--project", PROJECT.id, "docs", "write", doc.path, "--text", doc.text], undefined, dataDir);
      else {
        mkdirSync(dirname(join(dataDir, "vault", doc.path)), { recursive: true });
        writeFileSync(join(dataDir, "vault", doc.path), doc.text);
      }
    }
  }
  const ids: string[] = [];
  for (const t of [...SEED_TASKS, ...extra]) seedTask(dataDir, t, ids);
}

// A port the kernel just handed out is free unless something grabs it in between. Two probes listen at
// the same time before either server starts, so the kernel cannot hand both servers the same port; a
// shared port would silently point both test projects at whichever server bound it first.
function freePort(): Promise<number> {
  return new Promise((done, reject) => {
    const probe = createServer();
    probe.once("error", reject);
    probe.listen(0, "127.0.0.1", () => {
      const address = probe.address();
      probe.close(() => (typeof address === "object" && address ? done(address.port) : reject(new Error("no port"))));
    });
  });
}

// A server on its own freshly seeded data, logging to test-results/ui-server-<name>.log. `extra` is what
// only this server may see.
async function start(name: string, port: number, extra: SeedTask[]): Promise<string> {
  const dir = join(scratch, name);
  mkdirSync(dir, { recursive: true });
  seed(dir, extra);
  const url = `http://127.0.0.1:${port}`;
  const log = createWriteStream(join(results, `ui-server-${name}.log`));
  const server = spawn(bin, ["--data", dir, "serve", "--port", String(port), "--web", web, "--no-agents"], { stdio: ["ignore", "pipe", "pipe"] });
  servers.push(server);
  server.stdout?.pipe(log);
  server.stderr?.pipe(log);
  await waitHealthy(name, server, url);
  return url;
}

async function waitHealthy(name: string, server: ChildProcess, url: string) {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (server.exitCode !== null) fail(`the ${name} server exited (code ${server.exitCode}); see test-results/ui-server-${name}.log`);
    try {
      const res = await fetch(`${url}/api/health`);
      // Another service on the port would answer too: only genie's health counts.
      if (res.ok && (await res.json()).ok === true) return;
    } catch {
      // Not listening yet.
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  fail(`the server at ${url} did not become healthy in 30 s`);
}

const seededAt = new Date().toISOString();
const [visualPort, flowsPort] = await Promise.all([freePort(), freePort()]);
const [visualUrl, flowsUrl] = await Promise.all([
  start("visual", visualPort, capture ? CAPTURE_TASKS : []),
  // The flows server alone carries the decision the keyboard walkthrough answers (FLOW_TASKS): on the
  // screenshots' server it would put a waiting task on /tasks and /board and change their baselines.
  start("flows", flowsPort, FLOW_TASKS),
]);

const uid = process.getuid?.() ?? 1000;
const gid = process.getgid?.() ?? 1000;
const docker = spawn(
  "docker",
  [
    "run", "--rm", "--init", "--network", "host", "--ipc", "host",
    "--user", `${uid}:${gid}`,
    "--volume", `${root}:/work`, "--workdir", "/work",
    "--env", "HOME=/tmp", "--env", "CI", "--env", `GENIE_UI_VISUAL_URL=${visualUrl}`, "--env", `GENIE_UI_FLOWS_URL=${flowsUrl}`, "--env", `GENIE_UI_SEEDED_AT=${seededAt}`, ...(capture ? ["--env", `GENIE_UI_CAPTURE=1`, "--env", `GENIE_UI_THEMES=${process.env.GENIE_UI_THEMES ?? "current"}`] : []),
    image,
    "npx", "playwright", "test", "--config", "test/ui/playwright.config.ts", ...(capture ? ["--project", "capture-desktop", "--project", "capture-phone"] : []), ...passthrough,
  ],
  { stdio: "inherit" },
);
const code: number = await new Promise((done) => docker.on("exit", (c) => done(c ?? 1)));
process.exit(code);
