import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Link, useParams } from "react-router";
import { type LiveState, liveTeam, MAIL_TITLE, useAgentConfig } from "@/entities/agent-config";
import { Avatar, displayName, ROLE_TITLE_RU } from "@/entities/member";
import { RemoveMemberButton, RestartMemberButton, TeamActions } from "@/features/manage-team";
import { StageBars, STAGES, stageOf, STATUS_NAME } from "@/entities/task";
import { type LiveSession, type Mail, type MailLevel, MessageText, type TeamDetail, useSendMail, useTeam } from "@/entities/team";
import { clock, dayLabel, plural, readPref, timeAgo, useTick, writePref } from "@/shared/lib";
import { Icon, useToast } from "@/shared/ui";
import { TeamScheme } from "./TeamScheme.tsx";

/** One chat entry: broadcast rows (one per recipient) and kickoffs are merged. */
interface Entry {
  key: string;
  at: string;
  kind: "system" | "agent" | "mine";
  from: string;
  fromRole: string;
  to: string[];
  text: string;
  urgent: boolean;
  level: MailLevel;
  intent?: Mail["intent"];
  delivered: boolean;
  title?: string;
}

const LEVEL_LABEL: Record<MailLevel, string> = { low: "не срочно", normal: "обычное", high: "важное", interrupt: "прервать шаг" };
const INTENT_LABEL: Record<NonNullable<Mail["intent"]>, string> = { question: "вопрос", blocker: "блокер", verdict: "вердикт", done: "готово", fyi: "к сведению" };
const STOPPED: Record<string, string> = {
  owner: "остановлена владельцем",
  orchestrator: "остановлена оркестратором",
  task_closed: "остановлена: задача закрыта",
  planned: "план идеи заведён",
  launch_failed: "не запустилась",
  all_lost: "остановлена: связь потеряна",
};

/** What a message is for, and whether it is urgent: only what differs from the usual. */
function MailBadges({ level, intent }: { level: MailLevel; intent?: Mail["intent"] }) {
  return (
    <>
      {(level === "high" || level === "interrupt") && <span className="urgent-tag">{level === "interrupt" ? "прерывание" : "важное"}</span>}
      {intent && <span className="mail-kind">{INTENT_LABEL[intent]}</span>}
    </>
  );
}

function toEntries(mail: Mail[]): Entry[] {
  const out: Entry[] = [];
  for (const m of mail) {
    const last = out.at(-1);
    const kind: Entry["kind"] = m.kind === "kickoff" || m.kind === "system" ? "system" : m.fromRole === "human" ? "mine" : "agent";
    const sameSend = last && last.from === m.from && last.at.slice(0, 19) === m.at.slice(0, 19) && (last.text === m.text || (kind === "system" && last.kind === "system"));
    if (sameSend && last) {
      last.to.push(m.to);
      last.delivered &&= !!m.deliveredAt;
      continue;
    }
    out.push({
      key: String(m.id),
      at: m.at,
      kind,
      from: m.from.replace(/^owner \((.*)\)$/, "$1"),
      fromRole: m.fromRole,
      to: [m.to],
      text: m.text,
      urgent: !!m.urgent || (m.level as MailLevel) === "interrupt",
      level: m.level as MailLevel,
      intent: m.intent,
      delivered: !!m.deliveredAt,
      title: m.kind === "kickoff" ? "Команда запущена" : undefined,
    });
  }
  return out;
}

function recipients(to: string[], team: TeamDetail): string {
  const everyone = team.members.length + 1;
  if (to.length >= everyone - 1 && to.length > 1) return "всем";
  return to.map((x) => (x === "orchestrator" ? "оркестратор" : displayName(x))).join(", ");
}

const EVENT_TEXT: Record<string, (e: Record<string, unknown>) => string> = {
  team_created: () => "команда создана",
  member_started: (e) => `${displayName(String(e.member))} запущен`,
  member_stopped: (e) => `${displayName(String(e.member))} остановлен`,
  member_added: (e) => `добавлен ${displayName(String(e.member).split(":")[0])}`,
  status: (e) => `${displayName(String(e.member))}: ${e.status}`,
  task_status: (e) => `${e.task} → ${STATUS_NAME[e.status as keyof typeof STATUS_NAME] ?? e.status}`,
  agent_error: (e) => `${displayName(String(e.member))}: ошибка модели`,
  blocked: (e) => `${e.task} заблокирована: ${e.reason}`,
  team_stopped: () => "команда остановлена",
  member_lost: (e) => `${displayName(String(e.member))}: связь потеряна (${e.reason})`,
  member_recovered: (e) => `${displayName(String(e.member))}: снова на связи`,
  team_recovered: () => "связь с командой восстановлена",
  team_restarted: (e) => `перезапущены: ${(e.members as string[]).map(displayName).join(", ")}`,
  member_restarted: (e) => `${displayName(String(e.member))} перезапущен`,
  team_silent: (e) =>
    `команда ничего не делает ${Math.round(Number(e.idleSecs ?? 0) / 60)} мин — оркестратору отправлено письмо`,
  launch_failed: () => "не удалось запустить участников",
};

