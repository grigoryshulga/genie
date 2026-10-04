import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Link, useParams } from "react-router";
import { Avatar, displayName, ROLE_TITLE_RU } from "@/entities/member";
import { type LiveSession, type Mail, type MailLevel, MessageText, type PeekMessage, type TeamDetail, usePeek, useRestartMember, useSendMail, useSetPaused, useTeam } from "@/entities/team";
import { SpendText, useChatUsage } from "@/entities/usage";
import { useTask } from "@/entities/task";
import { ModelMenu } from "@/features/agent-model";
import { IDEA_TEMPLATE, IdeaPlan } from "@/features/shape-idea";
import { clock, plural, timeAgo, useTick } from "@/shared/lib";
import { Icon, Markdown, useToast } from "@/shared/ui";
import "./agent.css";

type Member = TeamDetail["members"][number];

/** One step of the agent's work: a thought or a tool call. */
type Step =
  | { kind: "think"; key: string; text: string }
  | { kind: "tool"; key: string; name: string; args: string; at?: string; out?: string; error?: boolean; took?: number; running?: boolean; since?: string };

/** One row of the agent's feed: its conversation, then the mail still on the way to it. Steps in a row fold into one. */
type Row =
  | { kind: "mail"; key: string; mail: Mail[]; text: string; at?: string; pending?: boolean }
  | { kind: "say"; key: string; text: string; at?: string }
  | { kind: "steps"; key: string; steps: Step[] }
  | { kind: "note"; key: string; text: string; at?: string };

const INTENT_LABEL: Record<NonNullable<Mail["intent"]>, string> = { question: "вопрос", blocker: "блокер", verdict: "вердикт", done: "готово", fyi: "к сведению" };
const LEVELS: { level: MailLevel; label: string; hint: string }[] = [
  { level: "interrupt", label: "Прервать шаг", hint: "остановит команду и придёт первым" },
  { level: "high", label: "На следующем шаге", hint: "после текущей команды" },
];

/** A tool call's arguments, short: the command for bash, the path for file tools. */
function argsLine(name: string, args: string): string {
  try {
    const a = JSON.parse(args) as Record<string, unknown>;
    const pick = a.command ?? a.path ?? a.file_path ?? a.pattern ?? a.url;
    if (typeof pick === "string") return pick;
  } catch {
    // clipped arguments are not JSON: show them as they are
  }
  return name === "bash" ? args : args.replace(/^\{|\}$/g, "");
}

function toRows(conversation: PeekMessage[], team: TeamDetail, member: string, live?: LiveSession): Row[] {
  const byId = new Map(team.mail.map((m) => [m.id, m]));
  const rows: Row[] = [];
  const calls: Extract<Step, { kind: "tool" }>[] = [];
  const tools = new Map<string, Extract<Step, { kind: "tool" }>>();
  // Thoughts and tool calls with nothing said between them share one row.
  const step = (s: Step) => {
    const last = rows[rows.length - 1];
    if (last?.kind === "steps") last.steps.push(s);
    else rows.push({ kind: "steps", key: `s${s.key}`, steps: [s] });
  };
  conversation.forEach((m, i) => {
    // The conversation is a sliding window: key by time so an opened step stays open as it moves.
    const key = m.at ? `${m.role}@${m.at}` : `${i}`;
    if (m.role === "custom:genie-mail") {
      const mail = (m.mailIds ?? []).map((id) => byId.get(id)).filter((x): x is Mail => !!x);
      rows.push({ kind: "mail", key, mail, text: m.text, at: m.at });
    } else if (m.role === "assistant") {
      (m.parts ?? []).forEach((p, j) => {
        if (p.type === "thinking") step({ kind: "think", key: `${key}.${j}`, text: p.text });
        else if (p.type === "text") rows.push({ kind: "say", key: `${key}.${j}`, text: p.text, at: m.at });
        else {
          const s: Extract<Step, { kind: "tool" }> = { kind: "tool", key: `${key}.${j}`, name: p.name, args: argsLine(p.name, p.args), at: m.at };
          if (p.id) tools.set(p.id, s);
          calls.push(s);
          step(s);
        }
      });
    } else if (m.role === "toolResult") {
      const s = m.tool?.id ? tools.get(m.tool.id) : undefined;
      if (s) {
        s.out = m.text;
        s.error = m.tool?.error;
        // The call's message is stamped when the agent made it, the result when the tool answered.
        if (s.at && m.at) s.took = Math.max(0, Date.parse(m.at) - Date.parse(s.at));
      }
    } else if (m.role === "user") {
      rows.push({ kind: "note", key, text: m.text, at: m.at });
    }
  });
  // The call running now has no result yet.
  if (live?.state === "working" && live.tool) {
    const open = [...calls].reverse().find((s) => s.out === undefined);
    if (open) Object.assign(open, { running: true, since: live.tool.since });
  }
  const pending = team.mail.filter((m) => m.to === member && !m.deliveredAt);
  for (const m of pending) rows.push({ kind: "mail", key: `p${m.id}`, mail: [m], text: m.text, at: m.at, pending: true });
  return rows;
}

