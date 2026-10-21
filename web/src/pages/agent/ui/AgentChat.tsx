import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Link, useParams } from "react-router";
import { displayName, ROLE_TITLE_RU } from "@/entities/member";
import { type LiveSession, LongOutput, type Mail, type MailLevel, MessageText, PartyAvatar, type PeekMessage, type TeamDetail, usePeek, useRestartMember, useSendMail, useSetPaused, useTeam } from "@/entities/team";
import { SpendText, useChatUsage } from "@/entities/usage";
import { useTask } from "@/entities/task";
import { ModelMenu } from "@/features/agent-model";
import { IDEA_TEMPLATE, IdeaPlan } from "@/features/shape-idea";
import { clock, plural, timeAgo, useTick } from "@/shared/lib";
import { Breadcrumbs, Button, Character, characterState, Chip, EmptyState, Icon, IconButton, LoadingState, Markdown, Prose, Sheet, Skeleton, Tag, useToast, WorkingArcs } from "@/shared/ui";
import "./agent.css";
import "@/widgets/team-view/team.css";
import "./agent-sheets.css";

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

  // The character in the header reports the agent: thinking while it works, surprised when it needs the
  // owner, asleep otherwise — and waking for a moment after the owner resumes it.
  const memberPaused = member?.state === "paused";
  const [woke, setWoke] = useState(false);
  const wasPaused = useRef(memberPaused);
  useEffect(() => {
    if (wasPaused.current && !memberPaused) {
      setWoke(true);
      wasPaused.current = memberPaused;
      const t = window.setTimeout(() => setWoke(false), 5_000);
      return () => window.clearTimeout(t);
    }
    wasPaused.current = memberPaused;
  }, [memberPaused]);

  useEffect(() => {
    setQuote(undefined);
    setOpen(new Set());
    stick.current = true;
  }, [agentKey]);
  useLayoutEffect(() => {
    const el = feedRef.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [rows.length, agentKey]);

  if (q.isPending)
    return (
      <main className="main ac-page">
        <div className="ac-center">
          <LoadingState label="Загрузка разговора" width={520}>
            <Skeleton width={160} height={20} />
            <Skeleton height={52} />
            <Skeleton height={52} />
            <Skeleton height={40} width="68%" />
          </LoadingState>
        </div>
      </main>
    );
  if (q.isError || !team) return <main className="main ac-page"><div className="ac-center">Команда не найдена</div></main>;
  if (!member || !name) return <main className="main ac-page"><div className="ac-center">В команде нет участника {name}</div></main>;

  const active = team.state === "active";
  const paused = member.state === "paused";
  const st = stateOf(member, live ?? undefined, active);
  const queued = team.mail.filter((m) => m.to === name && !m.deliveredAt);
  const pausedAt = [...team.log].reverse().find((e) => e.event === "member_paused" && e.member === name)?.at;
  const who = displayName(name);
  // The character reports this agent: one state per view, never invented.
  const pose = characterState({ working: working ? 1 : 0, waiting: st.cls === "error" ? 1 : 0, resumed: woke });
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
        <header className="ac-head">
          <Link to={`/team/${encodeURIComponent(team.id)}`} className="g-icon-btn m-only" aria-label="Назад к команде">
            <Icon.back />
          </Link>
          <Character state={pose} size={38} />
          <div className="ac-who">
            <div className="ac-nm">
              <b>{who}</b>
              <span className="ac-role">{ROLE_TITLE_RU[member.role] ?? member.role}</span>
            </div>
            <span className={`ac-state ${st.cls}`}>
              {st.cls === "working" ? <WorkingArcs size={13} label="Работает" /> : <i className="ac-led" aria-hidden="true" />}
              {st.text}
            </span>
          </div>
          <span className="grow" />
          <span className="ac-crumbs d-only">
            <Breadcrumbs
              items={[
                { label: "Задачи", to: "/tasks" },
                { label: <span className="g-mono">{team.task}</span>, to: `/tasks?task=${encodeURIComponent(team.task)}` },
                { label: "Команда", to: `/team/${encodeURIComponent(team.id)}` },
                { label: who },
              ]}
            />
          </span>
          {active && member.state !== "stopped" && (
            <span className="ac-actions d-only">
              <Button variant="ghost" sm onClick={() => pause(!paused)} disabled={setPaused.isPending}>
                {paused ? "Продолжить" : "Пауза"}
              </Button>
              <Button variant="secondary" sm onClick={doRestart} disabled={restart.isPending} title="Новый процесс с текущими настройками роли; разговор продолжится">
                Перезапустить
              </Button>
            </span>
          )}
        </header>

        <nav className="ac-tabs g-tabs" aria-label="Участники команды">
          <Link to={`/team/${encodeURIComponent(team.id)}`} className="ac-tab">
            Схема
          </Link>
          {team.members.map((m) => (
            <Link key={m.name} to={`/team/${encodeURIComponent(team.id)}/${encodeURIComponent(m.name)}`} className="ac-tab" aria-current={m.name === name ? "page" : undefined}>
              <PartyAvatar role={m.role} name={m.name} sm />
              {displayName(m.name)}
              {active && m.activity === "working" && <WorkingArcs size={11} label="Работает" />}
              {m.state === "paused" && <i className="ac-dot" aria-hidden="true" title="на паузе" />}
            </Link>
          ))}
        </nav>

        <div className={`ac-feed${paused ? " dim" : ""}`} ref={feedRef} role="log" aria-label={`Разговор ${who}`} onScroll={(e) => (stick.current = e.currentTarget.scrollHeight - e.currentTarget.scrollTop - e.currentTarget.clientHeight < 60)}>
          <div className="ac-inner">
            {paused && conversation.length > 0 && <div className="ac-caption">Разговор остановлен паузой{pausedAt ? ` в ${clock(pausedAt)}` : ""}</div>}
            {idea && ideaTask && (
              <article className="ac-msg mine">
                <span className="ac-slot">
                  <PartyAvatar role="human" name="Вы" />
                </span>
                <div className="ac-body">
                  <div className="ac-from">
                    <b>Ваша идея</b>
                  </div>
                  <Sheet className="ac-bubble">
                    <Prose>
                      <Markdown text={ideaTask.description} />
                    </Prose>
                  </Sheet>
                </div>
                <span className="ac-time">{clock(ideaTask.created)}</span>
              </article>
            )}
            {rows.map((r) => (
              <Fragment key={r.key}>
                {r.kind === "mail" && <MailRow row={r} team={team} paused={paused} />}
                {r.kind === "say" && (
                  <div className="ac-say">
                    <span className="ac-slot">
                      <PartyAvatar role={member.role} name={member.name} />
                    </span>
                    <div className="ac-body">
                      <Prose>
                        <Markdown text={r.text} />
                      </Prose>
                    </div>
                    <span className="ac-time">{r.at ? clock(r.at) : ""}</span>
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
                  <div className="ac-note">
                    <span className="ac-note-text">{r.text}</span>
                    <span className="ac-time">{r.at ? clock(r.at) : ""}</span>
                  </div>
                )}
              </Fragment>
            ))}
            {!rows.length && !idea &&
              (peek.isPending ? (
                <LoadingState label="Загрузка разговора" lines={2} width={320} />
              ) : (
                <EmptyState title={live ? "Разговор пока пуст" : "Сессия агента не запущена: разговор появится, когда он получит письмо"} />
              ))}
          </div>
        </div>

        {active && member.state !== "stopped" && (
          <>
            <div className="ac-chips m-only">
              {paused ? (
                <>
                  <Chip className="ac-chip-primary" onClick={() => pause(false)} disabled={setPaused.isPending}>
                    Продолжить с почтой
                  </Chip>
                  <Chip onClick={doRestart} disabled={restart.isPending}>
                    Перезапустить
                  </Chip>
                </>
              ) : (
                <>
                  <Chip pressed={level === "interrupt"} onClick={interruptNow}>
                    Прервать шаг
                  </Chip>
                  <Chip onClick={() => pause(true)}>Пауза</Chip>
                  <Chip pressed={info} aria-expanded={info} onClick={() => setInfo(!info)}>
                    {idea ? "План" : "Что он делает?"}
                  </Chip>
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
                  <span>к шагу</span>
                  <span className="g-mono">{quote}</span>
                  <IconButton label="Убрать ссылку на шаг" className="ac-quote-x" onClick={() => setQuote(undefined)}>
                    <Icon.close size={11} />
                  </IconButton>
                </div>
              )}
              <div className="ac-box">
                <textarea
                  ref={inputRef}
                  className="g-textarea ac-input"
                  aria-label="Текст поправки"
                  rows={2}
                  value={draft}
                  placeholder={paused ? `Добавить к почте, которую ${who} получит при продолжении` : idea ? `Ответить ${who} или попросить поправить план` : `Что поправить в работе ${who}?`}
                  onChange={(e) => setDraft(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && (e.metaKey || e.ctrlKey) && (e.preventDefault(), submit())}
                />
                <div className="ac-row">
                  {!paused && !idea && (
                    <div role="radiogroup" aria-label="Когда доставить" className="ac-levels g-seg">
                      {LEVELS.map((l) => (
                        <button key={l.level} type="button" role="radio" aria-checked={level === l.level} onClick={() => setLevel(l.level)}>
                          {l.label}
                        </button>
                      ))}
                    </div>
                  )}
                  <span className="ac-hint">{paused ? "придёт при продолжении" : idea ? `${who} ответит здесь, план обновится справа` : LEVELS.find((l) => l.level === level)?.hint}</span>
                  <span className="grow" />
                  <Button type="submit" variant="primary" sm className="ac-send" disabled={!draft.trim() || send.isPending} title="Отправить (⌘↵)">
                    {lvl === "interrupt" ? "Прервать и отправить" : "Отправить"}
                  </Button>
                </div>
              </div>
            </form>
          </>
        )}
      </main>

      {info && <button type="button" className="g-scrim m-only" aria-label="Закрыть панель" onClick={() => setInfo(false)} />}
      <aside className={`ac-aside${info ? " open" : ""}`} aria-label={paused ? "Пауза" : idea ? "План" : "Об агенте"}>
        <span className="ac-aside-head m-only">
          <span>{paused ? "Пауза" : idea ? "План" : "Об агенте"}</span>
          <IconButton label="Закрыть" onClick={() => setInfo(false)}>
            <Icon.close size={13} />
          </IconButton>
        </span>
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
            <Tag key={name} tone={x.bad ? "signal" : "outline"}>
              {x.n > 1 ? `${name} ×${x.n}` : name}
            </Tag>
          ))}
          {thoughts > 0 && <Tag>{thoughts > 1 ? `Thinking ×${thoughts}` : "Thinking"}</Tag>}
        </span>
        <span className="args">{now?.args}</span>
        {!now && ms > 0 && <span className="took">{took(ms)}</span>}
        {now && <WorkingArcs size={12} label="Идёт работа" />}
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
          <Tag>Thinking…</Tag>
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
          <Tag tone={step.error ? "signal" : "outline"}>{step.name}</Tag>
          <span className="args">{step.args}</span>
          {step.took !== undefined && <span className="took">{took(step.took)}</span>}
          {step.running && <WorkingArcs size={12} label="Идёт работа" />}
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
      {open && step.out && <div className="ac-out"><LongOutput text={step.out} /></div>}
    </div>
  );
}

