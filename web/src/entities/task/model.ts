// UI-side view of the tracker model. Types and constants come straight from the
// server code, so the API contract is checked by the compiler on both sides.

import type { Status, Task, TaskSummary } from "../../shared/api/types.ts";

export type { Status, Task, TaskSummary };

export const STATUS_NAME: Record<Status, string> = {
  inbox: "Входящие",
  draft: "Черновик",
  refining: "Уточнение",
  ready: "Готово к работе",
  in_progress: "В работе",
  review: "На ревью",
  changes_requested: "Доработка",
  approved: "Одобрено",
  needs_owner: "Нужно решение",
  done: "Готово",
  cancelled: "Отменено",
};

export const STATUS_ORDER: Status[] = ["needs_owner", "review", "changes_requested", "in_progress", "approved", "ready", "refining", "draft", "inbox", "done", "cancelled"];

export const ACTIVE: Status[] = ["draft", "refining", "ready", "in_progress", "review", "changes_requested", "approved", "needs_owner"];

/** Open = not finished: what the task list shows unless asked otherwise. */
export const OPEN: Status[] = ["inbox", ...ACTIVE];

export type PresetId = "open" | "decisions" | "inbox" | "working" | "prep" | "done";

/** Quick status sets above the task list. */
export const PRESETS: { id: PresetId; name: string; statuses: Status[] }[] = [
  { id: "open", name: "Открытые", statuses: OPEN },
  { id: "decisions", name: "Нужно решение", statuses: ["needs_owner"] },
  { id: "inbox", name: "Входящие", statuses: ["inbox"] },
  { id: "working", name: "В работе", statuses: ["in_progress", "changes_requested", "review", "approved"] },
  { id: "prep", name: "Подготовка", statuses: ["draft", "refining", "ready"] },
  { id: "done", name: "Завершённые", statuses: ["done", "cancelled"] },
];

/** How far a task is through its acceptance criteria. */
export type Readiness = "none" | "zero" | "partial" | "full";
export type TimeRange = "today" | "7d" | "30d" | "stale7" | "stale30";
export type TaskSort = "priority" | "updated" | "created" | "id";

export const READINESS_NAME: Record<Readiness, string> = { none: "без критериев", zero: "не начата", partial: "в процессе", full: "все выполнены" };
export const TIME_NAME: Record<TimeRange, string> = { today: "сегодня", "7d": "за 7 дней", "30d": "за 30 дней", stale7: "больше 7 дней назад", stale30: "больше 30 дней назад" };
export const SORT_NAME: Record<TaskSort, string> = { priority: "по приоритету", updated: "сначала обновлённые", created: "сначала новые", id: "по номеру" };

/** The task list's search and filters; they live in the page's address, so a link keeps them. */
export interface TaskFilter {
  q: string;
  statuses: Status[];
  ready?: Readiness;
  time?: TimeRange;
  /** Which date `time` looks at. */
  by: "updated" | "created";
  priorities: number[];
  /** An epic's id, or "none" for tasks outside epics. */
  epic?: string;
  /** "me", "none" (nobody responsible) or a person's login. */
  who?: string;
  sort: TaskSort;
  grouped: boolean;
}

const STATUSES = Object.keys(STATUS_NAME) as Status[];
const oneOf = <T extends string>(v: string | null, all: readonly T[]): T | undefined => (v && (all as readonly string[]).includes(v) ? (v as T) : undefined);

export function parseFilter(sp: URLSearchParams): TaskFilter {
  const statuses = (sp.get("status") ?? "").split(",").filter((s): s is Status => STATUSES.includes(s as Status));
  return {
    q: sp.get("q") ?? "",
    statuses: statuses.length ? statuses : OPEN,
    ready: oneOf(sp.get("ready"), ["none", "zero", "partial", "full"] as const),
    time: oneOf(sp.get("time"), ["today", "7d", "30d", "stale7", "stale30"] as const),
    by: sp.get("by") === "created" ? "created" : "updated",
    priorities: (sp.get("prio") ?? "").split(",").filter((p) => /^[0-4]$/.test(p)).map(Number),
    epic: sp.get("epic") || undefined,
    who: sp.get("who") || undefined,
    sort: oneOf(sp.get("sort"), ["priority", "updated", "created", "id"] as const) ?? "priority",
    grouped: sp.get("group") !== "none",
  };
}

/** Writes a filter into the address, keeping its other parameters (the open task); defaults stay out. */
export function writeFilter(f: TaskFilter, base: URLSearchParams): URLSearchParams {
  const sp = new URLSearchParams(base);
  const put = (k: string, v: string | undefined) => (v ? sp.set(k, v) : sp.delete(k));
  put("q", f.q.trim() ? f.q : undefined);
  put("status", sameSet(f.statuses, OPEN) ? undefined : f.statuses.join(","));
  put("ready", f.ready);
  put("time", f.time);
  put("by", f.time && f.by === "created" ? "created" : undefined);
  put("prio", f.priorities.length ? [...f.priorities].sort().join(",") : undefined);
  put("epic", f.epic);
  put("who", f.who);
  put("sort", f.sort === "priority" ? undefined : f.sort);
  put("group", f.grouped ? undefined : "none");
  return sp;
}