/** How long a step took, short: «0,3 с», «14 с», «2 мин». */
function took(ms: number): string {
  if (ms < 1000) return `${(ms / 1000).toFixed(1).replace(".", ",")} с`;
  if (ms < 60_000) return `${Math.round(ms / 1000)} с`;
  return `${Math.round(ms / 60_000)} мин`;
}

function stateOf(m: Member, s: LiveSession | undefined, teamActive: boolean): { cls: string; text: string; sub: string } {
  if (!teamActive || m.state === "stopped") return { cls: "stopped", text: "остановлен", sub: "Команда не работает." };
  if (m.state === "paused") return { cls: "paused", text: "на паузе", sub: "Сессия остановлена, почта ждёт продолжения." };
  if (m.state === "error" || m.activity === "error") return { cls: "error", text: "ошибка", sub: s?.lastError ?? "Перезапуск вернёт его в работу." };
  if (m.activity === "working" || s?.state === "working") {
    const tool = s?.tool;
    return {
      cls: "working",
      text: tool ? `работает · ${tool.name} ${timeAgo(tool.since)}` : "работает",
      sub: "Почту возьмёт на ближайшей границе шага; «Прервать шаг» остановит текущую команду.",
    };
  }
  return { cls: "idle", text: "свободен", sub: "Письмо его разбудит." };
}

const quoteOf = (r: Extract<Step, { kind: "tool" }>) => `${r.name} · ${r.args.length > 48 ? `${r.args.slice(0, 48)}…` : r.args}`;

