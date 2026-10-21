import type { Status } from "../../shared/api/types.ts";
import type { Spend, SpendItem } from "../usage/model.ts";

export interface Meta {
  prefix: string;
  project: string;
  created: string;
  counts: Partial<Record<Status, number>>;
  user: string;
  roles: string[];
  roleModels: Record<string, { model?: string; thinking?: string }>;
  tailnet?: string;
}

export type ProjectRole = "viewer" | "member" | "admin" | "owner";

export const PROJECT_ROLE_NAME: Record<ProjectRole, string> = { owner: "владелец", admin: "админ", member: "участник", viewer: "только чтение" };

/** What each role may do in a project, for people who hand them out. */
export const PROJECT_ROLE_HINT: Record<ProjectRole, string> = {
  viewer: "видит задачи и документацию, ничего не меняет",
  member: "ставит задачи, отвечает агентам, принимает работу",
  admin: "плюс люди, приглашения, автоматизации и настройки проекта",
  owner: "как админ, и получает уведомления для владельцев",
};

export type Autonomy = "autonomous" | "assisted" | "manual";

export const AUTONOMY: { id: Autonomy; name: string; hint: string }[] = [
  { id: "autonomous", name: "Автономно", hint: "оркестратор ведёт задачи и сам закрывает принятые" },
  { id: "assisted", name: "С подтверждением", hint: "оркестратор ведёт задачи, закрывают их люди" },
  { id: "manual", name: "Вручную", hint: "оркестратор сервера не работает; задачи ведут люди и их агенты" },
];

/** A user of the server. */
export interface Person {
  id: number;
  login: string;
  name: string;
  email?: string;
  isAdmin: boolean;
  disabled: boolean;
  created: string;
  /** Where the person's photo is served; none without a photo. */
  avatar?: string;
}

export interface Membership {
  user: Person;
  role: ProjectRole;
}

/** A project as `/api/projects` lists it, with the caller's role. */
export interface ProjectInfo {
  icon?: string;
  slug: string;
  name: string;
  trackerDir: string;
  repo?: string;
  space: string;
  autonomy: Autonomy;
  /** How finished work gets integrated unless a task says otherwise. */
  integration: string;
  created: string;
  role: ProjectRole;
}

export function personName(p: Pick<Person, "login" | "name">): string {
  return p.name || p.login;
}

/** Two letters for an avatar. */
export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  const two = parts.length > 1 ? parts[0][0] + parts[1][0] : name.slice(0, 2);
  return two.toUpperCase();
}

/** People who can be responsible for a task: active members, the current one kept even if they left. */
export function responsibleChoices(members: Membership[], current?: string): { login: string; label: string }[] {
  const out = members
    .filter((m) => !m.user.disabled && m.role !== "viewer")
    .map((m) => ({ login: m.user.login, label: m.user.name ? `${m.user.name} (@${m.user.login})` : `@${m.user.login}` }));
  if (current && !out.some((o) => o.login === current)) out.unshift({ login: current, label: `@${current}` });
  return out;
}

/** One line of the server's preflight (`genie doctor`). */
export interface DoctorCheck {
  area: string;
  level: "ok" | "warn" | "fail";
  text: string;
  hint?: string;
}

export const DOCTOR_AREA: Record<string, string> = {
  data: "Данные",
  web: "Веб",
  people: "Люди",
  projects: "Проекты",
  agents: "Агенты",
  pi: "pi",
  models: "Модели",
  sandbox: "Песочница",
  git: "git",
  channels: "Каналы",
  network: "Сеть",
  vault: "База знаний",
};

/** How the vault syncs with its git remote. */
export interface VaultSync {
  remote?: string;
  every: number;
  last?: {
    remote: string;
    branch: string;
    at?: string;
    ok: boolean;
    error?: string;
    pulled: number;
    pushed: number;
    both: string[];
    /** Edits that overlapped, and where the other version went. */
    conflicts: string[];
  } | null;
}

/** The preflight in one line: what is broken first. */
export function doctorSummary(checks: DoctorCheck[]): { level: DoctorCheck["level"]; text: string } {
  const fail = checks.filter((c) => c.level === "fail").length;
  const warn = checks.filter((c) => c.level === "warn").length;
  if (fail) return { level: "fail", text: `Нужно исправить: ${fail}${warn ? `, предупреждений: ${warn}` : ""}` };
  if (warn) return { level: "warn", text: `Готов к работе, предупреждений: ${warn}` };
  return { level: "ok", text: "Всё готово" };
}

