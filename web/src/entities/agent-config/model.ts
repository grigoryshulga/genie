// Agent configuration as the server shows it (GET /api/agent-config and the
// item endpoints): roles, team templates, skills, MCP connections. Pure data and
// helpers only — no imports — so the node tests can load this file directly.

export type RoleClass = "analyst" | "executor" | "reviewer" | "tester" | "documenter" | "orchestrator";
export type Origin = "builtin" | "override" | "custom" | "legacy";
export type FileAccess = "write" | "read" | "none";
export type Stage = "refinement" | "delivery";
export type Workspace = "worktree" | "repo" | "scratch";
export type MailMode = "open" | "flow";
export type RelKind = "handoff" | "returns" | "reports" | "consults";

export interface RoleDef {
  id: string;
  title: string;
  description: string;
  class: RoleClass;
  extends?: string;
  model?: string;
  thinking?: string;
  names: string[];
  allow: string[];
  deny: string[];
  /** The effective permissions (class set with allow/deny applied). */
  capabilities: string[];
  files: FileAccess;
  denyCommands: string[];
  /** Absent: every skill installed for the harness. */
  skills?: string[];
  mcp: string[];
  stages: Stage[];
  projects?: string[];
  instructions?: string;
  /** Only on the role endpoint. */
  prompt?: string;
  origin: Origin;
  path?: string;
  stale?: boolean;
}

export interface MemberDef {
  key: string;
  role: string;
  name?: string;
  model?: string;
  thinking?: string;
  instructions?: string;
}

export interface Relation {
  from: string;
  to: string[];
  type: RelKind;
  on?: string;
  note?: string;
}

export interface TeamDef {
  id: string;
  title: string;
  description: string;
  stage: Stage;
  workspace: Workspace;
  mail: MailMode;
  members: MemberDef[];
  relations: Relation[];
  relationsDerived: boolean;
  charter?: string;
  projects?: string[];
  origin: Origin;
  path?: string;
  warnings: string[];
  stale?: boolean;
}

export interface SkillDef {
  name: string;
  description: string;
  dir: string;
}

export interface McpServer {
  id: string;
  description: string;
  transport: "stdio" | "http";
  projects?: string[];
  /** Agents reach it through the genie gateway; `false`: the harness gets the entry itself. */
  gateway: boolean;
}

/** A connection started as agents would get it: its tools, or why it does not start. */
export interface McpCheck {
  ok: boolean;
  ms: number;
  error?: string;
  serverInfo?: { name?: string; version?: string } | null;
  protocolVersion?: string | null;
  instructions?: string | null;
  tools?: { name: string; description?: string | null }[];
}

/** A tool call through the gateway: an `mcp.called` event of the project's journal. */
export interface McpCall {
  id: number;
  at: string;
  actor: string;
  actorRole: string;
  /** The team (task) of a team member. */
  subject?: string;
  payload: { server: string; tool: string; ok: boolean; ms: number; role: string; args?: string; error?: string; refused?: boolean; job?: number };
}

/** `*` matches any text, `?` one character (as the gateway matches tool grants). */
export function globMatch(pattern: string, text: string): boolean {
  const re = pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".");
  return new RegExp(`^${re}$`, "s").test(text);
}

/** Roles that may use a tool of a connection: granted all of it (`server`, `*`) or a matching `server:pattern`. */
export function toolUsers(roles: RoleDef[], server: string, tool: string): string[] {
  return roles
    .filter((r) => r.mcp.some((g) => g === "*" || g === server || (g.startsWith(`${server}:`) && globMatch(g.slice(server.length + 1), tool))))
    .map((r) => r.id);
}

export interface Problem {
  level: "error" | "warning";
  item: string;
  path?: string;
  message: string;
}

export interface Catalogue {
  project: string;
  admin: boolean;
  /** pi still loads pi-mcp-adapter, which replaces its native MCP support (the agents' connections). */
  mcpAdapterLoaded: boolean;
  /** Agents reach MCP connections through the genie gateway (`runtime.mcpGateway`). */
  mcpGateway: boolean;
  /** Whether agents run in the bubblewrap sandbox (`runtime.sandbox`), and why not. */
  sandbox?: { active: boolean; note: string };
  /** How many automations name each template and role. */
  automations?: { templates: Record<string, number>; roles: Record<string, number> };
  roles: RoleDef[];
  teams: TeamDef[];
  skills: SkillDef[];
  mcp: McpServer[];
  problems: Problem[];
  permissions: string[];
  /** The permissions each class starts from. */
  classes: Record<string, string[]>;
}