export function AgentChat() {
  useTick(15_000);
  const { teamId, member: name } = useParams();
  const q = useTeam(teamId);
  const team = q.data;
  const member = team?.members.find((m) => m.name === name);
  const session = team?.sessions?.[name ?? ""];
  const working = member?.activity === "working" || session?.state === "working";
  const peek = usePeek(teamId, name, !!working);
  const send = useSendMail();
  const setPaused = useSetPaused();
  const restart = useRestartMember();
  const toast = useToast();
  const [level, setLevel] = useState<MailLevel>("high");
  const [draft, setDraft] = useState("");
  const [quote, setQuote] = useState<string | undefined>();
  const [info, setInfo] = useState(false);
  const [open, setOpen] = useState<Set<string>>(new Set());
  const feedRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const stick = useRef(true);
  // A paused or stopped agent has no session to look into: keep what was seen last.
  const seen = useRef(new Map<string, PeekMessage[]>());
  const agentKey = `${teamId}/${name}`;
  const fresh = peek.data?.conversation;
  if (fresh?.length) seen.current.set(agentKey, fresh);
  const conversation = fresh?.length ? fresh : (seen.current.get(agentKey) ?? []);
  const live = peek.data?.session ?? session;
  // The idea planner: its kickoff is for the agent, the person sees their own idea instead.
  const isIdea = team?.template === IDEA_TEMPLATE;
  const ideaTask = useTask(isIdea ? team?.task : undefined).data;
  const rows = useMemo(() => {
    const all = team && name ? toRows(conversation, team, name, live ?? undefined) : [];
    return isIdea ? all.filter((r) => !(r.kind === "mail" && r.mail.length > 0 && r.mail.every((m) => m.kind === "kickoff"))) : all;
  }, [conversation, team, name, live, isIdea]);
  const toggle = (key: string) => setOpen((s) => new Set(s.has(key) ? [...s].filter((k) => k !== key) : [...s, key]));

  useEffect(() => {
    setQuote(undefined);
    setOpen(new Set());
    stick.current = true;
  }, [agentKey]);
  useLayoutEffect(() => {
    const el = feedRef.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [rows.length, agentKey]);

  if (q.isPending) return <main className="main"><div className="empty">Загрузка…</div></main>;
  if (q.isError || !team) return <main className="main"><div className="empty">Команда не найдена</div></main>;
  if (!member || !name) return <main className="main"><div className="empty">В команде нет участника {name}</div></main>;

  const active = team.state === "active";
  const paused = member.state === "paused";
  const st = stateOf(member, live ?? undefined, active);
  const queued = team.mail.filter((m) => m.to === name && !m.deliveredAt);
  const pausedAt = [...team.log].reverse().find((e) => e.event === "member_paused" && e.member === name)?.at;
  const who = displayName(name);
  // The idea planner: the plan it keeps sits next to the chat instead of the agent's details.
  const idea = isIdea;
  // Interrupting a paused agent makes no sense: its mail waits anyway.
  const lvl: MailLevel = paused ? "high" : level;

  const submit = () => {
    const text = draft.trim();
    if (!text) return;
    stick.current = true;
    const body = quote ? `К шагу «${quote}»:\n${text}` : text;
    send.mutate(
      { team: team.id, to: name, text: body, level: lvl },
      {
        onSuccess: () => {
          setDraft("");
          setQuote(undefined);
        },
        onError: (e) => toast(`Не отправлено: ${e.message}`, "error"),
      },
    );
  };
  const pause = (on: boolean) =>
    setPaused.mutate(
      { team: team.id, member: name, paused: on },
      { onSuccess: () => toast(on ? `${who} на паузе: почта подождёт` : `${who} продолжает${queued.length ? ` и получит ${queued.length} ${plural(queued.length, "письмо", "письма", "писем")}` : ""}`), onError: (e) => toast(e.message, "error") },
    );
  const doRestart = () =>
    restart.mutate(
      { team: team.id, member: name },
      { onSuccess: () => toast(`${who} перезапускается: разговор продолжится с текущими настройками роли`), onError: (e) => toast(`${who} не перезапущен: ${e.message}`, "error") },
    );
  // On a phone the chip is the level switch: the next correction stops the step.
  const interruptNow = () => {
    setLevel(level === "interrupt" ? "high" : "interrupt");
    inputRef.current?.focus();
  };

  return (
    <>
      <main className="main agent-chat">
        <header className="topbar">
          <Link to={`/team/${encodeURIComponent(team.id)}`} className="icon-btn m-only" aria-label="Назад к команде">
            <Icon.back />
          </Link>
          <nav className="crumbs d-only" aria-label="Путь">
            <Link to="/tasks">Задачи</Link>
            <span>/</span>
            <Link to={`/tasks?task=${encodeURIComponent(team.task)}`} className="mono">
              {team.task}
            </Link>
            <span>/</span>
            <Link to={`/team/${encodeURIComponent(team.id)}`}>Команда</Link>
            <span>/</span>
            <span className="here">{who}</span>
          </nav>
          <span className="ac-who m-only">
            <Avatar role={member.role} name={member.name} activity={active ? member.activity : undefined} size="md" />
            <span>
              <b>{who}</b> <span className="muted">{ROLE_TITLE_RU[member.role] ?? member.role}</span>
              <span className={`ac-state ${st.cls}`}>
                {st.cls === "working" && <span className="spin" />}
                {st.text}
              </span>
            </span>
          </span>
          <span className="grow" />
          {active && member.state !== "stopped" && (
            <span className="ac-actions d-only">
              <button type="button" className="btn sm" onClick={() => pause(!paused)} disabled={setPaused.isPending}>
                {paused ? "Продолжить" : "Пауза"}
              </button>
              <button type="button" className="btn sm" onClick={doRestart} disabled={restart.isPending} title="Новый процесс с текущими настройками роли; разговор продолжится">
                Перезапустить
              </button>
            </span>
          )}
        </header>

        <nav className="ac-tabs" aria-label="Участники команды">
          <Link to={`/team/${encodeURIComponent(team.id)}`} className="ac-tab">
            Схема
          </Link>
          <span className="ac-sep" />
          {team.members.map((m) => (
            <Link key={m.name} to={`/team/${encodeURIComponent(team.id)}/${encodeURIComponent(m.name)}`} className={`ac-tab${m.name === name ? " on" : ""}`} aria-current={m.name === name ? "page" : undefined}>
              <Avatar role={m.role} name={m.name} size="md" />
              {displayName(m.name)}
              {active && m.activity === "working" && <span className="spin" />}
              {m.state === "paused" && <span className="ac-dot amber" title="на паузе" />}
            </Link>
          ))}
        </nav>

        <div className={`ac-feed${paused ? " dim" : ""}`} ref={feedRef} role="log" aria-label={`Разговор ${who}`} onScroll={(e) => (stick.current = e.currentTarget.scrollHeight - e.currentTarget.scrollTop - e.currentTarget.clientHeight < 60)}>
          <div className="inner">
            {paused && conversation.length > 0 && <div className="ac-caption">Разговор остановлен паузой{pausedAt ? ` в ${clock(pausedAt)}` : ""}</div>}
            {idea && ideaTask && (
              <article className="mail ac-mail mine">
                <span className="slot">
                  <Avatar role="human" name="Вы" size="md" />
                </span>
                <div className="body">
                  <div className="meta">
                    <b>Ваша идея</b>
                  </div>
                  <div className="text">
                    <Markdown text={ideaTask.description} />
                  </div>
                </div>
                <span className="t">{clock(ideaTask.created)}</span>
              </article>
            )}
            {rows.map((r) => (
              <Fragment key={r.key}>
                {r.kind === "mail" && <MailRow row={r} team={team} paused={paused} />}
                {r.kind === "say" && (
                  <div className="ac-say">
                    <Avatar role={member.role} name={member.name} size="md" />
                    <div className="text">
                      <Markdown text={r.text} />
                    </div>
                    <span className="t">{r.at ? clock(r.at) : ""}</span>
                  </div>
                )}
                {r.kind === "steps" && (
                  <Steps
                    row={r}
                    open={open}
                    toggle={toggle}
                    paused={paused}
                    onQuote={active ? (t) => (setQuote(quoteOf(t)), inputRef.current?.focus()) : undefined}
                  />
                )}
                {r.kind === "note" && (
                  <div className="sys">
                    <span />
                    <span className="body">{r.text}</span>
                    <span className="t">{r.at ? clock(r.at) : ""}</span>
                  </div>
                )}
              </Fragment>
            ))}
            {!rows.length && !idea && (
              <div className="empty">
                {peek.isPending ? "Загрузка разговора…" : live ? "Разговор пока пуст" : "Сессия агента не запущена: разговор появится, когда он получит письмо"}
              </div>
            )}
          </div>
        </div>

        {active && member.state !== "stopped" && (
          <>
            <div className="ac-chips m-only">
              {paused ? (
                <>
                  <button type="button" className="chip primary" onClick={() => pause(false)} disabled={setPaused.isPending}>
                    Продолжить с почтой
                  </button>
                  <button type="button" className="chip" onClick={doRestart} disabled={restart.isPending}>
                    Перезапустить
                  </button>
                </>
              ) : (
                <>
                  <button type="button" className="chip amber" aria-pressed={level === "interrupt"} onClick={interruptNow}>
                    Прервать шаг
                  </button>
                  <button type="button" className="chip" onClick={() => pause(true)}>
                    Пауза
                  </button>
                  <button type="button" className={`chip${idea ? " primary" : ""}`} onClick={() => setInfo(!info)} aria-expanded={info}>
                    {idea ? "План" : "Что он делает?"}
                  </button>
                </>
              )}
            </div>
            <form
              className={`ac-composer${lvl === "interrupt" ? " hard" : ""}`}
              aria-label={`Поправка для ${who}`}
              onSubmit={(e) => {
                e.preventDefault();
                submit();
              }}
            >
              {quote && (
                <div className="ac-quote">
                  к шагу <span className="mono">{quote}</span>
                  <button type="button" className="icon-btn" aria-label="Убрать ссылку на шаг" onClick={() => setQuote(undefined)}>
                    <Icon.close size={11} />
                  </button>
                </div>
              )}
              <textarea
                ref={inputRef}
                aria-label="Текст поправки"
                rows={2}
                value={draft}
                placeholder={paused ? `Добавить к почте, которую ${who} получит при продолжении` : idea ? `Ответить ${who} или попросить поправить план` : `Что поправить в работе ${who}?`}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && (e.metaKey || e.ctrlKey) && (e.preventDefault(), submit())}
              />
              <div className="ac-row">
                {!paused && !idea && (
                  <div role="radiogroup" aria-label="Когда доставить" className="ac-levels">
                    {LEVELS.map((l) => (
                      <button key={l.level} type="button" role="radio" aria-checked={level === l.level} className={level === l.level ? `on ${l.level}` : ""} onClick={() => setLevel(l.level)}>
                        {l.label}
                      </button>
                    ))}
                  </div>
                )}
                <span className="hint">{paused ? "придёт при продолжении" : idea ? `${who} ответит здесь, план обновится справа` : LEVELS.find((l) => l.level === level)?.hint}</span>
                <span className="grow" />
                <button type="submit" className={`btn sm ${lvl === "interrupt" ? "amber" : "primary"}`} disabled={!draft.trim() || send.isPending} title="Отправить (⌘↵)">
                  {lvl === "interrupt" ? "Прервать и отправить" : "Отправить"}
                </button>
              </div>
            </form>
          </>
        )}
      </main>

      <aside className={`team-aside ac-aside${info ? " open" : ""}`} aria-label={paused ? "Пауза" : idea ? "План" : "Об агенте"}>
        {paused ? (
          <PausedPanel who={who} since={pausedAt} queued={queued} busy={setPaused.isPending || restart.isPending} onResume={() => pause(false)} onRestart={doRestart} />
        ) : idea ? (
          <IdeaPlan taskId={team.task} planner={who} onClose={() => setInfo(false)} />
        ) : (
          <AgentInfo member={member} team={team} live={live ?? undefined} st={st} queued={queued} />
        )}
      </aside>
    </>
  );
}