/** What happened in a project over a period (`genie stats`). */
export interface ProjectStats {
  factory?: import("../../shared/api/generated/FactoryAnalytics.ts").FactoryAnalytics;
  project: string;
  name: string;
  created: number;
  createdByPeople: number;
  done: number;
  cancelled: number;
  open: number;
  cycleHoursMedian: number | null;
  cycleHoursP90: number | null;
  decisions: number;
  answerHoursMedian: number | null;
  returns: number;
  commentsByPeople: number;
  commentsByAgents: number;
  mcpCalls: number;
  runs: number;
  runsFailed: number;
  jobs: number;
  jobsFailed: number;
  proposals: number;
  proposalsApproved: number;
  proposalsRejected: number;
  people: string[];
  /** The period day by day (UTC dates, oldest first), for charts. */
  daily: DayStats[];
  /** What the agents' models cost in the period (an older server sends none). */
  usage?: ProjectUsage;
}

/** What the agents' models spent in a project over the period. */
export interface ProjectUsage {
  spend: Spend;
  /** Every epic, then `""` (tasks outside epics) and `"-"` (work on no task). */
  epics: SpendItem[];
  /** The most expensive tasks. */
  tasks: SpendItem[];
  /** The most expensive chats. */
  chats: SpendItem[];
}

/** One day of a project's period. */
export interface DayStats {
  day: string;
  created: number;
  done: number;
  runs: number;
  runsFailed: number;
  /** Dollars (models with a price) and tokens (all models). */
  cost?: number;
  tokens?: number;
  /** Dollars by model, models with a price only. */
  costByModel?: Record<string, number>;
}

/** One day's cost of several projects, by model. */
export interface CostDay {
  day: string;
  cost: number;
  tokens: number;
  byModel: Record<string, number>;
}

/** The cost of several projects added up, day by day, oldest first. */
export function costDays(projects: Pick<ProjectStats, "daily">[]): CostDay[] {
  const by = new Map<string, CostDay>();
  for (const p of projects)
    for (const d of p.daily ?? []) {
      const t = by.get(d.day) ?? { day: d.day, cost: 0, tokens: 0, byModel: {} };
      t.cost += d.cost ?? 0;
      t.tokens += d.tokens ?? 0;
      for (const [m, c] of Object.entries(d.costByModel ?? {})) t.byModel[m] = (t.byModel[m] ?? 0) + c;
      by.set(d.day, t);
    }
  return [...by.values()].sort((a, b) => a.day.localeCompare(b.day));
}

/** The days of several projects added up, day by day. */
export function sumDays(projects: Pick<ProjectStats, "daily">[]): DayStats[] {
  const by = new Map<string, DayStats>();
  for (const p of projects)
    for (const d of p.daily ?? []) {
      const t = by.get(d.day) ?? { day: d.day, created: 0, done: 0, runs: 0, runsFailed: 0 };
      t.created += d.created;
      t.done += d.done;
      t.runs += d.runs;
      t.runsFailed += d.runsFailed;
      by.set(d.day, t);
    }
  return [...by.values()].sort((a, b) => a.day.localeCompare(b.day));
}

/** The counts of several projects added up (medians do not add up, so they are left out). */
export function statsTotals(projects: ProjectStats[]) {
  const sum = (f: (p: ProjectStats) => number) => projects.reduce((n, p) => n + f(p), 0);
  return {
    created: sum((p) => p.created),
    createdByPeople: sum((p) => p.createdByPeople),
    done: sum((p) => p.done),
    open: sum((p) => p.open),
    decisions: sum((p) => p.decisions),
    returns: sum((p) => p.returns),
    runs: sum((p) => p.runs),
    runsFailed: sum((p) => p.runsFailed),
    openProjects: projects.filter((p) => p.open > 0).length,
  };
}

/** Hours in words: «40 мин», «5,5 ч», «3,2 дн». */
export function hoursText(h: number | null | undefined): string {
  if (h === null || h === undefined) return "—";
  if (h < 1) return `${Math.max(1, Math.round(h * 60))} мин`;
  if (h < 48) return `${h.toFixed(1).replace(".", ",")} ч`;
  return `${(h / 24).toFixed(1).replace(".", ",")} дн`;
}

/** Initials of words, ignoring punctuation between parts of a project name. */
export function projectMonogram(name: string): string {
  const words = name.match(/[\p{L}\p{N}]+/gu) ?? [];
  const [first, second] = words;
  if (!first) return "?";
  const letters = second ? [Array.from(first)[0], Array.from(second)[0]] : Array.from(first).slice(0, 2);
  return letters.join("").toLocaleUpperCase("ru-RU");
}