export function sameSet<T>(a: readonly T[], b: readonly T[]): boolean {
  return a.length === b.length && a.every((x) => b.includes(x));
}

/** Whether the filter narrows anything beyond the default open tasks. */
export function isFiltered(f: TaskFilter): boolean {
  return !!f.q.trim() || !sameSet(f.statuses, OPEN) || !!f.ready || !!f.time || f.priorities.length > 0 || !!f.epic || !!f.who;
}

export function readinessOf(t: Pick<TaskSummary, "acceptanceDone" | "acceptanceTotal">): Readiness {
  if (!t.acceptanceTotal) return "none";
  if (!t.acceptanceDone) return "zero";
  return t.acceptanceDone >= t.acceptanceTotal ? "full" : "partial";
}

const DAY = 86_400_000;

function inRange(iso: string, range: TimeRange, now: number): boolean {
  const at = Date.parse(iso);
  if (range === "today") {
    const midnight = new Date(now);
    midnight.setHours(0, 0, 0, 0);
    return at >= midnight.getTime();
  }
  const days = { "7d": 7, "30d": 30, stale7: 7, stale30: 30 }[range];
  return range.startsWith("stale") ? now - at > days * DAY : now - at <= days * DAY;
}

/**
 * Whether a task passes every filter but the status one: the status presets count with it.
 * Epics show up only while someone has to act on them, unless the list is narrowed to one epic.
 */
export function matchesBesidesStatus(t: TaskSummary, f: TaskFilter, login?: string, now = Date.now()): boolean {
  if (f.epic && f.epic !== "none" ? t.parent !== f.epic : !inTaskViews(t)) return false;
  if (f.epic === "none" && t.parent) return false;
  const q = f.q.trim().toLowerCase();
  if (q && ![t.id, t.title, t.needsOwner?.question ?? "", ...t.labels].some((s) => s.toLowerCase().includes(q))) return false;
  if (f.ready && readinessOf(t) !== f.ready) return false;
  if (f.time && !inRange(f.by === "created" ? t.created : t.updated, f.time, now)) return false;
  if (f.priorities.length && !f.priorities.includes(t.priority)) return false;
  if (f.who === "me" ? !login || t.assignee !== login : f.who === "none" ? !!t.assignee : f.who ? t.assignee !== f.who : false) return false;
  return true;
}

export function matchesFilter(t: TaskSummary, f: TaskFilter, login?: string, now = Date.now()): boolean {
  return f.statuses.includes(t.status) && matchesBesidesStatus(t, f, login, now);
}

const seqOf = (id: string) => Number(/(\d+)$/.exec(id)?.[1] ?? 0);

/** Orders a list; the server's own order is priority, then creation. */
export function sortTasks(tasks: TaskSummary[], sort: TaskSort): TaskSummary[] {
  const by: Record<TaskSort, (a: TaskSummary, b: TaskSummary) => number> = {
    priority: (a, b) => a.priority - b.priority || Date.parse(a.created) - Date.parse(b.created),
    updated: (a, b) => Date.parse(b.updated) - Date.parse(a.updated),
    created: (a, b) => Date.parse(b.created) - Date.parse(a.created),
    id: (a, b) => seqOf(a.id) - seqOf(b.id),
  };
  return [...tasks].sort(by[sort]);
}

/** Old addresses (`/inbox`, `/decisions`, …) as the task list's filters, so saved links keep working. */
export const LEGACY_VIEWS: Record<string, { statuses?: Status[]; who?: string }> = {
  mine: { who: "me" },
  inbox: { statuses: ["inbox"] },
  decisions: { statuses: ["needs_owner"] },
  active: {},
  prep: { statuses: ["draft", "refining", "ready"] },
  done: { statuses: ["done", "cancelled"] },
};

export interface Column {
  id: string;
  name: string;
  statuses: Status[];
  /** Status a card gets when dropped into the column. */
  target: Status;
}

export const COLUMNS: Column[] = [
  { id: "inbox", name: "Входящие", statuses: ["inbox"], target: "inbox" },
  { id: "prep", name: "Подготовка", statuses: ["draft", "refining"], target: "refining" },
  { id: "ready", name: "Готово к работе", statuses: ["ready"], target: "ready" },
  { id: "in_progress", name: "В работе", statuses: ["in_progress", "changes_requested"], target: "in_progress" },
  { id: "review", name: "На ревью", statuses: ["review"], target: "review" },
  { id: "approved", name: "Одобрено", statuses: ["approved"], target: "approved" },
  { id: "needs_owner", name: "Нужно решение", statuses: ["needs_owner"], target: "needs_owner" },
  { id: "done", name: "Готово", statuses: ["done", "cancelled"], target: "done" },
];

