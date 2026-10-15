// Automation rules as people read them: the trigger and each step in words, not
// in the rule's JSON. Pure: the node tests load this file directly.

// A deep import on purpose: this file stays pure, and the node tests load it directly.
import { type Status, STATUS_NAME } from "../task/model.ts";

type Spec = { on?: Record<string, unknown>; steps?: Record<string, unknown>[] };

const EVENT: Record<string, string> = {
  "task.*": "любое событие задачи",
  "task.created": "задача создана",
  "task.updated": "задача изменена",
  "task.status_changed": "статус задачи изменился",
  "task.commented": "новый комментарий",
  "task.criterion_checked": "отмечен критерий приёмки",
  "task.artifact_added": "добавлен артефакт",
  "task.blocked": "задача заблокирована",
  "task.unblocked": "с задачи снят блок",
  "task.team_assigned": "задаче собрали команду",
  "mail.sent": "письмо в команде",
  "mcp.called": "вызов MCP",
};
const TYPE: Record<string, string> = { task: "задача", bug: "баг", spike: "исследование", epic: "эпик" };
const TYPES: Record<string, string> = { task: "задач", bug: "багов", spike: "исследований", epic: "эпиков" };
const ACTOR: Record<string, string> = { human: "человек", agent: "агент", orchestrator: "оркестратор" };
const CREATED_BY: Record<string, string> = { human: "человеком", agent: "агентом", orchestrator: "оркестратором" };
const TO: Record<string, string> = {
  "task.author": "автору задачи",
  "event.actor": "автору события",
  "project.admins": "админам проекта",
  "project.members": "участникам проекта",
  owner: "владельцу",
};
const DAYS = ["воскресеньям", "понедельникам", "вторникам", "средам", "четвергам", "пятницам", "субботам"];
const STEP_FIELDS = new Set(["id", "if", "retry", "timeout", "onError"]);

const status = (s: unknown) => `«${STATUS_NAME[String(s) as Status] ?? String(s)}»`;
/** `a`, `a или b`, `a, b или c`. */
const or = (xs: string[]) => (xs.length > 1 ? `${xs.slice(0, -1).join(", ")} или ${xs.at(-1)}` : (xs[0] ?? ""));
const and = (xs: string[]) => (xs.length > 1 ? `${xs.slice(0, -1).join(", ")} и ${xs.at(-1)}` : (xs[0] ?? ""));
const many = (v: unknown, f: (x: string) => string = String) => (Array.isArray(v) ? v : [v]).map((x) => f(String(x)));

/** One condition of a trigger's `where`: `{ to: "done" }` → `статус → «Готово»`. */
export function condition(key: string, v: unknown): string {
  const [op, arg] = Array.isArray(v) ? ["in", v] : v && typeof v === "object" ? (Object.entries(v as Record<string, unknown>)[0] ?? ["", ""]) : ["=", v];
  const is = op === "=" || op === "in";
  if (key === "to" && is) return `статус → ${or(many(arg, status))}`;
  if (key === "to" && op === "not") return `статус → не ${or(many(arg, status))}`;
  if (key === "from" && is) return `из ${or(many(arg, status))}`;
  if ((key === "status" || key === "task.status") && is) return `в статусе ${or(many(arg, status))}`;
  if (key === "task.type" && is) return `тип: ${or(many(arg, (x) => TYPE[x] ?? x))}`;
  if (key === "task.type" && op === "not") return `кроме ${or(many(arg, (x) => TYPES[x] ?? x))}`;
  if (key === "task.labels" && op === "contains") return `с меткой ${or(many(arg))}`;
  if (key === "task.labels" && op === "not_contains") return `без метки ${or(many(arg))}`;
  if (key === "actor.role" && is) return `автор — ${or(many(arg, (x) => ACTOR[x] ?? x))}`;
  if (op === "exists") return arg ? `есть ${key}` : `нет ${key}`;
  const ops: Record<string, string> = { "=": "=", in: "∈", not: "≠", contains: "содержит", not_contains: "без", gt: ">", lt: "<", prefix: "начинается с" };
  return `${key} ${ops[op] ?? op} ${many(arg).join(", ")}`;
}

/** A five-field cron as words where it is a common one: `0 17 * * 5` → `по пятницам в 17:00`. */
export function cronText(expr: string): string {
  const f = expr.trim().split(/\s+/);
  if (f.length !== 5) return expr;
  const [m, h, dom, mon, dow] = f;
  const any = (...xs: string[]) => xs.every((x) => x === "*");
  if (/^\d+$/.test(m) && /^\d+$/.test(h) && any(dom, mon)) {
    const at = `${h.padStart(2, "0")}:${m.padStart(2, "0")}`;
    if (dow === "*") return `каждый день в ${at}`;
    if (dow === "1-5") return `по будням в ${at}`;
    if (/^[0-7](,[0-7])*$/.test(dow)) return `по ${and(dow.split(",").map((d) => DAYS[Number(d) % 7]))} в ${at}`;
  }
  if (/^\*\/\d+$/.test(m) && any(h, dom, mon, dow)) return `каждые ${m.slice(2)} мин`;
  if (/^\d+$/.test(m) && any(h, dom, mon, dow)) return `каждый час в :${m.padStart(2, "0")}`;
  return expr;
}

