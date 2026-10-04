import { Fragment, type ReactNode, useEffect, useState } from "react";
import { Link, useNavigate } from "react-router";
import { DocDiagBadge, DocStaleBadge, DocStatusBadge } from "@/entities/doc";
import { Avatar, Avatars } from "@/entities/member";
import type { DocsImpact, DocsImpactReason } from "@/shared/api";
import {
  ArtifactThumb,
  EpicIcon,
  EpicProgress,
  historyText,
  Labels,
  PRIORITY_NAME,
  PriorityIcon,
  type Status,
  STATUS_NAME,
  StatusIcon,
  statusNote,
  type Task,
  useArtifactViewer,
  useCheck,
  useComment,
  useDeleteTask,
  useDocsImpact,
  useEpicMap,
  useMoveTask,
  usePatchTask,
  useTask,
  useTasks,
} from "@/entities/task";
import { useAgentConfig } from "@/entities/agent-config";
import { PersonAvatar, responsibleChoices, useMembers } from "@/entities/project";
import { useSession } from "@/entities/session";
import type { Team } from "@/entities/team";
import { SpendText, useTaskUsage } from "@/entities/usage";
import { IDEA_TEMPLATE } from "@/features/shape-idea";
import { OwnerDecision } from "@/features/owner-decision";
import { SpawnTeamDialog } from "@/features/spawn-team";
import { plural, timeAgo, useTick } from "@/shared/lib";
import { ConfirmDialog, Icon, Markdown, useToast } from "@/shared/ui";
import { TaskDelivery } from "./TaskDelivery.tsx";

const KIND_NAME: Record<string, string> = { note: "заметка", progress: "прогресс", question: "вопрос", decision: "решение", review: "ревью", handoff: "передача", owner: "владелец" };

export type TaskTab = "about" | "team";

/**
 * A task: the preview beside a list or board (`variant="panel"`), or its own page with
 * tabs (`variant="page"`) — the description with every property, and the team.
 */