function MailRow({ row, team, paused }: { row: Extract<Row, { kind: "mail" }>; team: TeamDetail; paused: boolean }) {
  const m = row.mail[0];
  if (!m) {
    return (
      <div className="ac-note">
        <span className="ac-note-text">{row.text}</span>
        <span className="ac-time">{row.at ? clock(row.at) : ""}</span>
      </div>
    );
  }
  const mine = m.fromRole === "human";
  const from = mine ? "Вы" : m.fromRole === "orchestrator" ? "Оркестратор" : m.kind === "system" || m.kind === "kickoff" ? "genie" : displayName(m.from);
  const hard = m.level === "interrupt";
  return (
    <>
      {row.mail.map((x, i) => (
        <article key={x.id} className={`ac-msg${mine ? " mine" : ""}${hard ? " urgent" : ""}${i ? " cont" : ""}`}>
          <span className="ac-slot">{!i && <PartyAvatar role={mine ? "human" : x.fromRole} name={x.from} />}</span>
          <div className="ac-body">
            {!i && (
              <div className="ac-from">
                <b>{from}</b>
                {hard && <Tag tone="signal">прервать шаг</Tag>}
                {x.level === "high" && mine && <Tag tone="outline">на следующем шаге</Tag>}
                {x.intent && <Tag tone="outline">{INTENT_LABEL[x.intent]}</Tag>}
              </div>
            )}
            {mine ? (
              <Sheet className="ac-bubble">
                <div className="ac-text">
                  <MessageText text={x.text} team={team.id} />
                </div>
              </Sheet>
            ) : (
              <div className="ac-text">
                <MessageText text={x.text} team={team.id} />
              </div>
            )}
            {mine && <span className={`ac-receipt${row.pending ? "" : " ok"}`}>{row.pending ? (paused ? "придёт при продолжении" : hard ? "прерывает шаг…" : "ждёт границы шага") : "доставлено"}</span>}
          </div>
          <span className="ac-time">{clock(x.at)}</span>
        </article>
      ))}
    </>
  );
}

