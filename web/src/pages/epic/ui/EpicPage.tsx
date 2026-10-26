import { useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { Avatars } from "@/entities/member";
import { canWriteProject, useSession } from "@/entities/session";
import {
  ArtifactThumb,
  EpicIcon,
  EpicProgress,
  historyText,
  Labels,
  PRIORITY_NAME,
  PROGRESS,
  PriorityIcon,
  progressOf,
  type Status,
  STATUS_NAME,
  StatusIcon,
  type Task,
  type TaskSummary,
  useArtifactViewer,
  useCheck,
  useMoveTask,
  usePatchTask,
  useTask,
  useTasks,
} from "@/entities/task";
import { type Team, useTeamMap } from "@/entities/team";
import { moneyText, SpendText, type SpendItem, spendTitle, useTaskUsage } from "@/entities/usage";
import { AddArtifactDialog } from "@/features/add-artifact";
import type { NewTaskPreset } from "@/features/create-task";
import { timeAgo, useTick } from "@/shared/lib";
import { Banner, Button, EmptyState, ErrorState, GroupHead, Icon, IconButton, Markdown, Segments, Skeleton, Tag, useToast } from "@/shared/ui";
// The epic screen reuses the task-detail property blocks (`detail.css`) and the task page rail
// (`task-page.css`) under its own layout (`epics.css`), then the responsive and centring layers.
import "./epic-sheets.css";

export function EpicPage({ onNew }: { onNew: (preset?: NewTaskPreset) => void }) {
  useTick();
  const id = useParams().epicId ?? "";
  const q = useTask(id);
  const all = useTasks().data ?? [];
  const teams = useTeamMap();
  const usage = useTaskUsage(id || undefined).data;
  const session = useSession().data;
  const writable = canWriteProject(session);
  const move = useMoveTask();
  const toast = useToast();
  const fail = (e: Error) => toast(`Не удалось: ${e.message}`, "error");
  const viewer = useArtifactViewer(fail);
  const [adding, setAdding] = useState(false);

  if (q.isPending)
    return (
      <main className="main epic-page" aria-busy="true" aria-label="Эпик загружается">
        <div className="epic-skeleton">
          <Skeleton width={120} height={16} />
          <Skeleton width="60%" height={34} />
          <Skeleton width="90%" height={16} />
          <Skeleton width="80%" height={16} />
          <Skeleton height={140} />
        </div>
      </main>
    );
  if (q.isError)
    return (
      <main className="main epic-page">
        <div className="epic-state">
          <ErrorState message={q.error.message} onRetry={() => void q.refetch()} />
          <Link className="g-btn secondary sm" to="/epics">
            К эпикам
          </Link>
        </div>
      </main>
    );
  const epic = q.data;
  if (epic.type !== "epic") {
    return (
      <main className="main epic-page">
        <div className="epic-state">
          <EmptyState title={`${epic.id} — не эпик`} action={<Link className="g-btn secondary" to={`/tasks?task=${encodeURIComponent(epic.id)}`}>Открыть задачу</Link>}>
            Эпиками считаются крупные вехи; обычная задача живёт на своей странице.
          </EmptyState>
        </div>
      </main>
    );
  }
  const tasks = all.filter((t) => t.parent === epic.id);
  const done = tasks.filter((t) => progressOf(t.status) === "closed").length;
  const readyToClose = tasks.length > 0 && done === tasks.length && epic.status !== "done" && epic.status !== "cancelled";

  return (
    <main className="main epic-page">
      <header className="td-bar ep-bar">
        <Link to="/epics" className="td-link">
          Эпики
        </Link>
        <Icon.chevron size={12} />
        <span className="mono ep-id"><EpicIcon id={epic.id} size={12} /> {epic.id}</span>
        <span className="grow" />
        <Link className="g-btn secondary sm d-only" to={`/board?epic=${encodeURIComponent(epic.id)}`}>
          <Icon.board size={13} />
          Задачи эпика на доске
        </Link>
        <Button variant="primary" sm onClick={() => onNew({ epic: epic.id })}>
          <Icon.plus size={13} />
          Задача в эпик
        </Button>
      </header>

      <div className="epic-body">
        <div className="epic-main">
          <EpicHead epic={epic} writable={writable} />
          {epic.needsOwner && (
            <section className="owner-box epic-owner" aria-label="Нужно ваше решение">
              <div className="h">
                <StatusIcon status="needs_owner" size={15} />
                Нужно ваше решение
                <span className="when">
                  {epic.needsOwner.by} · {timeAgo(epic.needsOwner.at)}
                </span>
              </div>
              <p>{epic.needsOwner.question}</p>
              <div className="actions">
                <span className="grow" />
                <Link className="g-btn primary sm" to={`/tasks?status=needs_owner&task=${encodeURIComponent(epic.id)}`}>
                  Ответить
                </Link>
              </div>
            </section>
          )}
          {readyToClose && (
            <Banner
              tone="accent"
              title="Эпик готов к закрытию"
              action={
                writable ? (
                  <Button
                    variant="secondary"
                    sm
                    disabled={move.isPending}
                    onClick={() =>
                      move.mutate({ id: epic.id, status: "done" }, { onSuccess: () => toast(`${epic.id} закрыт`), onError: fail })
                    }
                  >
                    Закрыть эпик
                  </Button>
                ) : undefined
              }
            >
              Все задачи эпика закрыты. Проверьте критерии успеха — сервер примет закрытие, когда они выполнены.
            </Banner>
          )}
          <Criteria epic={epic} writable={writable} />
          <EpicTasks epic={epic} tasks={tasks} all={all} teams={teams} costs={usage?.tasks} onNew={() => onNew({ epic: epic.id })} writable={writable} />
          <EditableText epic={epic} field="plan" title="Дорожная карта" empty="Оркестратор распишет шаги к цели эпика" writable={writable} />
        </div>

        <aside className="epic-aside" aria-label="Свойства эпика">
          <EpicProps epic={epic} tasks={tasks} teams={teams} spend={usage?.spend} writable={writable} />
          <section className="sec">
            <h3>Прогресс</h3>
            {tasks.length ? <EpicProgress tasks={tasks} big legend="list" /> : <span className="muted">Задач пока нет</span>}
          </section>
          <section className="sec">
            <h3>
              Общие артефакты <span className="n">{epic.artifacts.length}</span>
              {writable && (
                <IconButton label="Добавить артефакт в эпик" className="sec-edit" onClick={() => setAdding(true)}>
                  <Icon.plus size={13} />
                </IconButton>
              )}
            </h3>
            {epic.artifacts.length ? (
              <div className="epic-artifacts">
                {epic.artifacts.map((a) => (
                  <button type="button" key={a.id} className="artifact tall" onClick={() => viewer.show(epic.id, a.id)}>
                    <ArtifactThumb task={epic.id} artifact={a} />
                    <span className="txt">
                      <span className="nm">{a.name}</span>
                      <span className="who">
                        {a.kind} · {a.author}
                        {a.note ? ` · ${a.note}` : ""}
                      </span>
                    </span>
                  </button>
                ))}
              </div>
            ) : (
              <span className="muted">Общих материалов пока нет</span>
            )}
          </section>
          <section className="sec">
            <h3>Активность</h3>
            <div className="timeline">
              {[...epic.history]
                .reverse()
                .slice(0, 8)
                .map((h) => (
                  <div key={`${h.at}${h.event}${h.to ?? ""}`} className="ev">
                    <i />
                    <span className="txt">
                      <span className="who">{h.actor}</span> {historyText(h, true)}
                    </span>
                    <span className="when">{timeAgo(h.at)}</span>
                  </div>
                ))}
            </div>
          </section>
        </aside>
      </div>
      {adding && <AddArtifactDialog task={epic.id} epic onClose={() => setAdding(false)} />}
      {viewer.modal}
    </main>
  );
}

/** The headline: the id and the status over the title, with the goal as the lede under it. */
function EpicHead({ epic, writable }: { epic: Task; writable: boolean }) {
  return (
    <header className="epic-head">
      <div className="td-meta">
        <EpicIcon filled={epic.status === "done"} size={15} />
        <span className="mono td-id">{epic.id}</span>
        <span className="g-status">
          <StatusIcon status={epic.status} size={15} />
          {STATUS_NAME[epic.status]}
        </span>
        <Tag tone="outline">эпик</Tag>
        <span className="td-meta-more">
          <PriorityIcon priority={epic.priority} size={13} />
          {PRIORITY_NAME[epic.priority]}
        </span>
      </div>
      <EditableTitle epic={epic} writable={writable} />
      <EditableText epic={epic} field="description" title="Цель" empty="Цель ещё не записана" writable={writable} lede />
    </header>
  );
}

function EditableTitle({ epic, writable }: { epic: Task; writable: boolean }) {
  const patch = usePatchTask();
  const toast = useToast();
  return (
    <h1
      className="title g-page-title"
      contentEditable={writable}
      suppressContentEditableWarning
      spellCheck={false}
      onBlur={(e) => {
        if (!writable) return;
        const title = e.currentTarget.textContent?.trim() ?? "";
        if (title && title !== epic.title) patch.mutate({ id: epic.id, patch: { title } }, { onError: (err) => toast(`Не удалось: ${err.message}`, "error") });
      }}
      onKeyDown={(e) => e.key === "Enter" && (e.preventDefault(), e.currentTarget.blur())}
    >
      {epic.title}
    </h1>
  );
}

/** The goal reads as the lede of the page; editing it replaces the prose with a composer in place. */
function EditableText({
  epic,
  field,
  title,
  empty,
  writable,
  lede,
}: {
  epic: Task;
  field: "description" | "plan";
  title: string;
  empty: string;
  writable: boolean;
  lede?: boolean;
}) {
  const patch = usePatchTask();
  const toast = useToast();
  const [draft, setDraft] = useState<string | undefined>();
  const body =
    draft === undefined ? (
      <Markdown text={epic[field]} empty={empty} />
    ) : (
      <div className="composer">
        <textarea className="g-textarea" aria-label={title} rows={8} value={draft} onChange={(e) => setDraft(e.target.value)} />
        <div className="bar">
          markdown
          <span className="grow" />
          <Button variant="ghost" sm onClick={() => setDraft(undefined)}>
            Отмена
          </Button>
          <Button
            variant="secondary"
            sm
            onClick={() => patch.mutate({ id: epic.id, patch: { [field]: draft } }, { onSuccess: () => setDraft(undefined), onError: (e) => toast(`Не удалось: ${e.message}`, "error") })}
          >
            Сохранить
          </Button>
        </div>
      </div>
    );
  if (lede)
    return (
      <div className="epic-lede">
        {body}
        {draft === undefined && writable && (
          <IconButton label={`Изменить: ${title.toLowerCase()}`} className="epic-lede-edit" onClick={() => setDraft(epic[field])}>
            <Icon.pencil size={14} />
          </IconButton>
        )}
      </div>
    );
  return (
    <section className="sec">
      <h3>
        {title}
        {draft === undefined && writable && (
          <IconButton label={`Изменить: ${title.toLowerCase()}`} className="sec-edit" onClick={() => setDraft(epic[field])}>
            <Icon.pencil size={14} />
          </IconButton>
        )}
      </h3>
      {body}
    </section>
  );
}

function Criteria({ epic, writable }: { epic: Task; writable: boolean }) {
  const check = useCheck();
  const toast = useToast();
  const done = epic.acceptance.filter((a) => a.done).length;
  return (
    <section className="sec">
      <h3>
        Критерии успеха
        <span className="n">
          {done} из {epic.acceptance.length}
        </span>
      </h3>
      {epic.acceptance.length ? (
        <div className="criteria">
          <Segments total={epic.acceptance.length} done={done} label="Критерии успеха" />
          {epic.acceptance.map((a) => (
            <label key={a.id} className={`g-check${a.done ? " done" : ""}`}>
              <input type="checkbox" checked={a.done} disabled={!writable} onChange={(e) => check.mutate({ id: epic.id, n: a.id, done: e.target.checked }, { onError: (err) => toast(`Не удалось: ${err.message}`, "error") })} />
              <span className="t">{a.text}</span>
              {a.checkedBy && <span className="by">{a.checkedBy}</span>}
            </label>
          ))}
        </div>
      ) : (
        <span className="muted">Оркестратор сформулирует, по каким признакам эпик считается достигнутым</span>
      )}
    </section>
  );
}

function EpicTasks({
  epic,
  tasks,
  all,
  teams,
  costs,
  onNew,
  writable,
}: {
  epic: Task;
  tasks: TaskSummary[];
  all: TaskSummary[];
  teams: Map<string, Team>;
  /** What each task cost, when its agents spent anything. */
  costs?: SpendItem[];
  onNew: () => void;
  writable: boolean;
}) {
  const priced = costs?.some((c) => c.spend.models.some((m) => m.cost !== null));
  const costOf = (id: string) => costs?.find((c) => c.id === id)?.spend;
  const navigate = useNavigate();
  const patch = usePatchTask();
  const toast = useToast();
  const closed = tasks.filter((t) => progressOf(t.status) === "closed").length;
  const movable = all.filter((t) => t.type !== "epic" && !t.parent && t.status !== "done" && t.status !== "cancelled");
  const open = (id: string) => navigate(`/tasks?epic=${encodeURIComponent(epic.id)}&task=${encodeURIComponent(id)}`);
  // Children grouped by where they stand: your move first, then review, work, ready, drafts, closed.
  const groups = PROGRESS.map((p) => ({ ...p, tasks: tasks.filter((t) => progressOf(t.status) === p.id) })).filter((g) => g.tasks.length > 0);

  return (
    <section className="sec epic-tasks">
      <h3>
        Задачи
        <span className="n">
          {closed} из {tasks.length} закрыто
        </span>
      </h3>
      {tasks.length ? (
        <div className="epic-task-groups">
          {groups.map((g) => (
            <div key={g.id} className="epic-task-group">
              <GroupHead count={g.tasks.length}>
                <StatusIcon status={g.tasks[0].status} size={13} />
                {g.name}
              </GroupHead>
              {g.tasks.map((t) => {
                const team = t.team ? teams.get(t.team) : undefined;
                const done = progressOf(t.status) === "closed";
                return (
                  <button type="button" key={t.id} className={`g-row epic-task${done ? " closed" : ""}`} onClick={() => open(t.id)}>
                    <StatusIcon status={t.status} size={14} />
                    <span className="mono epic-task-id">{t.id}</span>
                    <span className="epic-task-title">
                      {t.title}
                      {t.needsOwner && <span className="epic-task-note signal">{t.needsOwner.question}</span>}
                      {!done && t.openDeps.length > 0 && <span className="epic-task-note">ждёт {t.openDeps.join(", ")}</span>}
                    </span>
                    <span className="epic-task-team">{team ? <Avatars members={team.members} max={4} /> : null}</span>
                    {priced && <TaskCost spend={costOf(t.id)} />}
                    <span className="epic-task-status">{STATUS_NAME[t.status]}</span>
                  </button>
                );
              })}
            </div>
          ))}
        </div>
      ) : (
        <EmptyState
          title="Задач пока нет"
          character="sleep"
          characterLabel="Задачи ещё не заведены"
          action={
            <Button variant="secondary" onClick={onNew}>
              <Icon.plus size={13} />
              Новая задача в эпике
            </Button>
          }
        >
          Оркестратор разобьёт эпик на задачи; можно завести первую вручную.
        </EmptyState>
      )}
      <div className="epic-tasks-foot">
        {tasks.length > 0 && (
          <Button variant="ghost" sm onClick={onNew}>
            <Icon.plus size={13} />
            Новая задача в эпике
          </Button>
        )}
        {movable.length > 0 && writable && (
          <select
            className="g-select sm"
            aria-label="Перенести задачу в эпик"
            value=""
            onChange={(e) => {
              // Read the chosen IDs once, before the request: this select is controlled and falls back
              // to "" until its list refetches, so reading them in the callback would name the wrong task.
              const taskId = e.target.value;
              const targetEpic = epic.id;
              if (!taskId) return;
              patch.mutate(
                { id: taskId, patch: { parent: targetEpic } },
                { onSuccess: () => toast(`${taskId} теперь в эпике ${targetEpic}`), onError: (err) => toast(`Не удалось: ${err.message}`, "error") },
              );
            }}
          >
            <option value="">Перенести существующую…</option>
            {movable.map((t) => (
              <option key={t.id} value={t.id}>
                {t.id} · {t.title}
              </option>
            ))}
          </select>
        )}
      </div>
    </section>
  );
}

function TaskCost({ spend }: { spend?: SpendItem["spend"] }) {
  return (
    <span className="cost" title={spend ? spendTitle(spend) : undefined}>
      {spend && spend.models.some((m) => m.cost !== null) ? moneyText(spend.cost) : ""}
    </span>
  );
}

function EpicProps({ epic, tasks, teams, spend, writable }: { epic: Task; tasks: TaskSummary[]; teams: Map<string, Team>; spend?: SpendItem["spend"]; writable: boolean }) {
  const move = useMoveTask();
  const patch = usePatchTask();
  const toast = useToast();
  const fail = (e: Error) => toast(`Не удалось: ${e.message}`, "error");
  const active = tasks.filter((t) => t.team && teams.get(t.team)?.state === "active").map((t) => t.team!);
  return (
    <div className="props rail epic-props">
      <span className="k">Статус</span>
      <span className="v">
        <StatusIcon status={epic.status} size={14} />
        {writable ? (
          <select
            className="g-select sm"
            aria-label="Статус"
            value={epic.status}
            onChange={(e) => move.mutate({ id: epic.id, status: e.target.value as Status }, { onSuccess: () => toast(`${epic.id} → ${STATUS_NAME[e.target.value as Status]}`), onError: fail })}
          >
            {(Object.keys(STATUS_NAME) as Status[]).map((s) => (
              <option key={s} value={s}>
                {STATUS_NAME[s]}
              </option>
            ))}
          </select>
        ) : (
          <span>{STATUS_NAME[epic.status]}</span>
        )}
      </span>
      <span className="k">Приоритет</span>
      <span className="v">
        <PriorityIcon priority={epic.priority} size={14} />
        {writable ? (
          <select className="g-select sm" aria-label="Приоритет" value={epic.priority} onChange={(e) => patch.mutate({ id: epic.id, patch: { priority: Number(e.target.value) } }, { onError: fail })}>
            {PRIORITY_NAME.map((p, i) => (
              <option key={p} value={i}>
                {p}
              </option>
            ))}
          </select>
        ) : (
          <span>{PRIORITY_NAME[epic.priority]}</span>
        )}
      </span>
      <span className="k">Метки</span>
      <span className="v">{epic.labels.length ? <Labels labels={epic.labels} /> : <span className="muted">нет</span>}</span>
      <span className="k">Команды</span>
      <span className="v">
        {active.length ? (
          <>
            <span className="epic-active-dot" aria-hidden="true" />
            {active.map((t, i) => (
              <span key={t}>
                <Link to={`/team/${encodeURIComponent(t)}`}>{t}</Link>
                {i < active.length - 1 ? "," : ""}
              </span>
            ))}
          </>
        ) : (
          <span className="muted">сейчас не работают</span>
        )}
      </span>
      {!!spend?.calls && (
        <>
          <span className="k">Расходы</span>
          <span className="v">
            <SpendText spend={spend} />
          </span>
        </>
      )}
    </div>
  );
}
