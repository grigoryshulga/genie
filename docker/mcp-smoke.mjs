#!/usr/bin/env node
// Regression smoke helper for a built image (run by docker/smoke-test.sh): it proves that Pi-Bolt
// resolves the pi SDK it bundles — `@earendil-works/pi-coding-agent`, which the genie guard
// extension imports — and that the genie guard extension starts a role's MCP connections over
// stdio. Nothing here calls a model or touches the network: the "MCP server" is this same file in
// `server` mode, started by pi from the `mcp.json` the driver writes.
//
//   node mcp-smoke.mjs --guard /path/to/genie-guard.ts [--timeout <ms>]
//   node mcp-smoke.mjs server        # the fake MCP server, started by pi, not by a person
//
// Exit code 0 only when both assertions hold; every failure prints what was and was not observed.
// The SDK must be probed from a `.ts` extension: Pi-Bolt resolves its bundled SDK for extensions it
// loads as TypeScript, while a `.mjs` extension goes through Node's own loader and cannot see it.

import { spawn } from "node:child_process";
import { appendFileSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";

const SELF = fileURLToPath(import.meta.url);
/** How often the driver re-reads the marker files while waiting; the wait itself is bounded, not fixed. */
const POLL_MS = 100;
/** Grace for `pi` to exit after SIGTERM before the process group is killed outright. */
const STOP_MS = 2000;

/** A tiny extension that reports whether pi's bundled SDK resolves and has the MCP factory the guard needs. */
const SDK_PROBE = `import { writeFileSync } from "node:fs";
export default async function () {
  const out = process.env.SDK_SMOKE_RESULT ?? "";
  try {
    const sdk = await import("@earendil-works/pi-coding-agent");
    writeFileSync(out, "ok createMcpExtension=" + typeof sdk.createMcpExtension);
  } catch (e) {
    writeFileSync(out, "fail " + (e && e.message ? e.message : String(e)));
  }
}
`;

/** The fake MCP server: answers the handshake, records it, and signals readiness after `tools/list`. */
function runServer() {
  const log = process.env.MCP_SMOKE_LOG ?? "";
  const ready = process.env.MCP_SMOKE_READY ?? "SMOKE_READY";
  const note = (line) => {
    if (log) appendFileSync(log, `${line}\n`);
  };
  note(`server started pid=${process.pid}`);
  const answer = (message) => process.stdout.write(`${JSON.stringify(message)}\n`);
  const rl = createInterface({ input: process.stdin });
  rl.on("line", (line) => {
    let message;
    try {
      message = JSON.parse(line);
    } catch {
      note(`unparsed ${line}`);
      return;
    }
    if (message.method === "initialize") {
      note("initialize");
      const version = message.params?.protocolVersion;
      answer({
        jsonrpc: "2.0",
        id: message.id,
        result: {
          protocolVersion: typeof version === "string" ? version : "2024-11-05",
          capabilities: { tools: {} },
          serverInfo: { name: "genie-smoke", version: "1.0.0" },
        },
      });
    } else if (message.method === "tools/list") {
      note("tools/list");
      answer({
        jsonrpc: "2.0",
        id: message.id,
        result: { tools: [{ name: "hello", description: "smoke test tool", inputSchema: { type: "object", properties: {} } }] },
      });
      // Reached only after the SDK import, the guard's MCP wiring and the handshake all worked.
      note(ready);
    } else if (message.id !== undefined) {
      answer({ jsonrpc: "2.0", id: message.id, result: {} });
    }
  });
  rl.on("close", () => process.exit(0));
}

const read = (path) => (existsSync(path) ? readFileSync(path, "utf8") : "");
const indent = (text) =>
  text
    .split("\n")
    .map((line) => `      ${line}`)
    .join("\n");

/** Stop pi and everything it started (the fake MCP server) as one process group. */
async function stop(child, exited) {
  if (!child.pid) return;
  try {
    process.kill(-child.pid, "SIGTERM");
  } catch {
    // Already gone.
  }
  const hard = setTimeout(() => {
    try {
      process.kill(-child.pid, "SIGKILL");
    } catch {
      // Already gone.
    }
  }, STOP_MS);
  try {
    await Promise.race([exited, sleep(STOP_MS + 3000)]);
  } finally {
    clearTimeout(hard);
  }
}

/** Run pi with the guard extension and a fake MCP connection; assert the SDK and the MCP session. */
async function runDriver(guard, timeoutMs) {
  if (!guard) {
    console.error("usage: mcp-smoke.mjs --guard <genie-guard.ts> [--timeout <ms>]");
    return 2;
  }
  if (!existsSync(guard)) {
    console.error(`pi harness smoke test FAILED:\n  - the guard extension is missing: ${guard}`);
    return 1;
  }

  const dir = mkdtempSync(join(tmpdir(), "genie-mcp-smoke-"));
  const policyPath = join(dir, "policy.json");
  const mcpPath = join(dir, "mcp.json");
  const probePath = join(dir, "sdk-probe.ts");
  const sdkPath = join(dir, "sdk.result");
  const logPath = join(dir, "mcp.log");
  const ready = `SMOKE_READY_${process.pid}_${Date.now()}`;
  writeFileSync(policyPath, JSON.stringify({ role: "smoke", files: "write", denyCommands: [], mcp: { "genie-smoke": null } }));
  writeFileSync(mcpPath, JSON.stringify({ mcpServers: { "genie-smoke": { command: process.execPath, args: [SELF, "server"] } } }));
  writeFileSync(probePath, SDK_PROBE);

  const started = Date.now();
  const child = spawn("pi", ["--mode", "rpc", "--no-session", "-e", probePath, "-e", guard], {
    detached: true,
    stdio: ["pipe", "pipe", "pipe"],
    env: {
      ...process.env,
      // No provider is contacted and no model is chosen: the session only loads the extensions.
      PI_OFFLINE: "1",
      GENIE_POLICY: policyPath,
      GENIE_MCP_CONFIG: mcpPath,
      MCP_SMOKE_LOG: logPath,
      MCP_SMOKE_READY: ready,
      SDK_SMOKE_RESULT: sdkPath,
    },
  });
  let out = "";
  let err = "";
  child.stdout.on("data", (chunk) => {
    out = (out + chunk).slice(-8192);
  });
  child.stderr.on("data", (chunk) => {
    err = (err + chunk).slice(-8192);
  });
  const spawnError = new Promise((resolve) => child.on("error", resolve));
  child.on("error", (e) => {
    err += `spawn error: ${e.message}\n`;
  });
  const exited = new Promise((resolve) => child.on("exit", (code, signal) => resolve({ code, signal })));
  // A failed spawn emits `error` but not always `exit`: settle on whichever arrives first.
  if (!child.pid) await Promise.race([spawnError, sleep(1000)]);

  let sdk = "";
  let log = "";
  let mcp = false;
  if (child.pid) {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      sdk = read(sdkPath);
      log = read(logPath);
      mcp = log.includes(ready);
      if ((sdk && mcp) || child.exitCode !== null || child.signalCode !== null || Date.now() >= deadline) break;
      await sleep(POLL_MS);
    }
  }

  const elapsed = Date.now() - started;
  await stop(child, exited);
  rmSync(dir, { recursive: true, force: true });

  const problems = [];
  if (!child.pid) problems.push("could not start pi");
  if (!sdk) problems.push("the pi SDK probe produced no result");
  else if (!sdk.startsWith("ok")) problems.push(`the pi SDK does not resolve inside pi: ${sdk.slice(5)}`);
  else if (!sdk.includes("createMcpExtension=function")) problems.push(`the pi SDK has no createMcpExtension: ${sdk}`);
  if (!mcp) problems.push(`the guard extension did not start an MCP session within ${timeoutMs} ms`);

  if (problems.length) {
    console.error("pi harness smoke test FAILED:");
    for (const problem of problems) console.error(`  - ${problem}`);
    console.error(`  guard: ${guard}`);
    console.error(`  SDK probe result: ${sdk || "(none)"}`);
    console.error(`  fake MCP server log: ${log.trim() ? `\n${indent(log.trim())}` : "(empty: pi never started the server)"}`);
    if (err.trim()) console.error(`  pi stderr:\n${indent(err.trim())}`);
    if (out.trim()) console.error(`  pi stdout (tail):\n${indent(out.trim())}`);
    return 1;
  }
  console.log(`pi harness: the SDK resolves and the guard connects MCP (${elapsed} ms)`);
  return 0;
}

if (process.argv.includes("server")) {
  runServer();
} else {
  const args = process.argv.slice(2);
  const guardIndex = args.indexOf("--guard");
  const guard = guardIndex === -1 ? undefined : args[guardIndex + 1];
  const timeoutIndex = args.indexOf("--timeout");
  const timeoutMs = timeoutIndex === -1 ? 20_000 : Number(args[timeoutIndex + 1]);
  process.exit(await runDriver(guard, Number.isFinite(timeoutMs) && timeoutMs > 0 ? timeoutMs : 20_000));
}
