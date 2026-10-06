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
// - MCP calls through pi-mcp-adapter (the `mcp` proxy tool and the
//   `mcp__<server>` wrappers) to connections and tools the role was not
//   granted, and installing MCP servers.
//
// It also reports the tokens of every model response to the server
// (`POST /api/agent/usage`), so genie can count what each chat, task, epic and
// project cost; a report that fails is retried with the next one.
//
// These are soft limits: an agent with a shell can work around them. The MCP
// config genie passes to pi-mcp-adapter (`--mcp-config`) already holds only the
// role's connections; the guard also covers an adapter started without it. The
// extension is written by genie serve into its data directory; edit
// crates/genie/pi/genie-guard.ts in the repository instead.

import { readFileSync, statSync } from "node:fs";

export interface Policy {
  role: string;
  /** `write`, `read` or `none`. */
  files: string;
  denyCommands: string[];
  /** Granted MCP connections: all their tools (`null`) or tool name patterns. */
  mcp: Record<string, string[] | null>;
}

const POLICY = process.env.GENIE_POLICY ?? "";
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

const norm = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, "_");

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

/** Why an MCP call is outside the role's grants (undefined: allowed). */
export function mcpDenial(p: Policy, tool: string, input: Record<string, unknown>): string | undefined {
  const servers = Object.keys(p.mcp);
  const granted = (name: string) => servers.find((s) => norm(s) === norm(name));
  // pi-mcp-adapter names a server's tools `<server>_<tool>`.
  const owner = (name: string) => servers.find((s) => norm(name).startsWith(`${norm(s)}_`));
  const deny = (why: string) => `genie: ${why}. The role ${p.role} may use MCP connections: ${servers.join(", ") || "none"}`;
  let server: string | undefined;
  if (tool === "mcp") {
    if (input.action === "install") {
      return "genie: agents do not install MCP servers; ask a genie admin to add the server to mcp.json and grant it to the role";
    }
    for (const key of ["server", "connect", "instructions"]) {
      const v = input[key];
      if (typeof v === "string" && v && !granted(v)) return deny(`MCP connection ${v} is not granted`);
    }
    server = typeof input.server === "string" && input.server ? granted(input.server) : undefined;
  } else if (tool.startsWith("mcp__")) {
    const rest = tool.slice(5);
    server = granted(rest);
    // `mcp__<server>_<tool>` is a direct tool (`toolPrefix: "mcp"`): the adapter's config limits those.
    if (!server) return owner(rest) ? undefined : deny(`MCP connection ${rest} is not granted`);
  } else {
    return undefined;
  }
  const name = typeof input.tool === "string" ? input.tool : "";
  if (!name) return undefined;
  const s = server ?? owner(name);
  if (!s) return deny(`the MCP tool ${name} is not from a granted connection (name its server)`);
  const patterns = p.mcp[s];
  if (!patterns) return undefined;
  const base = norm(name).startsWith(`${norm(s)}_`) ? name.slice(s.length + 1) : name;
  if (patterns.some((pt) => glob(pt).test(name) || glob(pt).test(base))) return undefined;
  return `genie: the role ${p.role} may use only these tools of ${s}: ${patterns.join(", ")}`;
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

/** `NODE_OPTIONS` without the heap cap genie put there for pi (`--max-old-space-size=<mb>`); other options stay. */
export function withoutHeapCap(options: string, mb: string): string {
  return options
    .split(/\s+/)
    .filter((o) => o && o !== `--max-old-space-size=${mb}`)
    .join(" ");
}

/** Node read the cap at start-up; what pi runs (builds, test runs) inherits the environment and must not be held to pi's limit. */
function releaseHeapCap(): void {
  const cap = process.env.GENIE_NODE_HEAP_MB;
  if (!cap) return;
  const rest = withoutHeapCap(process.env.NODE_OPTIONS ?? "", cap);
  if (rest) process.env.NODE_OPTIONS = rest;
  else delete process.env.NODE_OPTIONS;
  delete process.env.GENIE_NODE_HEAP_MB;
}

export default function genieGuard(pi: any) {
  releaseHeapCap();
  reportUsage(pi);
  if (!POLICY) return;

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

  pi.on("tool_call", (event: any) => {
    const tool = String(event?.toolName ?? "");
    const input = (event?.input ?? {}) as Record<string, unknown>;
    const p = current();
    if (!p) {
      return tool === "mcp" || tool.startsWith("mcp__") ? { block: true, reason: "genie: the agent's rules could not be read; MCP is closed" } : undefined;
    }
    if ((tool === "edit" || tool === "write") && p.files !== "write") {
      return { block: true, reason: `genie: the role ${p.role} works read-only here; ${tool} is not available` };
    }
    if ((tool === "bash" || tool === "powershell") && p.denyCommands.length) {
      const hit = deniedBy(String(input.command ?? ""), p.denyCommands);
      if (hit) return { block: true, reason: `genie: the role ${p.role} may not run \`${hit}\` (denyCommands)` };
    }
    const why = mcpDenial(p, tool, input);
    return why ? { block: true, reason: why } : undefined;
  });
}