export function TaskDetail({ id, team, onClose, variant = "panel", tab = "about", teamPane }: {
  id: string;
  team?: Team;
  onClose: () => void;
  variant?: "panel" | "page";
  tab?: TaskTab;
  /** The team tab's content; the page passes it in so this widget stays below the pages. */
  teamPane?: ReactNode;
}) {
  useTick();
  const q = useTask(id);
  const toast = useToast();
  const move = useMoveTask();
  const patch = usePatchTask();
  const comment = useComment();
  const check = useCheck();
  const del = useDeleteTask();
  const [deleting, setDeleting] = useState(false);
  const [draft, setDraft] = useState("");
  const [editDesc, setEditDesc] = useState<string | undefined>();
  const [spawning, setSpawning] = useState(false);
  const agents = useAgentConfig();
  const epics = useEpicMap();
  const impactEnabled = q.data?.status === "review" || q.data?.status === "done";
  const impact = useDocsImpact(id, impactEnabled);
  const session = useSession().data;
  const me = session?.mode === "users" ? session.user.login : undefined;
  const myRole = session?.projects.find((p) => p.slug === session.project)?.role;
  const canDelete = myRole === "admin" || myRole === "owner";
  const members = useMembers(me ? session?.project : undefined).data;

  useEffect(() => {
    setDraft("");
    setEditDesc(undefined);
    setDeleting(false);
  }, [id]);

  const fail = (e: Error) => toast(`Не удалось: ${e.message}`, "error");
  const viewer = useArtifactViewer(fail);

  const Shell = variant === "page" ? "main" : "aside";
  if (q.isPending) return <Shell className={variant === "page" ? "main" : "detail"}><div className="empty">Загрузка…</div></Shell>;
  if (q.isError) return <Shell className={variant === "page" ? "main" : "detail"}><div className="empty">{q.error.message}</div></Shell>;
  const t: Task = q.data;
  const isEpic = t.type === "epic";
  const canSpawn = agents.isSuccess && !isEpic && team?.state !== "active" && !["done", "cancelled"].includes(t.status);
  const epic = t.parent ? epics.get(t.parent) : undefined;
  const epicChoices = [...epics.values()].filter((e) => e.id === t.parent || (e.status !== "done" && e.status !== "cancelled"));
  const people = responsibleChoices(members ?? [], t.assignee);
  const responsibleName = members?.find((m) => m.user.login === t.assignee)?.user.name;
  const assign = (login: string) =>
    patch.mutate(
      { id: t.id, patch: { assignee: login || null } },
      { onSuccess: () => toast(login ? `Ответственный за ${t.id}: @${login}` : `У ${t.id} больше нет ответственного`), onError: fail },
    );

  const setStatus = (status: Status, note?: string) =>
    move.mutate({ id: t.id, status, note }, { onSuccess: () => toast(`${t.id} → ${STATUS_NAME[status]} · оркестратор уведомлён`), onError: fail });

  // A status change's note is also a comment (`[in_progress → review] …`): the history
  // line already says it, so the comment is left out of the activity.
  const moves = new Set(t.history.filter((h) => h.event === "status").map((h) => `${h.from}>${h.to}`));
  const activity = [
    ...t.history.map((h) => ({ kind: "event" as const, at: h.at, h })),
    ...t.comments
      .filter((c) => {
        const n = statusNote(c.text);
        return !(n && moves.has(`${n.from}>${n.to}`));
      })
      .map((c) => ({ kind: "comment" as const, at: c.at, c })),
  ].sort((a, b) => a.at.localeCompare(b.at));

  const page = variant === "page";
  const pageLink = `/task/${encodeURIComponent(t.id)}`;
  const last = activity.at(-1);

  const title = (
    <h2
      className="title"
      contentEditable
      suppressContentEditableWarning
      spellCheck={false}
      onBlur={(e) => {
        const title = e.currentTarget.textContent?.trim() ?? "";
        if (title && title !== t.title) patch.mutate({ id: t.id, patch: { title } }, { onError: fail });
      }}
      onKeyDown={(e) => e.key === "Enter" && (e.preventDefault(), e.currentTarget.blur())}
    >
      {t.title}
    </h2>
  );

  // The page shows every property in its rail; the preview only what a glance needs.
  const props = (
    <div className={page ? "props rail" : "props"}>
      <span className="k">Статус</span>
      <span style={{ display: "flex", alignItems: "center", gap: 7 }}>
        <StatusIcon status={t.status} size={13} />
        <select aria-label="Статус" value={t.status} onChange={(e) => setStatus(e.target.value as Status)}>
          {(Object.keys(STATUS_NAME) as Status[]).map((s) => (
            <option key={s} value={s}>
              {STATUS_NAME[s]}
            </option>
          ))}
        </select>
      </span>
      <span className="k">Приоритет</span>
      <span style={{ display: "flex", alignItems: "center", gap: 7 }}>
        <PriorityIcon priority={t.priority} size={13} />
        <select aria-label="Приоритет" value={t.priority} onChange={(e) => patch.mutate({ id: t.id, patch: { priority: Number(e.target.value) } }, { onError: fail })}>
          {PRIORITY_NAME.map((p, i) => (
            <option key={p} value={i}>
              {p}
            </option>
          ))}
        </select>
      </span>
      {people.length > 0 && (
        <>
          <span className="k">Ответственный</span>
          <span className="wide" style={{ display: "flex", alignItems: "center", gap: 7, minWidth: 0 }}>
            {t.assignee && <PersonAvatar login={t.assignee} name={responsibleName} prefix="" />}
            <select aria-label="Ответственный" value={t.assignee ?? ""} onChange={(e) => assign(e.target.value)} title="Человек, который отвечает за задачу: ему приходят вопросы агентов">
              <option value="">Не назначен</option>
              {people.map((p) => (
                <option key={p.login} value={p.login}>
                  {p.login === me ? `${p.label} — я` : p.label}
                </option>
              ))}
            </select>
          </span>
        </>
      )}
      <span className="k">Команда</span>
      <span className="wide team-cell">
        {team ? (
          <>
            <Avatars members={team.members} max={6} />
            <Link to={page ? `${pageLink}/team` : teamLink(team)}>{team.state === "active" ? workingText(team.members.filter((m) => m.activity === "working").length) : "остановлена"}</Link>
          </>
        ) : (
          !canSpawn && <span className="muted">не собрана</span>
        )}
        {canSpawn && (
          <button type="button" className="btn ghost" style={{ height: 26 }} onClick={() => setSpawning(true)}>
            <Icon.userPlus size={12} />
            Собрать команду
          </button>
        )}
      </span>
      {!isEpic && (
        <>
          <span className="k">Эпик</span>
          <span className="wide" style={{ display: "flex", alignItems: "center", gap: 7, minWidth: 0 }}>
            <EpicIcon size={13} empty={!epic} />
            <select
              aria-label="Эпик"
              value={epic ? epic.id : ""}
              onChange={(e) =>
                patch.mutate(
                  { id: t.id, patch: { parent: e.target.value || null } },
                  { onSuccess: () => toast(e.target.value ? `${t.id} теперь в эпике ${e.target.value}` : `${t.id} больше не в эпике`), onError: fail },
                )
              }
            >
              <option value="">Без эпика</option>
              {epicChoices.map((e) => (
                <option key={e.id} value={e.id}>
                  {e.id} · {e.title}
                </option>
              ))}
            </select>
          </span>
        </>
      )}
      {(t.deps.length > 0 || (!isEpic && t.children.length > 0)) && (
        <>
          <span className="k">Связи</span>
          <span className="wide muted">
            {t.deps.length > 0 && (
              <>
                зависит от <TaskLinks ids={t.deps} />
              </>
            )}
            {t.deps.length > 0 && !isEpic && t.children.length > 0 && " · "}
            {!isEpic && t.children.length > 0 && (
              <>
                подзадачи <TaskLinks ids={t.children} />
              </>
            )}
          </span>
        </>
      )}
      {(page || t.labels.length > 0) && (
        <>
          <span className="k">Метки</span>
          <span className="wide">{t.labels.length ? <Labels labels={t.labels} /> : <span className="muted">нет</span>}</span>
        </>
      )}
      {page && (
        <>
          <span className="k">Интеграция</span>
          <span className="wide">
            <input
              key={`${t.id}-merge-${t.mergeStrategy}`}
              aria-label="Интеграция"
              title="Как результат попадёт в систему: договоритесь с оркестратором"
              defaultValue={t.mergeStrategy}
              placeholder="не задана"
              onBlur={(e) => e.target.value !== t.mergeStrategy && patch.mutate({ id: t.id, patch: { mergeStrategy: e.target.value } }, { onError: fail })}
            />
          </span>
          {t.worktree && (
            <>
              <span className="k">Ветка</span>
              <span className="wide mono" style={{ fontSize: 12, overflow: "hidden", textOverflow: "ellipsis" }} title={t.worktree.path}>
                {t.worktree.branch ?? t.worktree.path}
              </span>
            </>
          )}
          <TaskSpend id={t.id} epic={isEpic} />
        </>
      )}
    </div>
  );

  const description = (
    <section className="sec">
      <h3>
        {isEpic ? "Цель" : "Описание"}
        {editDesc === undefined ? (
          <button type="button" className="btn ghost" style={{ height: 24 }} onClick={() => setEditDesc(t.description)}>
            Изменить
          </button>
        ) : null}
      </h3>
      {editDesc === undefined ? (
        <Markdown text={t.description} empty="Описания нет" />
      ) : (
        <div className="composer">
          <textarea aria-label="Описание" rows={8} value={editDesc} onChange={(e) => setEditDesc(e.target.value)} />
          <div className="bar">
            markdown
            <span className="grow" />
            <button type="button" className="btn ghost" onClick={() => setEditDesc(undefined)}>
              Отмена
            </button>
            <button
              type="button"
              className="btn primary"
              onClick={() => patch.mutate({ id: t.id, patch: { description: editDesc } }, { onSuccess: () => setEditDesc(undefined), onError: fail })}
            >
              Сохранить
            </button>
          </div>
        </div>
      )}
    </section>
  );

  const criteria = (
    <section className="sec">
      <h3>
        {isEpic ? "Критерии успеха" : "Критерии приёмки"}
        <span className="n">
          {t.acceptance.filter((a) => a.done).length} из {t.acceptance.length}
        </span>
      </h3>
      {t.acceptance.length ? (
        <div className="criteria">
          {t.acceptance.map((a) => (
            <label key={a.id} className={a.done ? "done" : ""}>
              <input type="checkbox" checked={a.done} onChange={(e) => check.mutate({ id: t.id, n: a.id, done: e.target.checked }, { onError: fail })} />
              <span className="t">{a.text}</span>
              {a.checkedBy && <span className="by">{a.checkedBy}</span>}
            </label>
          ))}
        </div>
      ) : (
        <span className="muted">Оркестратор сформулирует критерии при уточнении</span>
      )}
    </section>
  );

  const epicLink = isEpic && (
    <Link className="epic-box link" to={`/epic/${encodeURIComponent(t.id)}`}>
      <EpicIcon />
      Это эпик: его задачи, прогресс и общие артефакты — на странице эпика
      <Icon.chevron size={12} style={{ marginLeft: "auto" }} />
    </Link>
  );

  const dialogs = (
    <>
      {viewer.modal}
      {spawning && <SpawnTeamDialog task={t} onClose={() => setSpawning(false)} />}
      {deleting && (
        <ConfirmDialog
          title={`Удалить ${t.id} навсегда?`}
          confirmLabel="Удалить"
          danger
          busy={del.isPending}
          onClose={() => setDeleting(false)}
          onConfirm={() =>
            del.mutate(
              { id: t.id, cascade: t.children.length > 0 },
              {
                onSuccess: (r) => {
                  toast(r.deleted.length > 1 ? `Удалено задач: ${r.deleted.length}` : `${t.id} удалена`);
                  setDeleting(false);
                  onClose();
                },
                onError: fail,
              },
            )
          }
        >
          «{t.title}» исчезнет вместе с комментариями, артефактами и историей — восстановить не получится.
          {t.children.length > 0 && ` Подзадачи (${t.children.length}) будут удалены вместе с ней.`}
          {t.team && " Команда, которая над ней работает, будет остановлена и удалена (ветка git останется)."}
        </ConfirmDialog>
      )}
    </>
  );

  const crumbs = (
    <span className="crumbs">
      {page && (
        <>
          <Link to="/tasks" className="d-only">
            Задачи
          </Link>
          <span className="d-only"> / </span>
        </>
      )}
      {t.parent && (
        <>
          {epic ? <Link to={`/epic/${encodeURIComponent(t.parent)}`}>{t.parent}</Link> : t.parent}
          {" / "}
        </>
      )}
      <span className="here">{t.id}</span>
    </span>
  );

  if (!page)
    return (
      <aside className="detail" aria-label={`Задача ${t.id}`}>
        <header className="topbar">
          <button type="button" className="icon-btn m-only" onClick={onClose} aria-label="Назад">
            <Icon.back />
          </button>
          {crumbs}
          <span className="grow" />
          <Link className="btn" to={pageLink} title="Открыть страницу задачи с командой и активностью">
            <Icon.external size={12} />
            Открыть страницу
          </Link>
          <button type="button" className="icon-btn d-only" onClick={onClose} aria-label="Закрыть">
            <Icon.close />
          </button>
        </header>

        <div className="detail-body">
          <div className="detail-head">
            {title}
            {props}
          </div>
          {epicLink}
          <OwnerDecision task={t} />
          {description}
          {criteria}
          {!isEpic && <TaskDelivery task={t.id} />}
          <footer className="detail-more">
            {last && <span className="muted">Последнее: {last.kind === "event" ? `${last.h.actor} ${historyText(last.h, isEpic)}` : `${last.c.author}, ${KIND_NAME[last.c.kind] ?? last.c.kind}`} · {timeAgo(last.at)}</span>}
            <span className="grow" />
            <Link to={pageLink}>Вся задача и активность →</Link>
          </footer>
        </div>
        {dialogs}
      </aside>
    );

  return (
    <main className="main task-page" aria-label={`Задача ${t.id}`}>
      <header className="topbar">
        <Link to="/tasks" className="icon-btn m-only" aria-label="Назад">
          <Icon.back />
        </Link>
        {crumbs}
        <span className="grow" />
        {team?.template === IDEA_TEMPLATE && team.state === "active" && team.members[0] && (
          // An idea being shaped: back to the conversation with its planner.
          <Link to={teamLink(team)} style={{ fontSize: 12 }}>
            Разговор о плане
          </Link>
        )}
        {canDelete && (
          <button type="button" className="icon-btn" onClick={() => setDeleting(true)} aria-label="Удалить задачу" title="Удалить задачу">
            <Icon.trash />
          </button>
        )}
      </header>

      <div className="tp-head">
        {title}
        <nav className="tp-tabs" aria-label="Разделы задачи">
          <Link to={pageLink} className={tab === "about" ? "on" : ""} aria-current={tab === "about" ? "page" : undefined}>
            {isEpic ? "Цель" : "Описание"}
          </Link>
          {!isEpic && (
            <Link to={`${pageLink}/team`} className={tab === "team" ? "on" : ""} aria-current={tab === "team" ? "page" : undefined}>
              Команда
              <TeamState team={team} />
            </Link>
          )}
        </nav>
      </div>

      {tab === "team" && !isEpic ? (
        (teamPane ?? (
          <div className="tp-noteam">
            <Icon.userPlus size={18} />
            <b>Команда не собрана</b>
            <span className="muted">Соберите команду агентов: здесь появятся её почта, схема и участники.</span>
            {canSpawn && (
              <button type="button" className="btn primary" onClick={() => setSpawning(true)}>
                Собрать команду
              </button>
            )}
          </div>
        ))
      ) : (
        <div className="tp-body">
          <div className="tp-main">
            {epicLink}
            {impactEnabled && <DocsImpactBlock result={impact.data} />}
            <OwnerDecision task={t} />
            {description}
            {criteria}
            {!isEpic && <TaskDelivery task={t.id} />}
            {t.plan.trim() && (
              <section className="sec">
                <h3>{isEpic ? "Дорожная карта" : "План"}</h3>
                <Markdown text={t.plan} />
              </section>
            )}
            {t.notes.trim() && (
              <section className="sec notes">
                <h3>Заметки</h3>
                <Markdown text={readableStamps(t.notes)} />
              </section>
            )}
            {t.artifacts.length > 0 && (
              <section className="sec">
                <h3>
                  Артефакты <span className="n">{t.artifacts.length}</span>
                </h3>
                <div className="artifacts">
                  {t.artifacts.map((a) => (
                    <button type="button" key={a.id} className="artifact" onClick={() => viewer.show(t.id, a.id)}>
                      <ArtifactThumb task={t.id} artifact={a} />
                      <span className="nm">{a.name}</span>
                      <span className="who">
                        {a.kind} · {a.author}
                      </span>
                    </button>
                  ))}
                </div>
              </section>
            )}
            <section className="sec">
              <h3>Активность</h3>
              {activity.map((a) =>
                a.kind === "event" ? (
                  <div key={`h${a.at}${a.h.event}`} className="ev">
                    <i />
                    <span>
                      <span style={{ color: "var(--text-2)" }}>{a.h.actor}</span> {historyText(a.h, isEpic)} · {timeAgo(a.at)}
                    </span>
                  </div>
                ) : (
                  <div key={`c${a.c.id}`} className={`comment${a.c.role === "human" ? " mine" : ""}`}>
                    <div className="head">
                      <Avatar role={a.c.role} name={a.c.author} size="solo" />
                      <span style={{ fontWeight: 500 }}>{a.c.author}</span>
                      <span className="k">{KIND_NAME[a.c.kind] ?? a.c.kind}</span>
                      <span className="when">{timeAgo(a.c.at)}</span>
                    </div>
                    <Markdown text={a.c.text} />
                  </div>
                ),
              )}
              <div className="composer">
                <textarea
                  aria-label="Комментарий"
                  placeholder="Комментарий для оркестратора и команды"
                  value={draft}
                  onChange={(e) => setDraft(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && draft.trim()) comment.mutate({ id: t.id, text: draft.trim() }, { onSuccess: () => setDraft(""), onError: fail });
                  }}
                />
                <div className="bar">
                  ⌘↵ отправить · оркестратор получит уведомление
                  <span className="grow" />
                  <button
                    type="button"
                    className="btn primary"
                    disabled={!draft.trim() || comment.isPending}
                    onClick={() => comment.mutate({ id: t.id, text: draft.trim() }, { onSuccess: () => setDraft(""), onError: fail })}
                  >
                    Отправить
                  </button>
                </div>
              </div>
            </section>
          </div>
          <aside className="tp-rail" aria-label="Свойства задачи">
            {props}
            {epic && <EpicBox id={epic.id} onArtifact={viewer.show} />}
          </aside>
        </div>
      )}
      {dialogs}
    </main>
  );
}