const stepsWord = (n: number) => plural(n, "шаг", "шага", "шагов");

/**
 * A run of the agent's steps. One step is one faded line: a tool call names its
 * tool, arguments, time and outcome; a thought is «Thinking…». Several steps in
 * a row fold into one line that counts them and names their tools. A click
 * opens the run, a step, its output or its thought.
 */
function Steps({ row, open, toggle, paused, onQuote }: { row: Extract<Row, { kind: "steps" }>; open: Set<string>; toggle: (key: string) => void; paused: boolean; onQuote?: (t: Extract<Step, { kind: "tool" }>) => void }) {
  const list = (
    <div className="ac-steplist">
      {row.steps.map((s) => (
        <StepLine key={s.key} step={s} open={open.has(s.key)} toggle={() => toggle(s.key)} paused={paused} onQuote={onQuote} />
      ))}
    </div>
  );
  if (row.steps.length === 1) return <div className="ac-steps">{list}</div>;

  const calls = row.steps.filter((s): s is Extract<Step, { kind: "tool" }> => s.kind === "tool");
  const thoughts = row.steps.length - calls.length;
  const names = new Map<string, { n: number; bad: boolean }>();
  for (const c of calls) {
    const x = names.get(c.name) ?? { n: 0, bad: false };
    names.set(c.name, { n: x.n + 1, bad: x.bad || !!c.error });
  }
  const now = calls.find((c) => c.running);
  const failed = calls.some((c) => c.error);
  const unfinished = !now && calls.some((c) => c.out === undefined);
  const ms = calls.reduce((a, c) => a + (c.took ?? 0), 0);
  const on = open.has(row.key);
  return (
    <div className={`ac-steps${on ? " open" : ""}`}>
      <button type="button" className={`ac-step group${now ? " running" : ""}${on ? " open" : ""}`} aria-expanded={on} onClick={() => toggle(row.key)} title={on ? "Свернуть шаги" : "Показать шаги"}>
        <Icon.chevron size={10} />
        <svg className="stack" width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
          <rect x="2.5" y="2.5" width="11" height="3.2" rx="1" />
          <rect x="2.5" y="6.4" width="11" height="3.2" rx="1" />
          <rect x="2.5" y="10.3" width="11" height="3.2" rx="1" />
        </svg>
        <span className="count">
          {row.steps.length} {stepsWord(row.steps.length)}
        </span>
        <span className="chips">
          {[...names].map(([name, x]) => (
            <span key={name} className={`ac-chip${x.bad ? " bad" : ""}`}>
              {x.n > 1 ? `${name} ×${x.n}` : name}
            </span>
          ))}
          {thoughts > 0 && <span className="ac-chip think">{thoughts > 1 ? `Thinking ×${thoughts}` : "Thinking"}</span>}
        </span>
        <span className="args">{now?.args}</span>
        {!now && ms > 0 && <span className="took">{took(ms)}</span>}
        {now && <span className="spin" />}
        <span className={`st${now ? " live" : failed ? " bad" : ""}`}>{now ? `идёт ${sinceShort(now.since)}`.trim() : failed ? "ошибка" : unfinished ? (paused ? "остановлено паузой" : "") : "готово"}</span>
      </button>
      {on && list}
    </div>
  );
}

