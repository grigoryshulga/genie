// genie-guard: keeps an agent within its role in the harness.
//
// genie serve loads this extension into every agent it runs (live sessions and
// one-shot turns) and points it at the agent's policy file (`GENIE_POLICY`),
// written from the role before the start and rewritten when the agent
// configuration changes — so a new rule applies to a running agent at its next
// tool call. On each tool call it blocks:
//
// - shell commands matching the role's `denyCommands` (glob patterns, matched
//   against every simple command of the line and inside quoted strings);
// - edit and write when the role or the job's workspace is read-only;
// - MCP calls (pi's built-in MCP tools `mcp__<server>__<tool>`, also those
//   a codemode script makes) to connections and tools the role was not
//   granted, and tools of pi-mcp-adapter, which pi 1.0 cannot run next to its
//   native MCP support.
//
// It also gives the agent its MCP connections: pi's built-in MCP extension is
// replaced by an instance of the same code that reads no `mcp.json` of the
// machine's user or of the repository, only the role's connections from the
// file in `GENIE_MCP_CONFIG` (native `mcp.json` format, written by genie serve),
// and keeps no OAuth state or log in pi's directory.
//
// It also reports the tokens of every model response to the server
// (`POST /api/agent/usage`), so genie can count what each chat, task, epic and
// project cost; a report that fails is retried with the next one.
//
// These are soft limits: an agent with a shell can work around them. The MCP
// config already holds only the role's connections; the guard is the second
// line, and the only one for what pi lets through by name (see `mcpDenial`). The
// extension is written by genie serve into its data directory; edit
// crates/genie/pi/genie-guard.ts in the repository instead.

import { readFileSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

export interface Policy {
  role: string;
  /** `write`, `read` or `none`. */
  files: string;
  denyCommands: string[];
  /** Granted MCP connections: all their tools (`null`) or tool name patterns. */
  mcp: Record<string, string[] | null>;
}

const POLICY = process.env.GENIE_POLICY ?? "";
const MCP_CONFIG = process.env.GENIE_MCP_CONFIG ?? "";
const BASE = (process.env.GENIE_URL ?? "").replace(/\/+$/, "");
const TOKEN = process.env.GENIE_TOKEN ?? "";
/** Commands that run another command, with their options that take a value (and leading operands). */
const WRAPPERS: Record<string, { valued: string[]; operands?: number }> = {
  sudo: { valued: ["-u", "-g", "-h", "-p", "-C", "-D", "-r", "-t", "-U", "-T"] },
  env: { valued: ["-u", "-C", "-S"] },
  nice: { valued: ["-n"] },
  timeout: { valued: ["-s", "-k"], operands: 1 },
  xargs: { valued: ["-I", "-n", "-P", "-d", "-L", "-s", "-E", "-a"] },
  time: { valued: ["-f", "-o"] },
  exec: { valued: ["-a"] },
  command: { valued: [] },
  builtin: { valued: [] },
  nohup: { valued: [] },
};

/** A glob (`*` any text, `?` one character) as an anchored regular expression. */
function glob(pattern: string): RegExp {
  const body = pattern
    .trim()
    .replace(/\s+/g, " ")
    .split("")
    .map((c) => (c === "*" ? ".*" : c === "?" ? "." : c.replace(/[.+^${}()|[\]\\]/g, "\\$&")))
    .join("");
  return new RegExp(`^${body}$`, "s");
}

/** The simple commands of a shell line, without leading assignments and wrappers (`sudo`, `env`…). */
export function simpleCommands(line: string): string[] {
  const out: string[] = [];
  for (const part of line.split(/\|\||&&|\$\(|[;&|\n()`{}]/)) {
    const words = part.trim().split(/\s+/).filter(Boolean);
    while (words.length) {
      const w = words[0];
      const wrapper = Object.hasOwn(WRAPPERS, w) ? WRAPPERS[w] : undefined;
      if (/^[A-Za-z_][A-Za-z0-9_]*=/.test(w)) words.shift();
      else if (wrapper) {
        words.shift();
        while (words.length && words[0].startsWith("-")) {
          const opt = words.shift() as string;
          if (wrapper.valued.includes(opt)) words.shift();
        }
        words.splice(0, wrapper.operands ?? 0);
      } else break;
    }
    if (words.length) {
      words[0] = words[0].replace(/^.*\//, ""); // /usr/bin/git → git
      out.push(words.join(" "));
    }
  }
  return out;
}

/** The denyCommands pattern a shell line hits, if any (quoted strings are checked too: `bash -c '…'`). */
export function deniedBy(line: string, patterns: string[], depth = 0): string | undefined {
  const rules = patterns.map((p) => [p, glob(p)] as const);
  for (const cmd of simpleCommands(line)) {
    const hit = rules.find(([, re]) => re.test(cmd));
    if (hit) return hit[0];
  }
  if (depth < 2) {
    for (const q of line.matchAll(/'([^']*)'|"((?:\\.|[^"\\])*)"/g)) {
      const hit = deniedBy(q[1] ?? q[2] ?? "", patterns, depth + 1);
      if (hit) return hit;
    }
  }
  return undefined;
}

/** The tools of pi's MCP resource support: they take the `server` to look in. */
const RESOURCE_TOOLS = ["list_mcp_resources", "list_mcp_resource_templates", "read_mcp_resource"];
/** Tools pi has itself; they are not MCP's business and need no lookup of their source. */
const CORE_TOOLS = new Set(["read", "bash", "powershell", "edit", "write", "grep", "find", "ls", "codemode", "tool_search"]);

/** Why a call of `tool` on MCP connection `server` is outside the role's grants (undefined: allowed). */
export function mcpDenial(p: Policy, server: string, tool: string): string | undefined {
  const servers = Object.keys(p.mcp);
  if (!Object.hasOwn(p.mcp, server)) {
    return `genie: MCP connection ${server} is not granted. The role ${p.role} may use MCP connections: ${servers.join(", ") || "none"}`;
  }
  const patterns = p.mcp[server];
  if (!patterns || patterns.some((pt) => glob(pt).test(tool))) return undefined;
  return `genie: the role ${p.role} may use only these tools of ${server}: ${patterns.join(", ")}`;
}

/**
 * Why a call of pi's MCP resource tools is outside the grants. Resources come with the whole
 * connection: a role limited to some tools of it gets none (as at the gateway).
 */
export function resourceDenial(p: Policy, tool: string, input: Record<string, unknown>): string | undefined {
  if (!RESOURCE_TOOLS.includes(tool)) return undefined;
  const server = typeof input.server === "string" ? input.server : "";
  if (!server) return undefined; // a listing: pi's own config of the connections limits it
  if (!Object.hasOwn(p.mcp, server)) return mcpDenial(p, server, "");
  if (p.mcp[server]) return `genie: the role ${p.role} may use only some tools of ${server}, not its resources`;
  return undefined;
}

/** The server and the tool's own name behind a pi MCP tool, from its label (`<server>/<tool>`). */
export function mcpOwner(label: unknown): { server: string; tool: string } | undefined {
  const i = typeof label === "string" ? label.indexOf("/") : -1;
  return i > 0 && typeof label === "string" ? { server: label.slice(0, i), tool: label.slice(i + 1) } : undefined;
}

/** What a granted connection looks like to pi (its `mcp.json` entry): exposure and tool patterns are genie's. */
export interface McpConfig {
  mcpServers?: Record<string, Record<string, unknown>>;
  autoEnableCodemode?: boolean;
}

/** Read the role's MCP config; an unreadable or missing file is a role without connections. */
export function readMcpConfig(path: string): McpConfig {
  if (!path) return {};
  try {
    const raw = JSON.parse(readFileSync(path, "utf8"));
    return raw && typeof raw === "object" ? raw : {};
  } catch (e) {
    console.error(`[genie-guard] MCP config ${path}: ${e instanceof Error ? e.message : String(e)}; the agent has no MCP`);
    return {};
  }
}

/** OAuth state of MCP servers kept in memory: nothing of the machine user's sign-ins reaches an agent. */
function memoryCredentials(): any {
  const states = new Map<string, unknown>();
  const key = (name: string, url: string) => `${name}|${url}`;
  return {
    forServer: (name: string, url: string) => ({
      load: () => states.get(key(name, url)),
      save: (state: unknown) => void states.set(key(name, url), state),
      withRefreshLock: <T>(fn: () => T) => fn(),
    }),
    tokens: (name: string, url: string) => (states.get(key(name, url)) as any)?.tokens,
    remove: (name: string, url: string) => states.delete(key(name, url)),
  };
}

/**
 * Connect the role's MCP servers with pi's built-in MCP code, in place of pi's own extension (an
 * extension that registers `/mcp` replaces it), so that no other config is read. Returns the tool
 * name → connection and tool it was registered for, which the guard checks calls against.
 */
async function connectMcp(pi: any, config: McpConfig, configPath: string): Promise<Map<string, { server: string; tool: string }>> {
  const owners = new Map<string, { server: string; tool: string }>();
  const { createMcpExtension } = await import("@earendil-works/pi-coding-agent");
  for (const [name, entry] of Object.entries(config.mcpServers ?? {})) {
    try {
      pi.registerMcpServer(name, entry);
    } catch (e) {
      console.error(`[genie-guard] MCP connection ${name}: ${e instanceof Error ? e.message : String(e)}`);
    }
  }
  // pi's extension registers the tools through this object: note which connection's tool each is.
  const seen = new Proxy(pi, {
    get(target, prop) {
      if (prop !== "registerTool") return Reflect.get(target, prop);
      return (tool: any) => {
        const owner = mcpOwner(tool?.label);
        if (owner && typeof tool.name === "string" && tool.name.startsWith("mcp__")) owners.set(tool.name, owner);
        return target.registerTool(tool);
      };
    },
  });
  const dir = configPath ? dirname(configPath) : tmpdir();
  const factory = createMcpExtension({
    loadConfig: () => ({ servers: [], errors: [], autoEnableCodemode: config.autoEnableCodemode === true }),
    credentials: memoryCredentials(),
    logPath: join(dir, "mcp.log"),
  });
  await factory(seen as any);
  return owners;
}

/** Tokens of one model response, as `POST /api/agent/usage` takes them. */
export interface UsageReport {
  model: string;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
}

const count = (v: unknown) => (typeof v === "number" && Number.isFinite(v) && v > 0 ? Math.round(v) : 0);

/** The usage of an assistant message (pi's `AssistantMessage`), or nothing when it spent no tokens. */
export function usageOf(message: any): UsageReport | undefined {
  if (message?.role !== "assistant" || !message.usage) return undefined;
  const u = message.usage;
  const r = { input: count(u.input), output: count(u.output), cacheRead: count(u.cacheRead), cacheWrite: count(u.cacheWrite) };
  if (!r.input && !r.output && !r.cacheRead && !r.cacheWrite) return undefined;
  const model = String(message.model ?? "").trim();
  const provider = String(message.provider ?? "").trim();
  return { model: provider && model && !model.startsWith(`${provider}/`) ? `${provider}/${model}` : model || provider || "unknown", ...r };
}

function reportUsage(pi: any): void {
  if (!BASE || !TOKEN) return;
  let pending: UsageReport[] = [];
  let sending: Promise<void> = Promise.resolve();
  async function send(): Promise<void> {
    if (!pending.length) return;
    const batch = pending;
    pending = [];
    try {
      const res = await fetch(`${BASE}/api/agent/usage`, {
        method: "POST",
        headers: { authorization: `Bearer ${TOKEN}`, "content-type": "application/json" },
        body: JSON.stringify({ reports: batch }),
        signal: AbortSignal.timeout(10_000),
      });
      if (res.status >= 500) throw new Error(`HTTP ${res.status}`);
    } catch (e) {
      // Kept for the next response; a long outage keeps only the latest reports.
      pending = [...batch, ...pending].slice(-500);
      console.error(`[genie-guard] usage: ${e instanceof Error ? e.message : String(e)}`);
    }
  }
  /** One report at a time, in order; the agent does not wait for it. */
  const flush = () => (sending = sending.then(send));
  pi.on("message_end", (event: any) => {
    const r = usageOf(event?.message);
    if (!r) return;
    pending.push(r);
    void flush();
  });
  // A one-shot turn exits right after its answer: send what is left first.
  pi.on("agent_end", () => flush());
  pi.on("session_shutdown", () => flush());
}

export default async function genieGuard(pi: any) {
  reportUsage(pi);
  if (!POLICY) return;
  // Always: a role without connections still must not get the machine user's MCP servers.
  const owners = await connectMcp(pi, readMcpConfig(MCP_CONFIG), MCP_CONFIG);

  let policy: Policy | undefined;
  let stamp = "";
  let reported = false;
  function current(): Policy | undefined {
    try {
      const st = statSync(POLICY);
      const s = `${st.mtimeMs}:${st.size}`;
      if (s !== stamp) {
        const raw = JSON.parse(readFileSync(POLICY, "utf8"));
        policy = {
          role: String(raw.role ?? "?"),
          files: String(raw.files ?? "write"),
          denyCommands: Array.isArray(raw.denyCommands) ? raw.denyCommands.map(String) : [],
          mcp: raw.mcp && typeof raw.mcp === "object" ? raw.mcp : {},
        };
        stamp = s;
        reported = false;
      }
    } catch (e) {
      // Keep the last rules read; without any, MCP stays closed (below).
      if (!reported) console.error(`[genie-guard] policy ${POLICY}: ${e instanceof Error ? e.message : String(e)}`);
      reported = true;
    }
    return policy;
  }
  current();

  /** The source of a tool of pi-mcp-adapter, which has no place next to pi's native MCP support. */
  const adapterTool = (name: string) =>
    !CORE_TOOLS.has(name) && /pi-mcp-adapter/.test(String(pi.getAllTools().find((t: any) => t.name === name)?.sourceInfo?.path ?? ""));

  pi.on("tool_call", (event: any) => {
    const tool = String(event?.toolName ?? "");
    const input = (event?.input ?? {}) as Record<string, unknown>;
    const p = current();
    const mcp = owners.has(tool) || tool.startsWith("mcp__") || RESOURCE_TOOLS.includes(tool) || adapterTool(tool);
    if (!p) {
      return mcp ? { block: true, reason: "genie: the agent's rules could not be read; MCP is closed" } : undefined;
    }
    if ((tool === "edit" || tool === "write") && p.files !== "write") {
      return { block: true, reason: `genie: the role ${p.role} works read-only here; ${tool} is not available` };
    }
    if ((tool === "bash" || tool === "powershell") && p.denyCommands.length) {
      const hit = deniedBy(String(input.command ?? ""), p.denyCommands);
      if (hit) return { block: true, reason: `genie: the role ${p.role} may not run \`${hit}\` (denyCommands)` };
    }
    if (!mcp) return undefined;
    const owner = owners.get(tool);
    let why: string | undefined;
    if (owner) why = mcpDenial(p, owner.server, owner.tool);
    else if (RESOURCE_TOOLS.includes(tool)) why = resourceDenial(p, tool, input);
    // Neither registered for the role's connections nor pi's resource tools: not from genie's MCP wiring.
    else why = `genie: the tool ${tool} is not one of the role's MCP connections (pi-mcp-adapter must not be loaded next to pi's native MCP)`;
    return why ? { block: true, reason: why } : undefined;
  });
}