export interface FileState {
  path: string;
  content: string | null;
  hash: string;
}

export interface RoleDetail {
  role: RoleDef;
  file: FileState;
  builtin: string | null;
  usedBy: { templates: string[]; automations: AutomationRef[] };
  problems: Problem[];
  admin: boolean;
}

export interface TemplateDetail {
  template: TeamDef;
  file: FileState;
  builtin: string | null;
  usedBy: { automations: AutomationRef[] };
  problems: Problem[];
  admin: boolean;
}

export interface SkillDetail {
  skill: SkillDef;
  content: string | null;
  hash: string;
  files: string[];
  editable: boolean;
  usedBy: string[];
  admin: boolean;
}

export interface McpDetail {
  servers: McpServer[];
  admin: boolean;
  file?: FileState;
  problems?: Problem[];
}

export interface AutomationRef {
  id: number;
  project: string | null;
  name: string;
}

export interface ConfigChange {
  id: number;
  at: string;
  user: string;
  item: string;
  path: string;
  before?: string;
  after?: string;
}

export interface Preview {
  template: string;
  task: string;
  members: { key: string; name: string; role: string; kickoff: string }[];
  warnings: string[];
}

// ------------------------------------------------------------------ names shown to people

export const CLASS_TITLE: Record<RoleClass, string> = {
  analyst: "аналитик",
  executor: "исполнитель",
  reviewer: "ревьюер",
  tester: "тестировщик",
  documenter: "документатор",
  orchestrator: "оркестратор",
};

export const ORIGIN_TITLE: Record<Origin, string> = {
  builtin: "встроенная",
  override: "изменена",
  custom: "своя",
  legacy: "из config.json",
};

export const FILES_TITLE: Record<FileAccess, string> = { write: "читает и пишет", read: "только читает", none: "без файлов проекта" };
export const STAGE_TITLE: Record<Stage, string> = { refinement: "разбор", delivery: "работа" };
export const WORKSPACE_TITLE: Record<Workspace, string> = { worktree: "свой worktree", repo: "основная рабочая копия", scratch: "пустой каталог" };
export const MAIL_TITLE: Record<MailMode, string> = { open: "пишут кому угодно", flow: "по связям шаблона" };
export const RELATION_TITLE: Record<RelKind, string> = { handoff: "передаёт работу", returns: "возвращает", reports: "докладывает", consults: "советуется" };
export const ON_STATUSES = ["refining", "ready", "in_progress", "review", "changes_requested", "approved"];
/** A relation's task status as a label: when the handoff happens. */
export const ON_TITLE: Record<string, string> = {
  refining: "уточнение",
  ready: "готово к работе",
  in_progress: "в работе",
  review: "на ревью",
  changes_requested: "доработка",
  approved: "одобрено",
};

/** Permission groups with what each permission gives. */
export const PERMISSION_GROUPS: { title: string; items: { id: string; text: string }[] }[] = [
  {
    title: "Статусы задачи",
    items: [
      { id: "status.refine", text: "черновик → уточнение" },
      { id: "status.start", text: "готово к работе → в работе" },
      { id: "status.rework", text: "доработка → в работе" },
      { id: "status.submit", text: "в работе → на ревью" },
      { id: "status.approve", text: "на ревью → одобрено" },
      { id: "status.return", text: "на ревью → доработка" },
    ],
  },
  {
    title: "Задача",
    items: [
      { id: "task.scope", text: "название, тип, описание, критерии, зависимости, эпик" },
      { id: "task.plan", text: "план реализации" },
      { id: "task.check", text: "отмечать критерии приёмки" },
      { id: "task.create", text: "подзадачи своей задачи" },
      { id: "task.block", text: "блокировать и снимать блок" },
    ],
  },
  {
    title: "Знания",
    items: [
      { id: "docs.read", text: "поиск и чтение базы знаний" },
      { id: "docs.write", text: "запись страниц по политике раздела" },
    ],
  },
  {
    title: "Общение",
    items: [
      { id: "mail.team", text: "письма и вопросы команде" },
      { id: "mail.orchestrator", text: "письма оркестратору напрямую, в обход «голоса команды»" },
      { id: "team.peek", text: "смотреть, чем занят сосед" },
    ],
  },
];

/** What stays with the orchestrator and people whatever the role says. */
export const FIXED_PERMISSIONS =
  "Всегда только у оркестратора и людей: статусы «черновик», «готово к работе», «нужно решение», «готово» и «отменено», нарезка задач, приоритет, стратегия интеграции, исполнители, прерывание и пауза агентов. Сдавший работу не может сам её одобрить.";

