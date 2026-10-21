import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Link, Navigate, useParams } from "react-router";
import { type LiveState, liveTeam, MAIL_TITLE, useAgentConfig } from "@/entities/agent-config";
import { displayName, ROLE_TITLE_RU } from "@/entities/member";
import { RemoveMemberButton, RestartMemberButton, TeamActions } from "@/features/manage-team";
import { StageBars, STAGES, stageOf, STATUS_NAME } from "@/entities/task";
import { type LiveSession, type Mail, type MailLevel, MessageText, memberActivity, PartyAvatar, type TeamDetail, useSendMail, useTeam } from "@/entities/team";
import { useOnline } from "@/shared/api";
import { clock, dayLabel, plural, readPref, timeAgo, useTick, writePref } from "@/shared/lib";
import { Button, Character, characterState, ErrorState, Icon, IconButton, Select, Segmented, Sheet, Skeleton, Tag, Textarea, useToast, WorkingArcs } from "@/shared/ui";
import { TeamScheme } from "./TeamScheme.tsx";
import "./team-view-sheets.css";

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
      {(level === "high" || level === "interrupt") && <Tag tone="signal">{level === "interrupt" ? "прерывание" : "важное"}</Tag>}
      {intent && <Tag tone="outline">{INTENT_LABEL[intent]}</Tag>}
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

/**
 * A task's team: its stages, scheme, mail and members. It lives on the task page's
 * team route and opens from the rail; `/team/:teamId` only redirects there.
 */
