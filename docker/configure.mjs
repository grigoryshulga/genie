// Container start-up configuration, run by docker-entrypoint.sh as the service user.
//
// The server reads its settings only from <data>/config.json, so environment variables are
// applied to that file here (idempotently, and only for the keys they cover):
//
//   GENIE_BIND         bind address        (default 0.0.0.0 — a loopback bind is unreachable through published ports)
//   GENIE_PUBLIC_URL   base URL of links in mail and Telegram; its host is also added to allowHosts
//   GENIE_ALLOW_HOSTS  comma-separated Host header values accepted besides localhost:<port>
//   GENIE_SANDBOX      runtime.sandbox.mode: auto | bwrap | off (see docker-compose.sandbox.yml)
//
// With a Rust toolchain in the image (build argument RUST_TOOLCHAIN) agents share one build cache,
// <data>/cache: cargo's downloads and one target directory for everyone (dependencies are built once,
// and builds of several agents take turns on the disk instead of all linking at once), mold as the
// linker and a bounded number of build jobs. Each value is a default: set it in runtime.env to
// override it, or GENIE_AGENT_CACHE=0 to leave the agents' build settings alone.
//
//   GENIE_AGENT_BUILD_JOBS  CARGO_BUILD_JOBS of agents (default 8)
//
// Everything else in config.json is left alone. The script also takes pi-mcp-adapter out of pi's
// settings: earlier images registered it there, and on pi 1.0 it would replace pi's built-in MCP
// support, through which agents get the connections of their roles.

import { chmodSync, existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const env = process.env;
const data = env.GENIE_DATA || "/data";
const log = (msg) => console.error(`genie-entrypoint: ${msg}`);

function readJson(path, fallback) {
  if (!existsSync(path)) return fallback;
  try {
    const value = JSON.parse(readFileSync(path, "utf8"));
    if (value && typeof value === "object" && !Array.isArray(value)) return value;
  } catch (e) {
    log(`error: ${path} is not valid JSON (${e.message}); fix or remove it`);
    process.exit(1);
  }
  log(`error: ${path} must contain a JSON object`);
  process.exit(1);
}

function writeJson(path, value, mode) {
  mkdirSync(dirname(path), { recursive: true });
  const tmp = `${path}.tmp`;
  writeFileSync(tmp, `${JSON.stringify(value, null, 2)}\n`, { mode });
  chmodSync(tmp, mode);
  renameSync(tmp, path);
}

// --- <data>/config.json ---------------------------------------------------------------

const configPath = join(data, "config.json");
const config = readJson(configPath, {});
const before = JSON.stringify(config);
const fresh = !existsSync(configPath);

const bind = (env.GENIE_BIND || "").trim();
if (bind) config.bind = bind;
else if (config.bind === undefined) config.bind = "0.0.0.0";
else if (["127.0.0.1", "localhost", "::1"].includes(config.bind)) {
  log(
    `warning: config.json binds ${config.bind}: the server is not reachable through a published port. ` +
      "Set bind to 0.0.0.0 (or GENIE_BIND=0.0.0.0) unless the container uses host networking.",
  );
}

const hosts = new Set(Array.isArray(config.allowHosts) ? config.allowHosts : []);
for (const h of (env.GENIE_ALLOW_HOSTS || "").split(",")) if (h.trim()) hosts.add(h.trim());

const publicUrl = (env.GENIE_PUBLIC_URL || "").trim();
if (publicUrl) {
  let url;
  try {
    url = new URL(publicUrl);
  } catch {
    log(`error: GENIE_PUBLIC_URL is not a valid URL: ${publicUrl}`);
    process.exit(1);
  }
  config.publicUrl = publicUrl.replace(/\/+$/, "");
  hosts.add(url.host); // as clients send it: with the port when it is not the default one
  hosts.add(url.hostname);
}
if (hosts.size > 0) config.allowHosts = [...hosts];

const sandbox = (env.GENIE_SANDBOX || "").trim();
if (sandbox) {
  if (!["auto", "bwrap", "off"].includes(sandbox)) {
    log(`error: GENIE_SANDBOX must be auto, bwrap or off, got '${sandbox}'`);
    process.exit(1);
  }
  const runtime = config.runtime && typeof config.runtime === "object" ? config.runtime : {};
  const box = runtime.sandbox && typeof runtime.sandbox === "object" ? runtime.sandbox : {};
  config.runtime = { ...runtime, sandbox: { ...box, mode: sandbox } };
}

if (existsSync(env.GENIE_RUST_DIR || "/opt/rust") && env.GENIE_AGENT_CACHE !== "0") {
  const cache = join(data, "cache");
  mkdirSync(join(cache, "cargo"), { recursive: true });
  const runtime = config.runtime && typeof config.runtime === "object" ? config.runtime : {};
  const agentEnv = runtime.env && typeof runtime.env === "object" ? { ...runtime.env } : {};
  const defaults = {
    CARGO_HOME: join(cache, "cargo"),
    CARGO_TARGET_DIR: join(cache, "cargo-target"),
    CARGO_BUILD_JOBS: (env.GENIE_AGENT_BUILD_JOBS || "8").trim(),
  };
  const triple = { x64: "X86_64", arm64: "AARCH64" }[process.arch];
  if (triple && existsSync("/usr/bin/mold")) defaults[`CARGO_TARGET_${triple}_UNKNOWN_LINUX_GNU_RUSTFLAGS`] = "-C link-arg=-fuse-ld=mold";
  for (const [k, v] of Object.entries(defaults)) if (agentEnv[k] === undefined) agentEnv[k] = v;
  // The data directory is hidden from sandboxed agents; the cache shows through, writable.
  const box = runtime.sandbox && typeof runtime.sandbox === "object" ? runtime.sandbox : {};
  const writable = Array.isArray(box.writable) ? box.writable : [];
  config.runtime = { ...runtime, env: agentEnv, sandbox: { ...box, writable: writable.includes(cache) ? writable : [...writable, cache] } };
}

if (JSON.stringify(config) !== before || fresh) {
  writeJson(configPath, config, 0o600);
  log(`${fresh ? "created" : "updated"} ${configPath}`);
}

// --- pi: no pi-mcp-adapter ------------------------------------------------------------

const agentDir = env.PI_CODING_AGENT_DIR || join(env.HOME || "/data/home", ".pi", "agent");
const settingsPath = join(agentDir, "settings.json");
if (existsSync(settingsPath)) {
  const settings = readJson(settingsPath, {});
  const source = (p) => (typeof p === "string" ? p : p && typeof p.source === "string" ? p.source : "");
  // Any entry naming the adapter counts (npm:pi-mcp-adapter@x, a git source, the path the old image used).
  let removed = false;
  for (const key of ["packages", "extensions"]) {
    if (!Array.isArray(settings[key])) continue;
    const kept = settings[key].filter((p) => !source(p).includes("pi-mcp-adapter"));
    if (kept.length !== settings[key].length) {
      settings[key] = kept;
      removed = true;
    }
  }
  if (removed) {
    writeJson(settingsPath, settings, 0o600);
    log(`removed pi-mcp-adapter from ${settingsPath}: pi's built-in MCP support serves the agents`);
  }
}
const strayAdapter = join(agentDir, "extensions", "pi-mcp-adapter");
if (existsSync(strayAdapter)) log(`warning: ${strayAdapter} replaces pi's built-in MCP support: remove it`);