// ------------------------------------------------------------------ permissions

/** The set a role's `allow`/`deny` adjust: its parent's permissions, else its class's. */
export function basePermissions(role: RoleDef, roles: RoleDef[], classes: Record<string, string[]>): string[] {
  const parent = role.extends ? roles.find((r) => r.id === role.extends) : undefined;
  return parent ? parent.capabilities : (classes[role.class] ?? []);
}

/** `allow` and `deny` that turn `base` into `wanted`. */
export function allowDeny(base: string[], wanted: string[], order: string[]): { allow: string[]; deny: string[] } {
  const inOrder = (xs: string[]) => order.filter((c) => xs.includes(c));
  return { allow: inOrder(wanted.filter((c) => !base.includes(c))), deny: inOrder(base.filter((c) => !wanted.includes(c))) };
}

// ------------------------------------------------------------------ role files (frontmatter)

/** The frontmatter lines and the body of a role file (`undefined`: no frontmatter). */
export function splitFrontmatter(text: string): { lines: string[]; body: string } | undefined {
  const t = text.replace(/^﻿/, "");
  if (!/^---\r?\n/.test(t)) return undefined;
  const lines = t.slice(t.indexOf("\n") + 1).split("\n");
  const end = lines.findIndex((l) => {
    const x = l.replace(/\r$/, "");
    return x === "---" || x === "...";
  });
  if (end < 0) return undefined;
  return { lines: lines.slice(0, end).map((l) => l.replace(/\r$/, "")), body: lines.slice(end + 1).join("\n") };
}