function AgentInfo({ member, team, live, st, queued }: { member: Member; team: TeamDetail; live?: LiveSession; st: ReturnType<typeof stateOf>; queued: Mail[] }) {
  return (
    <div className="ac-info">
      <div className="who">
        <PartyAvatar role={member.role} name={member.name} />
        <span>
          <b>{displayName(member.name)}</b>
          <span className="ac-muted">
            {ROLE_TITLE_RU[member.role] ?? member.role} · <Link to={`/agents?tab=roles&id=${encodeURIComponent(member.role)}`}>роль</Link>
          </span>
        </span>
      </div>
      <div className={`ac-statebox ${st.cls}`}>
        <span className="line">
          {st.cls === "working" && <WorkingArcs size={13} label="Работает" />}
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
            <span className="g-mono">
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
            <dd className="g-mono">{team.worktree.branch}</dd>
          </>
        )}
      </dl>
      {member.status && member.status !== "starting" && (
        <section>
          <h2>Статус</h2>
          <p className="ac-said">{member.status}</p>
        </section>
      )}
      <section>
        <h2>Ждут доставки</h2>
        <p className="ac-muted">{queued.length ? `${queued.length} ${plural(queued.length, "письмо", "письма", "писем")}: ${queued.map((m) => (m.fromRole === "human" ? "ваше" : displayName(m.from))).join(", ")}` : "Нет писем."}</p>
      </section>
      <p className="ac-help">«Прервать шаг» остановит текущую команду.</p>
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
          Придёт при продолжении <span className="n">{queued.length}</span>
        </h2>
        {queued.map((m) => (
          <article key={m.id} className={`ac-queued${m.fromRole === "human" ? " mine" : ""}`}>
            <span className="ac-qhd">
              <PartyAvatar role={m.fromRole === "human" ? "human" : m.fromRole} name={m.from} sm />
              <b>{m.fromRole === "human" ? "Вы" : m.fromRole === "orchestrator" ? "Оркестратор" : displayName(m.from)}</b>
              {m.intent && <Tag tone="outline">{INTENT_LABEL[m.intent]}</Tag>}
              <span className="ac-time">{clock(m.at)}</span>
            </span>
            <span className="ac-text">{m.text}</span>
          </article>
        ))}
        {!queued.length && <p className="ac-muted">Писем нет.</p>}
      </section>
      <span className="grow" />
      <div className="ac-paused-actions">
        <Button variant="primary" onClick={onResume} disabled={busy}>
          Продолжить с почтой
        </Button>
        <Button variant="secondary" onClick={onRestart} disabled={busy}>
          Перезапустить
        </Button>
      </div>
      <p className="ac-help">При перезапуске разговор, рабочая копия и почта сохранятся.</p>
    </div>
  );
}