/** «идёт 2 мин» rather than «идёт сейчас». */
const sinceShort = (since?: string) => (since && timeAgo(since) !== "сейчас" ? timeAgo(since) : "");

function StepLine({ step, open, toggle, paused, onQuote }: { step: Step; open: boolean; toggle: () => void; paused: boolean; onQuote?: (t: Extract<Step, { kind: "tool" }>) => void }) {
  if (step.kind === "think") {
    return (
      <div className="ac-stepitem">
        <button type="button" className={`ac-step${open ? " open" : ""}`} aria-expanded={open} onClick={toggle}>
          <Icon.chevron size={10} />
          <span className="ac-chip think">Thinking…</span>
        </button>
        {open && <p className="ac-thought">{step.text}</p>}
      </div>
    );
  }
  const st = step.running ? `идёт ${sinceShort(step.since)}`.trim() : step.error ? "ошибка" : step.out !== undefined ? "готово" : paused ? "остановлено паузой" : "";
  return (
    <div className="ac-stepitem">
      <div className="ac-steprow">
        <button type="button" className={`ac-step${step.running ? " running" : ""}${open ? " open" : ""}`} aria-expanded={open} onClick={toggle} disabled={!step.out} title={step.args}>
          <Icon.chevron size={10} />
          <span className={`ac-chip${step.error ? " bad" : ""}`}>{step.name}</span>
          <span className="args">{step.args}</span>
          {step.took !== undefined && <span className="took">{took(step.took)}</span>}
          {step.running && <span className="spin" />}
          <span className={`st${step.running ? " live" : step.error ? " bad" : ""}`}>{st}</span>
        </button>
        {onQuote && (
          <button type="button" className="quote" aria-label="Поправить с этого шага" title="Поправить с этого шага" onClick={() => onQuote(step)}>
            <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
              <path d="M3 4v5a3 3 0 0 0 3 3h7M10 9l3 3-3 3" />
            </svg>
          </button>
        )}
      </div>
      {open && step.out && <pre className="out">{step.out}</pre>}
    </div>
  );
}