export function TeamView() {
  useTick(15_000);
  const { teamId } = useParams();
  const q = useTeam(teamId);
  const cfg = useAgentConfig().data;
  const send = useSendMail();
  const toast = useToast();
  const [to, setTo] = useState("all");
  const [level, setLevel] = useState<MailLevel>("normal");
  const [draft, setDraft] = useState("");
  // On a phone the scheme starts folded: the mail comes first there.
  const [scheme, setScheme] = useState(() => readPref("genie.teamScheme", window.innerWidth < 900 ? "closed" : "open") === "open");
  const chatRef = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const team = q.data;
  const entries = useMemo(() => (team ? toEntries(team.mail) : []), [team]);

  useEffect(() => setTo("all"), [teamId]);
  // An interrupt stops one agent's step: not for a broadcast or the orchestrator.
  useEffect(() => {
    if (level === "interrupt" && (to === "all" || to === "orchestrator")) setLevel("high");
  }, [to, level]);
  useLayoutEffect(() => {
    const el = chatRef.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [entries.length, teamId]);

  if (q.isPending) return <main className="main"><div className="empty">Загрузка…</div></main>;
  if (q.isError || !team) return <main className="main"><div className="empty">Команда не найдена</div></main>;

  const status = team.taskInfo?.status;
  const stage = status ? stageOf(status) : 0;
  const working = team.members.filter((m) => m.activity === "working");
  const active = team.state === "active";
  const spec = team.spec;
  const live = spec ? liveTeam(spec, team.members, status ?? "ready", active, displayName).live : {};
  const keyOf = (name: string) => spec?.members.find((m) => m.name === name)?.key ?? name;
  const templateKnown = !!spec?.template && !!cfg?.teams.some((t) => t.id === spec.template);

  const submit = () => {
    const text = draft.trim();
    if (!text) return;
    stick.current = true;
    send.mutate({ team: team.id, to, text, level }, { onSuccess: () => setDraft(""), onError: (e) => toast(`Не отправлено: ${e.message}`, "error") });
  };

  let lastDay = "";

  return (
    <>
      <main className="main">
        <header className="topbar team-top">
          <Link to="/active" className="icon-btn m-only" aria-label="Назад">
            <Icon.back />
          </Link>
          <nav className="crumbs" aria-label="Путь">
            <Link to="/active" className="d-only">
              Задачи
            </Link>
            <span className="d-only">/</span>
            <Link to={`/active?task=${encodeURIComponent(team.task)}`} className="mono">
              {team.task}
            </Link>
            <span>/</span>
            <span className="here">Команда</span>
          </nav>
          <span className="grow" />
          <TeamActions team={team} />
        </header>

        <div className="team-title">
          <h1>{team.taskInfo?.title ?? `Команда ${team.id}`}</h1>
          <p className="sub">
            {spec &&
              (spec.title ? (
                <span>
                  Шаблон {templateKnown ? <Link to={`/agents?tab=templates&id=${encodeURIComponent(spec.template!)}`}>«{spec.title}»</Link> : `«${spec.title}»`}
                </span>
              ) : (
                <span>Состав из ролей</span>
              ))}
            {spec && <span>{spec.mail === "flow" ? "почта по связям шаблона" : `почта: ${MAIL_TITLE[spec.mail]}`}</span>}
            {team.worktree && (
              <span title={team.worktree.path}>
                ветка <span className="mono">{team.worktree.branch}</span>
              </span>
            )}
            {active ? <span>работает {timeAgo(team.created) === "сейчас" ? "меньше минуты" : timeAgo(team.created)}</span> : <span>{STOPPED[team.stopReason ?? ""] ?? "остановлена"}</span>}
            {active && status === "needs_owner" && <span className="amber">нужно решение владельца</span>}
          </p>
          <StageBars stage={stage} big amber={status === "needs_owner"} />
          <div className="stage-labels d-only">
            {STAGES.map((s, i) => (
              <span key={s} className={i === stage - 1 ? "on" : ""}>
                {s}
              </span>
            ))}
          </div>
        </div>

        <TeamScheme
          team={team}
          open={scheme}
          onToggle={() => {
            writePref("genie.teamScheme", scheme ? "closed" : "open");
            setScheme(!scheme);
          }}
        />

        <div className="chat" ref={chatRef} role="log" aria-label="Почта команды" onScroll={(e) => (stick.current = e.currentTarget.scrollHeight - e.currentTarget.scrollTop - e.currentTarget.clientHeight < 60)}>
          <div className="inner">
            {entries.map((m, i) => {
              const day = dayLabel(m.at);
              const showDay = day !== lastDay;
              lastDay = day;
              const prev = entries[i - 1];
              const next = entries[i + 1];
              const same = (a?: Entry) => !!a && a.kind === m.kind && a.from === m.from && a.to.join() === m.to.join() && a.level === m.level && a.intent === m.intent && dayLabel(a.at) === day;
              const cont = same(prev) && !showDay;
              return (
                <Fragment key={m.key}>
                  {showDay && <div className="day">{day}</div>}
                  {m.kind === "system" ? (
                    <div className="sys">
                      <span />
                      {m.title ? (
                        <span className="body">
                          {m.title}: {m.to.map(displayName).join(", ")}
                        </span>
                      ) : (
                        <span className="body">
                          <b>{m.fromRole === "orchestrator" ? "Оркестратор" : displayName(m.from)} → {recipients(m.to, team)}:</b> {m.text}
                        </span>
                      )}
                      <span className="t">{clock(m.at)}</span>
                    </div>
                  ) : (
                    <article className={`mail${cont ? " cont" : ""}${m.urgent ? " urgent" : ""}${m.kind === "mine" ? " mine" : ""}`}>
                      <span className="slot">{!cont && <Avatar role={m.kind === "mine" ? "human" : m.fromRole} name={m.from} size="md" />}</span>
                      <div className="body">
                        {!cont && (
                          <div className="meta">
                            <b title={m.kind === "agent" && m.fromRole !== "orchestrator" ? ROLE_TITLE_RU[m.fromRole] : undefined}>
                              {m.kind === "mine" ? "Вы" : m.fromRole === "orchestrator" ? "Оркестратор" : displayName(m.from)}
                            </b>
                            <span className="muted">→ {recipients(m.to, team)}</span>
                            <MailBadges level={m.level} intent={m.intent} />
                          </div>
                        )}
                        <div className="text">
                          <MessageText text={m.text} team={team.id} />
                        </div>
                        {m.kind === "mine" && !same(next) && <span className="receipt">{m.delivered ? "получено" : "ещё не прочитано"}</span>}
                      </div>
                      <span className="t">{clock(m.at)}</span>
                    </article>
                  )}
                </Fragment>
              );
            })}
            {!entries.length && <div className="empty">Писем пока нет</div>}
            {active && working.length > 0 && (
              <div className="typing" aria-live="polite">
                <span className="spin" />
                {working.map((w) => displayName(w.name)).join(", ")} {working.length > 1 ? "работают" : "работает"}
              </div>
            )}
          </div>
        </div>

        {active && (
          <form
            className="team-composer"
            onSubmit={(e) => {
              e.preventDefault();
              submit();
            }}
          >
            <label className="tc-to">
              Кому
              <select value={to} onChange={(e) => setTo(e.target.value)}>
                <option value="all">всем</option>
                {team.members.map((m) => (
                  <option key={m.name} value={m.name}>
                    {displayName(m.name)} — {ROLE_TITLE_RU[m.role] ?? m.role}
                  </option>
                ))}
                <option value="orchestrator">оркестратору</option>
              </select>
            </label>
            <select className="tc-level" aria-label="Важность" title="Важность: прервать шаг — остановить текущий шаг агента (даже долгую команду) и передать сообщение первым" value={level} onChange={(e) => setLevel(e.target.value as MailLevel)}>
              {(["low", "normal", "high", "interrupt"] as MailLevel[])
                .filter((l) => l !== "interrupt" || (to !== "all" && to !== "orchestrator"))
                .map((l) => (
                  <option key={l} value={l}>
                    {LEVEL_LABEL[l]}
                  </option>
                ))}
            </select>
            <textarea
              aria-label="Сообщение"
              rows={1}
              value={draft}
              placeholder={to === "all" ? "Сообщение всей команде" : to === "orchestrator" ? "Сообщение оркестратору" : `Сообщение для ${displayName(to)}`}
              onChange={(e) => {
                setDraft(e.target.value);
                e.target.style.height = "auto";
                e.target.style.height = `${Math.min(140, e.target.scrollHeight)}px`;
              }}
              onKeyDown={(e) => e.key === "Enter" && (e.metaKey || e.ctrlKey) && (e.preventDefault(), submit())}
            />
            <button type="submit" className="send" aria-label="Отправить" title="Отправить (⌘↵)" disabled={!draft.trim() || send.isPending}>
              <Icon.send size={15} style={{ color: "#fff" }} />
            </button>
          </form>
        )}
      </main>

      <aside className="team-aside" aria-label="Участники и журнал">
        <div className="hd">Участники</div>
        {team.templateChanged && (
          <div className="aside-note">
            <b>Шаблон изменили после запуска</b>
            <span>Команда работает по снимку, взятому при старте. Новые настройки роли участник получит после перезапуска.</span>
          </div>
        )}
        <div className="members">
          {team.members.map((m) => (
            <MemberCard key={m.name} team={team} member={m} live={live[keyOf(m.name)]} session={active ? team.sessions?.[m.name] : undefined} />
          ))}
        </div>
        <div className="nav-section" style={{ margin: "14px 18px 8px" }}>
          Журнал
        </div>
        <ol className="events">
          {[...team.log]
            .filter((e) => e.event !== "mail" && !(e.event === "status" && e.status === "starting"))
            .reverse()
            .slice(0, 40)
            .map((e, i) => (
              <li key={i}>
                <span className="t">{clock(e.at)}</span>
                <span>{(EVENT_TEXT[e.event] ?? (() => e.event))(e)}</span>
              </li>
            ))}
        </ol>
      </aside>
    </>
  );
}

/** A member: who it is, what it does now, what it said last. */
function MemberCard({ team, member: m, live, session: s }: { team: TeamDetail; member: TeamDetail["members"][number]; live?: { state: LiveState; note?: string }; session?: LiveSession }) {
  useTick();
  const active = team.state === "active";
  const paused = active && m.state === "paused";
  const state: LiveState = !active || m.state === "stopped" ? "stopped" : paused ? "waiting" : m.activity === "error" || m.state === "error" ? "error" : m.activity === "working" ? "working" : (live?.state ?? "idle");
  const text =
    state === "working"
      ? s?.tool
        ? `работает · ${s.tool.name} ${timeAgo(s.tool.since)}`
        : "работает"
      : paused
        ? "на паузе"
        : state === "waiting"
          ? (live?.note ?? "ждёт")
        : state === "error"
          ? "ошибка"
          : state === "stopped"
            ? "остановлен"
            : "свободен";
  const queued = team.pending[m.name] ?? 0;
  const status = m.status && m.status !== "starting" ? m.status : "";
  const said = state !== "working" && s?.lastText && s.lastText !== status ? s.lastText : "";
  return (
    <div className={`mcard ${state}`}>
      <Avatar role={m.role} name={m.name} activity={active ? m.activity : undefined} state={m.state} size="md" />
      <div className="info">
        <div className="nm">
          <Link to={`/team/${encodeURIComponent(team.id)}/${encodeURIComponent(m.name)}`} className="mcard-open" title="Открыть разговор с агентом">
            <b>{displayName(m.name)}</b>
          </Link>
          <span className="muted">{ROLE_TITLE_RU[m.role] ?? m.role}</span>
          {active && (
            <span className="acts">
              <RestartMemberButton team={team.id} name={m.name} />
              <RemoveMemberButton team={team.id} name={m.name} role={m.role} />
            </span>
          )}
        </div>
        <div className="state">
          {state === "working" && <span className="spin" />}
          <span>{text}</span>
          {queued > 0 && (
            <span className="muted" title={`${queued} ${plural(queued, "письмо ждёт", "письма ждут", "писем ждут")} доставки`}>
              · ✉ {queued}
            </span>
          )}
        </div>
        {s?.tool && (
          <div className="tool" title={s.tool.args}>
            {s.tool.args}
          </div>
        )}
        {status && <div className="st">{status}</div>}
        {said && (
          <div className="said" title={said}>
            «{said}»
          </div>
        )}
        {m.model && (
          <div className="model">
            {m.model.replace(/^[^/]+\//, "")}
            {m.thinking ? ` · ${m.thinking}` : ""}
          </div>
        )}
      </div>
      <span className="when">{timeAgo(m.activityAt ?? m.statusAt)}</span>
    </div>
  );
}
