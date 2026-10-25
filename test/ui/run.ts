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

import { spawn, type ChildProcess } from "node:child_process";
import { createWriteStream, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { genie, seedDocs, seedTask } from "./seeding.ts";
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

const run = (args: string[], input: string | undefined, dataDir: string) => genie(bin, args, input, dataDir);

// The seed the two servers share, plus the tasks only one of them may have. Each server gets its own
// freshly written copy: a data directory records its project's paths absolutely, so a copied directory
// would keep writing into the one it was copied from, and a task made for the flows server would turn
// up in the screenshots and in their committed baselines.
function seed(dataDir: string, extra: SeedTask[] = []) {
  run(["user", "add", OWNER.login, "--name", OWNER.name, "--admin", "--password-stdin"], `${OWNER.password}\n`, dataDir);
  run(["project", "add", PROJECT.id, "--name", PROJECT.name, "--prefix", PROJECT.prefix], undefined, dataDir);
  run(["member", PROJECT.id, OWNER.login, "owner"], undefined, dataDir);
  seedDocs(bin, dataDir, PROJECT.id, [FLOW_DOC, ...(capture ? CAPTURE_DOCS : [])]);
  const ids: string[] = [];
  for (const t of [...SEED_TASKS, ...extra]) seedTask(bin, dataDir, PROJECT.id, t, ids);
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