/** What starts a rule, in words. */
export function describeTrigger(spec: unknown, enabled = true, dryRun = false): string {
  const on = (spec as Spec).on ?? {};
  let text: string;
  if (typeof on.event === "string") {
    const where = { ...((on.where && typeof on.where === "object" ? on.where : {}) as Record<string, unknown>) };
    let head = EVENT[on.event] ?? on.event;
    if (on.event === "task.status_changed" && "to" in where) {
      head = condition("to", where.to);
      delete where.to;
    }
    if (on.event === "task.created" && typeof where["actor.role"] === "string" && CREATED_BY[where["actor.role"]]) {
      head = `задача создана ${CREATED_BY[where["actor.role"]]}`;
      delete where["actor.role"];
    }
    const rest = Object.entries(where).map(([k, v]) => condition(k, v));
    text = `Событие: ${[head, ...rest].join(", ")}`;
  } else if (typeof on.schedule === "string") {
    text = `Расписание: ${cronText(on.schedule)}${typeof on.tz === "string" ? `, ${on.tz}` : ""}`;
  } else if (on.webhook) text = "Webhook: по вызову извне";
  else text = "Вручную";
  if (!enabled) text += " · выключено";
  if (dryRun) text += " · пробный режим";
  return text;
}

/** `45m` → `45 мин`, `24h` → `24 ч`, `2d` → `2 дн`. */
export function duration(v: unknown): string {
  const m = /^(\d+)\s*([smhd])$/.exec(String(v ?? "").trim());
  if (!m) return String(v ?? "");
  return `${m[1]} ${{ s: "с", m: "мин", h: "ч", d: "дн" }[m[2] as "s" | "m" | "h" | "d"]}`;
}

/** `{{ event.task.id }}` → `‹№ задачи›`: placeholders read as what they stand for. */
export function untemplate(text: string): string {
  const NAMES: Record<string, string> = { "event.task.id": "№ задачи", "event.task.title": "название задачи", "event.actor": "автор события", "event.task.author": "автор задачи" };
  return text.replace(/\{\{\s*([^}|]+?)\s*(\|[^}]*)?\}\}/g, (_, path: string) => {
    const out = /^steps\.([^.]+)\.output\.(.+)$/.exec(path);
    return `‹${NAMES[path] ?? (out ? `${out[2]} из шага ${out[1]}` : path.replace(/^event\./, ""))}›`;
  });
}

const short = (s: string, n: number) => (s.length > n ? `${s.slice(0, n - 1)}…` : s);
const recipients = (v: unknown) => and(many(v, (x) => TO[x] ?? x));

/**
 * A step in words: its kind (as the rule names it), what it does and a note.
 * `hint` holds what is too long to show (an agent's whole goal).
 */
export function describeStep(
  step: Record<string, unknown>,
  names: { role?: (id: string) => string | undefined; template?: (id: string) => string | undefined } = {},
): { kind: string; title: string; note?: string; hint?: string } {
  const kind = Object.keys(step).find((k) => !STEP_FIELDS.has(k)) ?? "?";
  const body = (step[kind] && typeof step[kind] === "object" ? step[kind] : {}) as Record<string, unknown>;
  const str = (k: string) => (typeof body[k] === "string" && body[k] ? untemplate(body[k] as string) : undefined);
  const notes = (...xs: (string | false | undefined)[]) => xs.filter(Boolean).join(" · ") || undefined;
  const timeout = step.timeout ? `до ${duration(step.timeout)}` : undefined;
  switch (kind) {
    case "agent": {
      const role = String(body.role ?? "");
      return {
        kind,
        title: names.role?.(role) ?? (role || "Агент"),
        note: notes("задание агенту", body.workspace === "read-only" && "только чтение", body.workspace === "worktree" && "свой worktree", timeout),
        hint: str("goal"),
      };
    }
    case "team": {
      const t = String(body.template ?? "");
      return { kind, title: `Команда «${names.template?.(t) ?? t}»`, note: notes("собрать команду для задачи", timeout) };
    }
    case "notify":
      return { kind, title: `Уведомление ${recipients(body.to)}`, note: str("title") && short(str("title")!, 80) };
    case "ask":
      return {
        kind,
        title: `Вопросы ${recipients(body.to)}`,
        note: notes(typeof body.from === "string" && `от ${body.from}`, body.remindAfter !== undefined && `напомнить через ${duration(body.remindAfter)}`, body.timeout !== undefined && `ждать ${duration(body.timeout)}`),
      };
    case "task.status":
      return { kind, title: `Статус → ${status(body.to)}`, note: str("note") && short(str("note")!, 80) };
    case "task.comment":
      return { kind, title: body.kind === "decision" ? "Решение в комментарии к задаче" : "Комментарий к задаче", note: str("text") && short(str("text")!, 80) };
    case "task.create":
      return { kind, title: "Создать задачу", note: str("title") };
    case "task.update":
      return { kind, title: "Изменить задачу" };
    case "task.get":
      return { kind, title: "Прочитать задачу" };
    case "task.ready":
      return { kind, title: `Статус → ${status("ready")}, если задачу ничего не держит`, note: "нет блока, открытых зависимостей, вопросов без ответа и заданий; выполнен DoR" };
    case "wait":
      return { kind, title: `Пауза ${duration(body.for)}` };
    case "wake_orchestrator":
      return { kind, title: "Разбудить оркестратора", note: str("text") ?? str("message") };
    case "changelog.add":
      return { kind, title: "Запись в чейнджлог", note: str("text") && short(str("text")!, 80) };
    case "release":
      return { kind, title: "Выпустить версию", note: str("version") };
    case "http": {
      let host = String(body.url ?? "");
      try {
        host = new URL(host).host;
      } catch {
        // not a plain URL (a placeholder): as written
      }
      return { kind, title: `HTTP-запрос ${String(body.method ?? "POST")}`, note: host || undefined };
    }
    default:
      return { kind, title: kind };
  }
}