export function TeamView({ teamId: embedded }: { teamId?: string }) {
  useTick(15_000);
  const params = useParams();
  const teamId = embedded ?? params.teamId;
  const q = useTeam(teamId);
  const cfg = useAgentConfig().data;
  const send = useSendMail();
  const online = useOnline();
  const toast = useToast();
  const [to, setTo] = useState("all");
  const [level, setLevel] = useState<MailLevel>("normal");
  const [draft, setDraft] = useState("");
  // On a phone the members and the journal open as a bottom sheet.
  const [sheet, setSheet] = useState(false);
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

  if (q.isPending) return <TeamSkeleton />;
  if (q.isError || !team) return <div className="tv-page-state"><ErrorState message={q.error?.message ?? "Команда не найдена"} onRetry={() => void q.refetch()} /></div>;
  if (!embedded) return <Navigate to={`/task/${encodeURIComponent(team.task)}/team`} replace />;

  const status = team.taskInfo?.status;
  const stage = status ? stageOf(status) : 0;
  const working = team.members.filter((m) => m.activity === "working");
  const active = team.state === "active";
  const spec = team.spec;
  const live = spec ? liveTeam(spec, team.members, status ?? "ready", active, displayName).live : {};
  const keyOf = (name: string) => spec?.members.find((m) => m.name === name)?.key ?? name;
  const templateKnown = !!spec?.template && !!cfg?.teams.some((t) => t.id === spec.template);
  // The character reports the team: it thinks while someone works, is surprised when the owner is needed.
  const pose = characterState({ working: active ? working.length : 0, waiting: active && status === "needs_owner" ? 1 : 0 });
  const offline = !online;

  const submit = () => {
    const text = draft.trim();
    if (!text) return;
    stick.current = true;
    send.mutate({ team: team.id, to, text, level }, { onSuccess: () => setDraft(""), onError: (e) => toast(`Не отправлено: ${e.message}`, "error") });
  };

  let lastDay = "";
  const levels = (["low", "normal", "high", "interrupt"] as MailLevel[]).filter((l) => l !== "interrupt" || (to !== "all" && to !== "orchestrator"));

  return (
    <div className="task-team tv">
      <section className="tv-main" aria-label="Команда">
        <div className="tv-head">
          <div className="tv-meta">
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
                ветка <span className="tv-mono">{team.worktree.branch}</span>
              </span>
            )}
            {active ? <span>работает {timeAgo(team.created) === "сейчас" ? "меньше минуты" : timeAgo(team.created)}</span> : <span>{STOPPED[team.stopReason ?? ""] ?? "остановлена"}</span>}
            {active && status === "needs_owner" && <Tag tone="signal">нужно решение владельца</Tag>}
            <i className="grow" />
            <span className="tv-acts">
              <Button variant="ghost" sm className="m-only" aria-haspopup="dialog" onClick={() => setSheet(true)}>
                Участники · {team.members.length}
              </Button>
              <TeamActions team={team} />
            </span>
          </div>
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

        <div className="tv-log" ref={chatRef} role="log" aria-label="Почта команды" onScroll={(e) => (stick.current = e.currentTarget.scrollHeight - e.currentTarget.scrollTop - e.currentTarget.clientHeight < 60)}>
          <div className="tv-inner">
            {entries.map((m, i) => {
              const day = dayLabel(m.at);
              const showDay = day !== lastDay;
              lastDay = day;
              const prev = entries[i - 1];
              const next = entries[i + 1];
              const same = (a?: Entry) => !!a && a.kind === m.kind && a.from === m.from && a.to.join() === m.to.join() && a.level === m.level && a.intent === m.intent && dayLabel(a.at) === day;
              const cont = same(prev) && !showDay;
              const body = (
                <>
                  {!cont && (
                    <div className="tv-from">
                      <b title={m.kind === "agent" && m.fromRole !== "orchestrator" ? ROLE_TITLE_RU[m.fromRole] : undefined}>
                        {m.kind === "mine" ? "Вы" : m.fromRole === "orchestrator" ? "Оркестратор" : displayName(m.from)}
                      </b>
                      <span className="tv-to-line">→ {recipients(m.to, team)}</span>
                      <MailBadges level={m.level} intent={m.intent} />
                    </div>
                  )}
                  <div className="tv-text">
                    <MessageText text={m.text} team={team.id} />
                  </div>
                </>
              );
              return (
                <Fragment key={m.key}>
                  {showDay && <div className="tv-day">{day}</div>}
                  {m.kind === "system" ? (
                    <div className="tv-sys">
                      <span className="tv-body">
                        {m.title ? (
                          <>
                            {m.title}: {m.to.map(displayName).join(", ")}
                          </>
                        ) : (
                          <>
                            <b>{m.fromRole === "orchestrator" ? "Оркестратор" : displayName(m.from)} → {recipients(m.to, team)}:</b> {m.text}
                          </>
                        )}
                      </span>
                      <span className="tv-time">{clock(m.at)}</span>
                    </div>
                  ) : (
                    <article className={`tv-mail${cont ? " cont" : ""}${m.urgent ? " urgent" : ""}${m.kind === "mine" ? " mine" : ""}`}>
                      <span className="tv-slot">{!cont && <PartyAvatar role={m.kind === "mine" ? "human" : m.fromRole} name={m.kind === "mine" ? "Вы" : m.from} />}</span>
                      <div className="tv-body">
                        {m.kind === "mine" ? <Sheet className="tv-bubble">{body}</Sheet> : <div className="tv-bubble">{body}</div>}
                        {m.kind === "mine" && !same(next) && <span className="tv-receipt">{m.delivered ? "получено" : "ещё не прочитано"}</span>}
                      </div>
                      <span className="tv-time">{clock(m.at)}</span>
                    </article>
                  )}
                </Fragment>
              );
            })}
            {!entries.length && <p className="tv-quiet">Писем пока нет. Первое придёт, когда команда начнёт работу.</p>}
            {active && working.length > 0 && (
              <div className="tv-typing" aria-live="polite">
                <WorkingArcs size={14} label="Идёт работа" />
                {working.map((w) => displayName(w.name)).join(", ")} {working.length > 1 ? "работают" : "работает"}
              </div>
            )}
          </div>
        </div>

        {active && (
          <form
            className="tv-composer"
            onSubmit={(e) => {
              e.preventDefault();
              submit();
            }}
          >
            <div className="tv-composer-opts">
              <label className="tv-to">
                Кому
                <Select sm value={to} onChange={(e) => setTo(e.target.value)}>
                  <option value="all">всем</option>
                  {team.members.map((m) => (
                    <option key={m.name} value={m.name}>
                      {displayName(m.name)} — {ROLE_TITLE_RU[m.role] ?? m.role}
                    </option>
                  ))}
                  <option value="orchestrator">оркестратору</option>
                </Select>
              </label>
              <span title="«Прервать шаг» остановит текущую команду">
                <Segmented label="Важность" value={level} onChange={setLevel} items={levels.map((l) => ({ id: l, label: LEVEL_LABEL[l] }))} />
              </span>
            </div>
            <div className="tv-composer-row">
              <Textarea
                className="tv-input"
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
              <IconButton type="submit" label="Отправить" title={offline ? "Нет связи с сервером" : "Отправить (⌘↵)"} className="tv-send" disabled={!draft.trim() || send.isPending || offline}>
                <Icon.send size={15} />
              </IconButton>
            </div>
          </form>
        )}
      </section>

      {sheet && <button type="button" className="g-scrim tv-scrim m-only" aria-label="Закрыть список участников" onClick={() => setSheet(false)} />}
      <aside className={`tv-aside${sheet ? " open" : ""}`} aria-label="Участники и журнал">
        <div className="tv-aside-head">
          <Character state={pose} size={44} />
          <span>Участники</span>
          <IconButton label="Закрыть" className="m-only" onClick={() => setSheet(false)}>
            <Icon.close size={14} />
          </IconButton>
        </div>
        {team.templateChanged && (
          <div className="tv-note">
            <b>Шаблон изменили после запуска</b>
            <span>Команда работает по снимку, взятому при старте. Новые настройки роли участник получит после перезапуска.</span>
          </div>
        )}
        <div className="tv-members">
          {team.members.map((m) => (
            <MemberCard key={m.name} team={team} member={m} live={live[keyOf(m.name)]} session={active ? team.sessions?.[m.name] : undefined} />
          ))}
        </div>
        <h3 className="tv-cap">Журнал</h3>
        <ol className="tv-events">
          {[...team.log]
            .filter((e) => e.event !== "mail" && !(e.event === "status" && e.status === "starting"))
            .reverse()
            .slice(0, 40)
            .map((e, i) => (
              <li key={i}>
                <span className="tv-time">{clock(e.at)}</span>
                <span>{(EVENT_TEXT[e.event] ?? (() => e.event))(e)}</span>
              </li>
            ))}
        </ol>
      </aside>
    </div>
  );
}