function MailRow({ row, team, paused }: { row: Extract<Row, { kind: "mail" }>; team: TeamDetail; paused: boolean }) {
  const m = row.mail[0];
  if (!m) {
    return (
      <div className="sys">
        <span />
        <span className="body">{row.text}</span>
        <span className="t">{row.at ? clock(row.at) : ""}</span>
      </div>
    );
  }
  const mine = m.fromRole === "human";
  const from = mine ? "Вы" : m.fromRole === "orchestrator" ? "Оркестратор" : m.kind === "system" || m.kind === "kickoff" ? "genie" : displayName(m.from);
  const hard = m.level === "interrupt";
  return (
    <>
      {row.mail.map((x, i) => (
        <article key={x.id} className={`mail ac-mail${mine ? " mine" : ""}${hard ? " urgent" : ""}${i ? " cont" : ""}`}>
          <span className="slot">{!i && <Avatar role={mine ? "human" : x.fromRole} name={x.from} size="md" />}</span>
          <div className="body">
            {!i && (
              <div className="meta">
                <b>{from}</b>
                {hard && <span className="urgent-tag">прервать шаг</span>}
                {x.level === "high" && mine && <span className="mail-kind">на следующем шаге</span>}
                {x.intent && <span className="mail-kind">{INTENT_LABEL[x.intent]}</span>}
              </div>
            )}
            <div className="text">
              <MessageText text={x.text} team={team.id} />
            </div>
            {mine && <span className={`receipt${row.pending ? "" : " ok"}`}>{row.pending ? (paused ? "придёт при продолжении" : hard ? "прерывает шаг…" : "ждёт границы шага") : "доставлено"}</span>}
          </div>
          <span className="t">{clock(x.at)}</span>
        </article>
      ))}
    </>
  );
}