export const PRIORITY_NAME = ["Срочно", "Высокий", "Средний", "Низкий", "Без приоритета"];

export const STAGES = ["Уточнение", "Готово к работе", "В работе", "Ревью", "Одобрено", "Принято"];

/** 0…6: how far a task is through refining → done. */
export function stageOf(status: Status, previous?: Status): number {
  const s = status === "needs_owner" && previous ? previous : status;
  const map: Partial<Record<Status, number>> = { refining: 1, ready: 2, in_progress: 3, changes_requested: 3, review: 4, approved: 5, done: 6 };
  return map[s] ?? 0;
}

/**
 * Epics have their own pages. In task views they only show up while someone has to
 * act on them as a whole: the orchestrator (inbox) or the owner (needs owner).
 */
export function inTaskViews(t: TaskSummary): boolean {
  return t.type !== "epic" || t.status === "inbox" || t.status === "needs_owner";
}

/** Where a task of an epic stands, for the epic's progress bar. */
export type Progress = "closed" | "owner" | "review" | "working" | "ready" | "early";

export const PROGRESS: { id: Progress; name: string; color: string }[] = [
  { id: "closed", name: "Закрыто", color: "#7c84f0" },
  { id: "owner", name: "Ждёт вашего решения", color: "#f0a04b" },
  { id: "review", name: "На ревью", color: "#4cb782" },
  { id: "working", name: "В работе", color: "#f2c94c" },
  { id: "ready", name: "Готово к работе", color: "#4a4d55" },
  { id: "early", name: "Черновик", color: "#2e3138" },
];

export function progressOf(status: Status): Progress {
  if (status === "done" || status === "cancelled") return "closed";
  if (status === "needs_owner") return "owner";
  if (status === "review" || status === "approved") return "review";
  if (status === "in_progress" || status === "changes_requested") return "working";
  if (status === "ready") return "ready";
  return "early";
}

const FIELD_RU: Record<string, string> = {
  title: "название",
  type: "тип",
  description: "описание",
  priority: "приоритет",
  "merge strategy": "интеграцию",
  plan: "план",
  notes: "заметки",
  labels: "метки",
  assignees: "исполнителей",
  assignee: "ответственного",
  acceptance: "критерии",
  dependencies: "зависимости",
};

/** A status change's note as the tracker stores it in a comment: `[in_progress → review] text`. */
export function statusNote(text: string): { from: string; to: string; note: string } | undefined {
  const m = /^\[([a-z_]+) → ([a-z_]+)\]\s*([\s\S]*)$/.exec(text.trim());
  return m ? { from: m[1], to: m[2], note: m[3] } : undefined;
}

/** History entry in words for the UI; the tracker records it in English for the agents. */
export function historyText(h: Task["history"][number], epic = false): string {
  const note = h.note ? ` — ${h.note.replace(/^work started on (.+)$/, "команда взяла $1").replace(/^split into (.+)$/, "разбита на $1")}` : "";
  if (h.event === "status" && h.from && h.to) return `${STATUS_NAME[h.from as Status] ?? h.from} → ${STATUS_NAME[h.to as Status] ?? h.to}${note}`;
  if (h.event === "created") return epic ? "создал эпик" : "создал задачу";
  const rules: [RegExp, (...m: string[]) => string][] = [
    [/^child (\S+) added$/, (id) => `добавил задачу ${id}`],
    [/^child (\S+) moved in$/, (id) => `перенёс сюда ${id}`],
    [/^child (\S+) moved out$/, (id) => `убрал ${id} из эпика`],
    [/^artifact #\d+ (.+) \((\S+)\) added$/, (name, kind) => `добавил артефакт ${name} (${kind})`],
    [/^acceptance #(\d+) checked$/, (n) => `отметил критерий #${n}`],
    [/^acceptance #(\d+) unchecked$/, (n) => `снял отметку с критерия #${n}`],
    [/^(\S+) split into (.+)$/, (id, ids) => `разбил ${id} на ${ids}`],
    [/^split into (.+)$/, (ids) => `разбил на ${ids}`],
    [/^blocked: (.+)$/, (r) => `заблокировал: ${r}`],
    [/^unblocked$/, () => "снял блокировку"],
    [/^assigned to team (.+)$/, (t) => `назначил команду ${t}`],
    [/^team released$/, () => "освободил задачу от команды"],
    [
      /^updated (.+)$/,
      (fields) =>
        `изменил ${fields
          .split(", ")
          .map((f) => (f.startsWith("epic → ") ? `эпик на ${f.slice(7)}` : f === "epic removed" ? "эпик (убран)" : (FIELD_RU[f] ?? f)))
          .join(", ")}`,
    ],
  ];
  for (const [re, fn] of rules) {
    const m = h.event.match(re);
    if (m) return fn(...m.slice(1)) + note;
  }
  return h.event + note;
}