/** The team page while it loads: the head, a few messages and the composer as placeholders. */
function TeamSkeleton() {
  return (
    <div className="task-team tv" aria-busy="true" aria-label="Команда загружается">
      <section className="tv-main">
        <div className="tv-head">
          <Skeleton width="40%" height={16} />
          <Skeleton height={8} />
        </div>
        <div className="tv-log">
          <div className="tv-inner">
            {[72, 48, 64].map((w, i) => (
              <div key={i} className="tv-mail">
                <Skeleton width={28} height={28} />
                <Skeleton width={`${w}%`} height={44} />
              </div>
            ))}
          </div>
        </div>
      </section>
      <aside className="tv-aside">
        <div className="tv-members">
          {[0, 1, 2].map((i) => (
            <Skeleton key={i} height={64} />
          ))}
        </div>
      </aside>
    </div>
  );
}

/** A member: who it is, what it does now, what it said last. */
function MemberCard({ team, member: m, live, session: s }: { team: TeamDetail; member: TeamDetail["members"][number]; live?: { state: LiveState; note?: string }; session?: LiveSession }) {
  useTick();
  const active = team.state === "active";
  const { state, text } = memberActivity(team, m, live, s);
  const queued = team.pending[m.name] ?? 0;
  const status = m.status && m.status !== "starting" ? m.status : "";
  const said = state !== "working" && s?.lastText && s.lastText !== status ? s.lastText : "";
  return (
    <div className={`tv-member ${state}`}>
      <PartyAvatar role={m.role} name={m.name} activity={active ? m.activity : undefined} state={m.state} />
      <div className="tv-info">
        <div className="tv-nm">
          <Link to={`/team/${encodeURIComponent(team.id)}/${encodeURIComponent(m.name)}`} className="tv-open" title="Открыть разговор с агентом">
            <b>{displayName(m.name)}</b>
          </Link>
          <span className="tv-role">{ROLE_TITLE_RU[m.role] ?? m.role}</span>
          {active && (
            <span className="tv-member-acts">
              <RestartMemberButton team={team.id} name={m.name} />
              <RemoveMemberButton team={team.id} name={m.name} role={m.role} />
            </span>
          )}
        </div>
        <div className="tv-state">
          {state === "working" && <WorkingArcs size={13} label="Работает" />}
          <span>{text}</span>
          {queued > 0 && (
            <span title={`${queued} ${plural(queued, "письмо ждёт", "письма ждут", "писем ждут")} доставки`}>· ✉ {queued}</span>
          )}
        </div>
        {s?.tool && (
          <div className="tv-tool" title={s.tool.args}>
            {s.tool.args}
          </div>
        )}
        {status && <div className="tv-status">{status}</div>}
        {said && (
          <div className="tv-said" title={said}>
            «{said}»
          </div>
        )}
        {m.model && (
          <div className="tv-model">
            {m.model.replace(/^[^/]+\//, "")}
            {m.thinking ? ` · ${m.thinking}` : ""}
          </div>
        )}
      </div>
      <span className="tv-when">{timeAgo(m.activityAt ?? m.statusAt)}</span>
    </div>
  );
}