function AgentInfo({ member, team, live, st, queued }: { member: Member; team: TeamDetail; live?: LiveSession; st: ReturnType<typeof stateOf>; queued: Mail[] }) {
  return (
    <div className="ac-info">
      <div className="who">
        <Avatar role={member.role} name={member.name} size="lg" />
        <span>
          <b>{displayName(member.name)}</b>
          <span className="muted">
            {ROLE_TITLE_RU[member.role] ?? member.role} · <Link to={`/agents?tab=roles&id=${encodeURIComponent(member.role)}`}>роль</Link>
          </span>
        </span>
      </div>
      <div className={`ac-statebox ${st.cls}`}>
        <span className="line">
          {st.cls === "working" && <span className="spin" />}
          {st.text}
        </span>
        <span className="sub">{live?.tool ? live.tool.args : st.sub}</span>
      </div>
      <dl className="ac-props">
        <dt>Модель</dt>
        <dd>
          {team.state === "active" && member.state !== "stopped" ? (
            <ModelMenu team={team.id} member={member} />
          ) : (
            <span className="mono">
              {member.model ? member.model.replace(/^[^/]+\//, "") : "как у роли"}
              {member.thinking ? ` · ${member.thinking}` : ""}
            </span>
          )}
        </dd>
        {live?.contextTokens !== undefined && (
          <>
            <dt>Контекст</dt>
            <dd>{live.contextTokens >= 1000 ? `${Math.round(live.contextTokens / 1000)} тыс. токенов` : `${live.contextTokens} токенов`}</dd>
          </>
        )}
        <ChatSpend team={team.id} member={member.name} />
        {live && (
          <>
            <dt>Прогонов</dt>
            <dd>
              {live.runs} · {live.failures ? `${live.failures} ${plural(live.failures, "сбой", "сбоя", "сбоев")} подряд` : "без сбоев"}
            </dd>
          </>
        )}
        {team.worktree && (
          <>
            <dt>Ветка</dt>
            <dd className="mono">{team.worktree.branch}</dd>
          </>
        )}
      </dl>
      {member.status && member.status !== "starting" && (
        <section>
          <h2>Статус</h2>
          <p className="said">{member.status}</p>
        </section>
      )}
      <section>
        <h2>Ждут доставки</h2>
        <p className="muted">{queued.length ? `${queued.length} ${plural(queued.length, "письмо", "письма", "писем")}: ${queued.map((m) => (m.fromRole === "human" ? "ваше" : displayName(m.from))).join(", ")}` : "Нет писем."}</p>
      </section>
      <p className="ac-help">Поправка приходит агенту письмом в его разговор. «Прервать шаг» останавливает текущую команду; «на следующем шаге» ждёт, пока он закончит.</p>
    </div>
  );
}

/** What the agent's whole chat spent; nothing until it spends. */
function ChatSpend({ team, member }: { team: string; member: string }) {
  const spend = useChatUsage(team, member).data?.spend;
  if (!spend?.calls) return null;
  return (
    <>
      <dt>Потрачено</dt>
      <dd>
        <SpendText spend={spend} />
      </dd>
    </>
  );
}

function PausedPanel({ who, since, queued, busy, onResume, onRestart }: { who: string; since?: string; queued: Mail[]; busy: boolean; onResume: () => void; onRestart: () => void }) {
  return (
    <div className="ac-info">
      <div className="ac-paused">
        <b>
          <svg width="12" height="12" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
            <rect x="3.5" y="2.5" width="3" height="11" rx="1" />
            <rect x="9.5" y="2.5" width="3" height="11" rx="1" />
          </svg>
          {who} на паузе{since ? ` с ${clock(since)}` : ""}
        </b>
        <span>Сессия остановлена, текущая команда прервана. Письма копятся и придут все сразу, когда вы продолжите. Остальная команда работает.</span>
      </div>
      <section>
        <h2>
          Придёт при продолжении <span className="muted">{queued.length}</span>
        </h2>
        {queued.map((m) => (
          <article key={m.id} className={`ac-queued${m.fromRole === "human" ? " mine" : ""}`}>
            <span className="ac-qhd">
              <Avatar role={m.fromRole === "human" ? "human" : m.fromRole} name={m.from} />
              <b>{m.fromRole === "human" ? "Вы" : m.fromRole === "orchestrator" ? "Оркестратор" : displayName(m.from)}</b>
              {m.intent && <span className="mail-kind">{INTENT_LABEL[m.intent]}</span>}
              <span className="t">{clock(m.at)}</span>
            </span>
            <span className="text">{m.text}</span>
          </article>
        ))}
        {!queued.length && <p className="muted">Писем нет. Поправку можно написать внизу: она придёт первой.</p>}
      </section>
      <span className="grow" />
      <div className="ac-paused-actions">
        <button type="button" className="btn primary" onClick={onResume} disabled={busy}>
          Продолжить с почтой
        </button>
        <button type="button" className="btn" onClick={onRestart} disabled={busy}>
          Перезапустить
        </button>
      </div>
      <p className="ac-help">Перезапуск поднимет новый процесс с текущими настройками роли; разговор, рабочая копия и почта сохранятся.</p>
    </div>
  );
}