const quote = (s: string) => `"${s.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
const plainItem = (s: string) => /^[A-Za-z0-9._/-]+$/.test(s);
const plainScalar = (s: string) => s !== "" && !/^[\s"'[|>]/.test(s) && !/\s$/.test(s);

function renderField(key: string, value: string | string[]): string[] {
  if (Array.isArray(value)) return [`${key}: [${value.map((v) => (plainItem(v) ? v : quote(v))).join(", ")}]`];
  if (value.includes("\n")) return [`${key}: |`, ...value.replace(/\n+$/, "").split("\n").map((l) => (l ? `  ${l}` : ""))];
  return [`${key}: ${plainScalar(value) ? value : quote(value)}`];
}

/**
 * Set (or, with `undefined`, remove) one frontmatter field of a role file,
 * keeping the other lines and the body as they are. The file format is the
 * server's flat YAML subset: `key: value`, `[a, b]`, indented `- item` lines
 * and `|` blocks.
 */
export function setFrontmatterKey(text: string, key: string, value: string | string[] | undefined): string {
  const fm = splitFrontmatter(text) ?? { lines: [], body: text };
  const lines = [...fm.lines];
  const start = lines.findIndex((l) => l.startsWith(`${key}:`) || new RegExp(`^${key.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s+:`).test(l));
  const rendered = value === undefined ? [] : renderField(key, value);
  if (start >= 0) {
    const block = /^\s*[|>]/.test(lines[start].slice(lines[start].indexOf(":") + 1));
    let end = start + 1;
    while (end < lines.length && (/^[ \t]/.test(lines[end]) || (block && lines[end].trim() === ""))) end++;
    lines.splice(start, end - start, ...rendered);
  } else {
    lines.push(...rendered);
  }
  const head = lines.length ? `---\n${lines.join("\n")}\n---\n` : "---\n---\n";
  return head + fm.body;
}

/**
 * The lines of a file that problems point at (1-based line → messages): the
 * line a message names (`line 3: …`, `… at line 5 column 2`), or else the line
 * of the field it starts with (`files: x`, "`stage: y`: …") — a frontmatter
 * field or a JSON key.
 */
export function problemLines(text: string, messages: string[]): Map<number, string[]> {
  const lines = text.split("\n");
  const out = new Map<number, string[]>();
  const add = (n: number, m: string) => {
    if (n >= 1 && n <= lines.length) out.set(n, [...(out.get(n) ?? []), m]);
  };
  for (const m of messages) {
    const at = /\bline (\d+)/.exec(m);
    if (at) {
      add(Number(at[1]), m);
      continue;
    }
    const key = /^`?([A-Za-z][\w-]*)(?::|`)/.exec(m)?.[1];
    if (!key) continue;
    const i = lines.findIndex((l) => l.startsWith(`${key}:`) || l.trimStart().startsWith(`"${key}":`) || l.trimStart().startsWith(`"${key}" :`));
    if (i >= 0) add(i + 1, m);
  }
  return out;
}

/** The messages of a refused save (`not saved: role:x: line 3: …; team:y: …`) without the items they are about. */
export function refusalMessages(text: string): string[] {
  return text
    .replace(/^not saved:\s*/, "")
    .split("; ")
    .map((m) => m.replace(/^(?:role|team|skill|mcp)(?::[\w.-]+)?:\s*/, ""));
}

/** Replace the body (the prompt) of a role file, keeping its frontmatter as it is. */
export function setBody(text: string, body: string): string {
  const fm = splitFrontmatter(text);
  const b = body.replace(/\s+$/, "");
  if (!fm) return b ? `${b}\n` : "";
  const head = fm.lines.length ? `---\n${fm.lines.join("\n")}\n---\n` : "---\n---\n";
  return b ? `${head}${b}\n` : head;
}

/** A new role file: a class (or a role to extend), a title and a prompt. */
export function newRoleFile(o: { title: string; description: string; base?: string; extends?: string; prompt: string }): string {
  let text = `---\n---\n${o.prompt.trim()}\n`;
  text = setFrontmatterKey(text, "title", o.title);
  text = setFrontmatterKey(text, "description", o.description);
  if (o.extends) text = setFrontmatterKey(text, "extends", o.extends);
  else if (o.base) text = setFrontmatterKey(text, "base", o.base);
  return text;
}

// ------------------------------------------------------------------ template graph

export interface GraphNode {
  key: string;
  label: string;
  sub: string;
  x: number;
  y: number;
  orchestrator?: boolean;
  /** Named by a problem of the template. */
  flagged?: boolean;
}

export interface GraphEdge {
  from: string;
  to: string;
  type: RelKind;
  on?: string;
  note?: string;
  /** SVG path. */
  d: string;
  /** Where the label goes. */
  lx: number;
  ly: number;
}

export const NODE_H = 48;
const ORCH_W = 150;

/**
 * Lay out a team: the orchestrator on top, members in a row. Relations to the
 * right arc above the row (work moves on), to the left below it (work comes
 * back, questions), reports go straight up. Nodes narrow as the team grows so
 * that five members fit a regular page.
 */
export function layoutTeam(
  members: { key: string; label: string; sub: string }[],
  relations: Relation[],
  problems: string[] = [],
): { width: number; height: number; nodeW: number; nodes: GraphNode[]; edges: GraphEdge[] } {
  const n = members.length;
  const nodeW = n <= 3 ? 164 : n === 4 ? 160 : 140;
  const step = nodeW + (n <= 4 ? 28 : 12);
  const width = Math.max(420, n * step + 48);
  const orchY = 12 + NODE_H / 2;
  const rowY = orchY + NODE_H + 86;
  const x0 = width / 2 - ((n - 1) * step) / 2;
  const flagged = (key: string) => problems.some((p) => p.includes(`\`${key}\``));
  const nodes: GraphNode[] = [
    { key: "orchestrator", label: "Оркестратор", sub: "голос команды", x: width / 2, y: orchY, orchestrator: true },
    ...members.map((m, i) => ({ ...m, x: x0 + i * step, y: rowY, flagged: flagged(m.key) })),
  ];
  const at = new Map(nodes.map((nd) => [nd.key, nd]));
  const index = new Map(members.map((m, i) => [m.key, i]));
  const edges: GraphEdge[] = [];
  let below = 0;
  for (const r of relations) {
    for (const to of r.to) {
      const a = at.get(r.from);
      const b = at.get(to);
      if (!a || !b || a === b) continue;
      const base = { from: r.from, to, type: r.type, on: r.on, note: r.note };
      if (a.orchestrator || b.orchestrator) {
        const [m, o] = a.orchestrator ? [b, a] : [a, b];
        const mx = m.x + (m.x < o.x ? 12 : m.x > o.x ? -12 : 0);
        const ox = o.x + Math.max(-ORCH_W / 2 + 14, Math.min(ORCH_W / 2 - 14, (m.x - o.x) / 3));
        const [my, oy] = [m.y - NODE_H / 2, o.y + NODE_H / 2 + 1];
        const d = a.orchestrator ? `M ${ox} ${oy} L ${mx} ${my}` : `M ${mx} ${my} L ${ox} ${oy}`;
        edges.push({ ...base, d, lx: (mx + ox) / 2, ly: (my + oy) / 2 });
        continue;
      }
      const span = Math.abs((index.get(to) ?? 0) - (index.get(r.from) ?? 0));
      const forward = b.x > a.x;
      const shift = forward ? 10 : -10;
      const [x1, x2] = [a.x + shift, b.x - shift];
      const y = forward ? a.y - NODE_H / 2 : a.y + NODE_H / 2;
      const lift = 26 + 28 * span;
      const qy = forward ? y - lift : y + lift;
      if (!forward) below = Math.max(below, lift / 2);
      edges.push({ ...base, d: `M ${x1} ${y} Q ${(x1 + x2) / 2} ${qy} ${x2} ${y}`, lx: (x1 + x2) / 2, ly: (y + qy) / 2 });
    }
  }
  return { width, height: rowY + NODE_H / 2 + below + 22, nodeW, nodes, edges };
}