/** Where a team opens: an idea being shaped goes straight to its planner's conversation. */
function teamLink(team: Team): string {
  if (team.template === IDEA_TEMPLATE && team.state === "active" && team.members[0]) return `/team/${encodeURIComponent(team.id)}/${encodeURIComponent(team.members[0].name)}`;
  return `/task/${encodeURIComponent(team.task)}/team`;
}

/** The team tab's quiet state: who works now, or that there is no team. */
function TeamState({ team }: { team?: Team }) {
  if (!team) return <span className="st">не собрана</span>;
  if (team.state !== "active") return <span className="st">остановлена</span>;
  const n = team.members.filter((m) => m.activity === "working").length;
  if (!n) return <span className="st">ждёт</span>;
  return (
    <span className="st on">
      <span className="spin" style={{ width: 8, height: 8 }} />
      {n} {plural(n, "работает", "работают", "работают")}
    </span>
  );
}

/** Task ids as links to their pages. */
function TaskLinks({ ids }: { ids: string[] }) {
  return (
    <>
      {ids.map((id, i) => (
        <Fragment key={id}>
          {i > 0 && ", "}
          <Link className="mono" to={`/task/${encodeURIComponent(id)}`}>
            {id}
          </Link>
        </Fragment>
      ))}
    </>
  );
}

