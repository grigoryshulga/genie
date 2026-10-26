// The development stand: a disposable genie server on rich, varied data, with Vite's hot reload in front of it.
// Edit web/src and see the change in the browser at once: no image build, no CI, no preview candidate.
//
//   npm run dev:stand                 start (seeds on the first run, then reuses the data)
//   npm run dev:stand -- --reset      throw the data away and seed it again
//   npm run dev:stand -- --no-build   skip `cargo build` (the binary is built only when Rust sources changed)
//   npm run dev:stand -- --clean      remove this worktree's stand data and exit
//
// Everything lives in one directory per worktree under $XDG_CACHE_HOME/genie/stand (no Docker volumes, no
// containers). Rust output is shared by all worktrees in $XDG_CACHE_HOME/genie/target, so a new worktree does not
// compile Genie from scratch. The server listens on loopback only and runs without agents.

import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { createHash } from "node:crypto";
import { createWriteStream, existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { basename, join, resolve } from "node:path";
import { genie, seedDocs, seedTask } from "./seeding.ts";
import { OWNER } from "./seed.ts";
import { PEOPLE, STAND_PROJECTS } from "./stand-data.ts";
import { TEMPLATE_FILE } from "./template-data.ts";
import { ROLE_FILE } from "./role-data.ts";
import { seedTeam } from "./stand-team.ts";

const root = resolve(import.meta.dirname, "../..");
const args = new Set(process.argv.slice(2));
const cache = join(process.env.XDG_CACHE_HOME ?? join(homedir(), ".cache"), "genie");
const targetDir = process.env.CARGO_TARGET_DIR ?? join(cache, "target");
const name = basename(root).replace(/[^\w.-]/g, "_");
const dataDir = join(cache, "stand", name);
const bin = process.env.GENIE_BIN ?? join(targetDir, "debug/genie");
// Stable ports per worktree: two worktrees can run stands side by side, and a restart keeps the bookmarks.
const slot = createHash("sha1").update(root).digest().readUInt16BE(0) % 100;
const apiPort = Number(process.env.GENIE_STAND_API_PORT ?? 7600 + slot);
const webPort = Number(process.env.GENIE_STAND_WEB_PORT ?? 5200 + slot);

function fail(message: string): never {
  console.error(`stand: ${message}`);
  process.exit(1);
}

if (args.has("--clean")) {
  rmSync(dataDir, { recursive: true, force: true });
  console.log(`removed ${dataDir}`);
  process.exit(0);
}
if (!existsSync(join(root, "node_modules/vite"))) fail("no node_modules: run `npm ci` in this worktree first");

if (!args.has("--no-build") && !process.env.GENIE_BIN) {
  console.log(`building genie (shared target: ${targetDir})`);
  execFileSync("cargo", ["build", "-p", "genie"], { cwd: root, stdio: "inherit", env: { ...process.env, CARGO_TARGET_DIR: targetDir } });
}
if (!existsSync(bin)) fail(`no genie binary at ${bin}`);

function seed() {
  rmSync(dataDir, { recursive: true, force: true });
  mkdirSync(dataDir, { recursive: true });
  const run = (a: string[], input?: string) => genie(bin, a, input, dataDir);
  for (const p of PEOPLE) run(["user", "add", p.login, "--name", p.name, ...(p.admin ? ["--admin"] : []), "--password-stdin"], `${p.password}\n`);
  for (const project of STAND_PROJECTS) {
    run(["project", "add", project.id, "--name", project.name, "--prefix", project.prefix]);
    for (const [login, role] of project.members) run(["member", project.id, login, role]);
    seedDocs(bin, dataDir, project.id, project.docs);
    const ids: string[] = [];
    for (const task of project.tasks) seedTask(bin, dataDir, project.id, task, ids);
    console.log(`  ${project.name}: ${project.tasks.length} tasks, ${project.docs.length} pages`);
  }
  mkdirSync(join(dataDir, "teams"), { recursive: true });
  writeFileSync(join(dataDir, "teams/integration-review.json"), JSON.stringify(TEMPLATE_FILE, null, 2));
  mkdirSync(join(dataDir, "agents"), { recursive: true });
  writeFileSync(join(dataDir, "agents/integration-analyst.md"), ROLE_FILE);
  writeFileSync(join(dataDir, "mcp.json"), JSON.stringify({ mcpServers: {
    knowledge: { url: "http://127.0.0.1:9/knowledge", description: "База знаний команды" },
    issues: { url: "http://127.0.0.1:9/issues", description: "Задачи проекта" },
  } }));
  console.log(`  a running team with mail and a saved agent conversation: ${seedTeam(bin, dataDir)}`);
}

if (args.has("--reset") || !existsSync(join(dataDir, "server.db"))) {
  console.log(`seeding ${dataDir}`);
  seed();
}

let stopping = false;
const children: ChildProcess[] = [];
function stop() {
  stopping = true;
  for (const child of children) child.kill();
}
process.on("exit", stop);
for (const signal of ["SIGINT", "SIGTERM"] as const) process.on(signal, () => process.exit(130));

const log = createWriteStream(join(dataDir, "server.log"), { flags: "a" });
const server = spawn(bin, ["--data", dataDir, "serve", "--port", String(apiPort), "--no-agents"], { stdio: ["ignore", "pipe", "pipe"] });
server.stdout?.pipe(log);
server.stderr?.pipe(log);
children.push(server);
server.on("exit", (code) => !stopping && fail(`the server exited (code ${code}); see ${join(dataDir, "server.log")}`));

for (let tries = 0; ; tries++) {
  try {
    const res = await fetch(`http://127.0.0.1:${apiPort}/api/health`);
    if (res.ok && (await res.json()).ok === true) break;
  } catch {
    // Not listening yet.
  }
  if (tries > 150) fail(`the server did not become healthy; see ${join(dataDir, "server.log")}`);
  await new Promise((r) => setTimeout(r, 200));
}

const vite = spawn("npm", ["run", "dev:web", "--", "--port", String(webPort), "--strictPort", "--host", "127.0.0.1"], {
  cwd: root,
  stdio: "inherit",
  env: { ...process.env, GENIE_API_PROXY_TARGET: `http://127.0.0.1:${apiPort}` },
});
children.push(vite);
vite.on("exit", (code) => process.exit(code ?? 0));

console.log(`
  Genie stand  http://127.0.0.1:${webPort}   (API ${apiPort}, data ${dataDir})
  Sign in as   ${OWNER.login} / ${OWNER.password}   (also: ivan, chen, guest / stand-password)
  Reset data   npm run dev:stand -- --reset      Remove it  npm run dev:stand -- --clean
`);