// ------------------------------------------------------------------ what a template needs

/** Templates a task can use now: its stage decides (refinement before `ready`). */
export function templatesFor(teams: TeamDef[], taskStatus: string): TeamDef[] {
  const early = ["inbox", "draft", "refining"].includes(taskStatus);
  return teams.filter((t) => (early ? t.stage === "refinement" : t.stage === "delivery"));
}

/** An empty template to start from. */
export function newTemplate(title: string, description: string): Record<string, unknown> {
  return {
    title,
    description,
    stage: "delivery",
    workspace: "worktree",
    members: [{ role: "executor" }, { role: "reviewer" }],
    relations: [
      { from: "executor", to: ["reviewer"], type: "handoff", on: "review", note: "the work is ready for review" },
      { from: "reviewer", to: ["executor"], type: "returns", on: "changes_requested" },
      { from: "reviewer", to: ["orchestrator"], type: "reports", note: "the verdict" },
    ],
  };
}

// ------------------------------------------------------------------ a running team

/** A member's name as people read it (`bender` → `Bender`). */
export const capitalized = (name: string) => (name ? name[0].toUpperCase() + name.slice(1) : name);

/** How a running team works: the snapshot it took from its template (or derived). */
export interface TeamSpecView {
  template?: string;
  title?: string;
  stage: Stage;
  workspace: Workspace;
  mail: MailMode;
  members: { key: string; name: string; role: string }[];
  relations: Relation[];
  charter?: string;
}

export type LiveState = "working" | "waiting" | "idle" | "error" | "stopped";

const FLOW = ["inbox", "draft", "refining", "ready", "in_progress", "review", "approved", "done"];

/** Whether the task has got to `status` on its way forward (changes_requested is work again). */
export function reached(current: string, status: string): boolean {
  if (status === "changes_requested") return current === "changes_requested";
  const pos = (s: string) => FLOW.indexOf(s === "changes_requested" ? "in_progress" : s);
  return pos(current) >= pos(status);
}

/**
 * Who in a running team works, who waits and for whom: a member not working
 * waits for a handoff whose status the task has not reached yet. `pending`
 * holds those handoffs as `from->to` member keys, `done` the relations whose
 * status the task has reached (the handoff has happened).
 */
export function liveTeam(
  spec: TeamSpecView,
  members: { name: string; activity?: string; state?: string; status?: string }[],
  taskStatus: string,
  teamActive: boolean,
  display: (name: string) => string = capitalized,
): { live: Record<string, { state: LiveState; note?: string }>; pending: string[]; done: string[] } {
  const nameOf = (key: string) => display(spec.members.find((m) => m.key === key)?.name ?? key);
  const live: Record<string, { state: LiveState; note?: string }> = {};
  const pending: string[] = [];
  for (const sm of spec.members) {
    const m = members.find((x) => x.name === sm.name);
    let state: LiveState =
      !teamActive || !m || m.state === "stopped" ? "stopped" : m.activity === "error" || m.state === "lost" ? "error" : m.activity === "working" ? "working" : "idle";
    let note = m?.status?.trim() || undefined;
    if (state === "idle") {
      const waits = spec.relations.filter((r) => r.type === "handoff" && r.on && r.to.includes(sm.key) && !reached(taskStatus, r.on));
      if (waits.length) {
        state = "waiting";
        note = `ждёт ${[...new Set(waits.map((r) => nameOf(r.from)))].join(", ")}`;
        pending.push(...waits.map((r) => `${r.from}->${sm.key}`));
      }
    }
    live[sm.key] = { state, note };
  }
  const done = spec.relations.filter((r) => r.on && reached(taskStatus, r.on)).flatMap((r) => r.to.map((to) => `${r.from}->${to}`));
  return { live, pending, done };
}