/** Notes carry raw ISO stamps (`2026-10-03T14:32:58.935Z`): shown as a short local date and time. */
function readableStamps(text: string): string {
  return text.replace(/\b\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?Z\b/g, (iso) => {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return iso;
    return d.toLocaleString("ru-RU", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
  });
}

/** Russian phrasing of the structured impact reasons; matching stays server-side (G-12 lesson). */
function impactReasonText(reasons: DocsImpactReason[]): string {
  return reasons
    .map((reason) => (reason.kind === "changed-path" ? `меняет ${reason.path} — под paths: ${reason.pattern}` : `ссылается на задачу ${reason.id} в related`))
    .join("; ");
}

/**
 * Russian phrasing of the degradation notes (G-39 F2). The core keeps its note
 * vocabulary stable and English for the CLI and logs, so the only Russian surface
 * maps that fixed vocabulary here. An unmapped note falls back to a generic
 * Russian line, so no English text ever becomes visible in the UI.
 */
function impactNoteText(note: string): string {
  const mapping: [RegExp, (m: RegExpExecArray) => string][] = [
    [/^no team worktree for this task$/, () => "у задачи нет рабочей копии команды"],
    [/^worktree (.+) does not exist$/, (m) => `рабочая копия ${m[1]} не найдена`],
    [/^worktree (.+) is not a git working tree$/, (m) => `${m[1]} — не git-рабочая копия`],
    [/^worktree (.+) could not be inspected: .*$/, (m) => `рабочую копию ${m[1]} не удалось проверить`],
    [/^no base commit recorded for the team$/, () => "для команды не записан базовый коммит"],
    [/^base (.+) is not reachable from the worktree$/, (m) => `базовый коммит ${m[1]} недоступен из рабочей копии`],
    [/^no changes found between (.+) and HEAD$/, (m) => `между ${m[1]} и HEAD изменений не найдено`],
    [/^docs index unavailable: .*$/, () => "индекс документации недоступен"],
  ];
  for (const [pattern, render] of mapping) {
    const match = pattern.exec(note);
    if (match) return render(match);
  }
  return "не удалось получить данные об изменениях";
}

/** Mockup screen 9: pages the task's changes may have made stale. A hint, never a gate. */
function DocsImpactBlock({ result }: { result: DocsImpact | undefined }) {
  const navigate = useNavigate();
  if (!result) return null;
  const candidates = result.candidates;
  const note = result.notes[0];
  return (
    <section className="doc-impact" aria-label="Документация, которую могла затронуть задача">
      <div className="h">
        <Icon.file size={14} />
        Документация, которую могла затронуть задача
        {candidates.length > 0 && <span className="n">{candidates.length}</span>}
        <span className="hint">подсказка · не блокирует</span>
      </div>
      {candidates.length > 0 ? (
        <div className="rows">
          {candidates.map((candidate) => (
            <button
              type="button"
              key={candidate.path}
              className="row"
              onClick={() => navigate(`/docs?page=${encodeURIComponent(candidate.path)}`)}
            >
              {/*
                Screen 9: title + badges on the first line, the full reason on its
                own wrapping line, so the matched pattern is never cut off (G-39 F1).
                F5: `DocMarks` from `@/entities/doc` renders icon-only tree/search
                marks and always draws a status icon (including «актуальна»/«без
                статуса»), while the mockup wants text badges and only for
                draft/deprecated — so the block reuses that module's badge
                components instead of `DocMarks`.
              */}
              <span className="line">
                <Icon.file size={13} />
                <span className="nm">{candidate.title}</span>
                {(candidate.status === "draft" || candidate.status === "deprecated") && <DocStatusBadge status={candidate.status} />}
                {candidate.stale && <DocStaleBadge count={candidate.staleReasons.length} />}
                {candidate.diagnostics.length > 0 && <DocDiagBadge count={candidate.diagnostics.length} />}
              </span>
              <span className="why">{impactReasonText(candidate.reasons)}</span>
            </button>
          ))}
        </div>
      ) : (
        <div className="muted">Затронутой документации не найдено</div>
      )}
      {note && (
        <div className="why note">
          {result.changedPathsAvailable ? `замечание: ${impactNoteText(note)}` : `нет данных об изменениях: ${impactNoteText(note)}`}
        </div>
      )}
    </section>
  );
}


/** The epic a task belongs to, in the page's rail: its progress, goal and shared artifacts. */
function EpicBox({ id, onArtifact }: { id: string; onArtifact: (task: string, n: number) => void }) {
  const epic = useTask(id).data;
  const tasks = (useTasks().data ?? []).filter((x) => x.parent === id);
  if (!epic) return null;
  const goal = epic.description.trim().split(/\n\s*\n/)[0];
  const closed = tasks.filter((x) => x.status === "done" || x.status === "cancelled").length;
  // The same file attached twice shows once.
  const artifacts = epic.artifacts.filter((a, i, all) => all.findIndex((b) => b.name === a.name) === i);
  return (
    <section className="epic-box" aria-label={`Эпик ${epic.id}`}>
      <Link to={`/epic/${encodeURIComponent(epic.id)}`} className="h">
        <EpicIcon size={12} />
        <span className="kind mono">{epic.id}</span>
        <span className="nm">{epic.title}</span>
      </Link>
      {tasks.length > 0 && (
        <span className="mini">
          <EpicProgress tasks={tasks} compact />
          {closed} из {tasks.length} закрыто
        </span>
      )}
      {goal && <div className="goal">{goal}</div>}
      {artifacts.length > 0 && (
        <div className="files">
          {artifacts.map((a) => (
            <button type="button" key={a.id} className="artifact small" onClick={() => onArtifact(epic.id, a.id)} title={a.note ?? `${a.kind} · ${a.author}`}>
              <Icon.file size={13} />
              <span className="nm">{a.name}</span>
            </button>
          ))}
        </div>
      )}
    </section>
  );
}

/** How many members of a running team work right now, in words. */
function workingText(n: number): string {
  return n ? `${n} ${plural(n, "работает", "работают", "работают")}` : "сейчас никто не работает";
}

/** What the agents spent on the task (with its subtasks; an epic, with its tasks); nothing until they spend. */
function TaskSpend({ id, epic }: { id: string; epic: boolean }) {
  const usage = useTaskUsage(id).data;
  if (!usage?.spend.calls) return null;
  const tasks = usage.tasks.length;
  return (
    <>
      <span className="k">Расходы</span>
      <span className="wide">
        <SpendText spend={usage.spend} extra={tasks > 1 ? (epic ? `${tasks} ${plural(tasks, "задача", "задачи", "задач")}` : "вместе с подзадачами") : undefined} />
      </span>
    </>
  );
}
