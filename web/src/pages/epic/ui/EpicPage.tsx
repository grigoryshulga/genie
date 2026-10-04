import { useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { Avatars } from "@/entities/member";
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
import { Icon, Markdown, useToast } from "@/shared/ui";

export function EpicPage({ onNew }: { onNew: (preset?: NewTaskPreset) => void }) {
  useTick();
  const id = useParams().epicId ?? "";
  const q = useTask(id);
  const all = useTasks().data ?? [];
  const teams = useTeamMap();
  const usage = useTaskUsage(id || undefined).data;
  const toast = useToast();
  const fail = (e: Error) => toast(`Не удалось: ${e.message}`, "error");
  const viewer = useArtifactViewer(fail);
  const [adding, setAdding] = useState(false);

  if (q.isPending) return <main className="main"><div className="empty">Загрузка…</div></main>;
  if (q.isError) return <main className="main"><div className="empty">{q.error.message}</div></main>;
  const epic = q.data;
  if (epic.type !== "epic") {
    return (
      <main className="main">
        <div className="empty">
          {epic.id} — не эпик. <Link to={`/tasks?task=${encodeURIComponent(epic.id)}`}>Открыть задачу</Link>
        </div>
      </main>
    );
  }
  const tasks = all.filter((t) => t.parent === epic.id);

  return (
    <main className="main epic-page">
      <header className="topbar">
        <Link to="/epics" className="muted" style={{ fontSize: 12 }}>
          Эпики
        </Link>
        <Icon.chevron size={12} />
        <span className="mono" style={{ fontSize: 12 }}>
          {epic.id}
        </span>
        <span className="grow" />
        <Link className="btn d-only" to={`/board?epic=${encodeURIComponent(epic.id)}`}>
          <Icon.board size={13} />
          Задачи эпика на доске
        </Link>
        <button type="button" className="btn primary" onClick={() => onNew({ epic: epic.id })}>
          <Icon.plus size={13} />
          Задача в эпик
        </button>
      </header>

      <div className="epic-body">
        <div className="epic-main">
          <EpicHead epic={epic} />
          {epic.needsOwner && (
            <section className="owner-box" aria-label="Нужно ваше решение">
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
                <Link className="btn amber" to={`/tasks?status=needs_owner&task=${encodeURIComponent(epic.id)}`}>
                  Ответить
                </Link>
              </div>
            </section>
          )}
          <EditableText epic={epic} field="description" title="Цель" empty="Цель ещё не записана" />
          <Criteria epic={epic} />
          <EpicTasks epic={epic} tasks={tasks} all={all} teams={teams} costs={usage?.tasks} onNew={() => onNew({ epic: epic.id })} />
          <EditableText epic={epic} field="plan" title="Дорожная карта" empty="Оркестратор распишет шаги к цели эпика" />
        </div>

        <aside className="epic-aside" aria-label="Свойства эпика">
          <EpicProps epic={epic} tasks={tasks} teams={teams} spend={usage?.spend} />
          <section className="sec">
            <h3>Прогресс</h3>
            {tasks.length ? <EpicProgress tasks={tasks} big legend="list" /> : <span className="muted">Задач пока нет</span>}
          </section>
          <section className="sec">
            <h3>
              Общие артефакты <span className="n">{epic.artifacts.length}</span>
              <button type="button" className="btn" style={{ height: 24, fontSize: 11.5 }} onClick={() => setAdding(true)} aria-label="Добавить артефакт в эпик">
                <Icon.plus size={11} />
                Добавить
              </button>
            </h3>
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
            <p className="hint">Каждая команда в эпике видит его цель и этот список в своей задаче.</p>
          </section>
          <section className="sec">
            <h3>Активность</h3>
            {[...epic.history]
              .reverse()
              .slice(0, 8)
              .map((h) => (
                <div key={`${h.at}${h.event}${h.to ?? ""}`} className="ev">
                  <i />
                  <span>
                    <span style={{ color: "var(--text-2)" }}>{h.actor}</span> {historyText(h, true)} · {timeAgo(h.at)}
                  </span>
                </div>
              ))}
          </section>
        </aside>
      </div>
      {adding && <AddArtifactDialog task={epic.id} epic onClose={() => setAdding(false)} />}
      {viewer.modal}
    </main>
  );
}

function EpicHead({ epic }: { epic: Task }) {
  const patch = usePatchTask();
  const toast = useToast();
  return (
    <div className="epic-head">
      <span className="kind">
        <EpicIcon filled={epic.status === "done"} />
        Эпик
      </span>
      <h2
        className="title"
        contentEditable
        suppressContentEditableWarning
        spellCheck={false}
        onBlur={(e) => {
          const title = e.currentTarget.textContent?.trim() ?? "";
          if (title && title !== epic.title) patch.mutate({ id: epic.id, patch: { title } }, { onError: (err) => toast(`Не удалось: ${err.message}`, "error") });
        }}
        onKeyDown={(e) => e.key === "Enter" && (e.preventDefault(), e.currentTarget.blur())}
      >
        {epic.title}
      </h2>
    </div>
  );
}

function EditableText({ epic, field, title, empty }: { epic: Task; field: "description" | "plan"; title: string; empty: string }) {
  const patch = usePatchTask();
  const toast = useToast();
  const [draft, setDraft] = useState<string | undefined>();
  return (
    <section className="sec">
      <h3>
        {title}
        {draft === undefined && (
          <button type="button" className="btn ghost" style={{ height: 24 }} onClick={() => setDraft(epic[field])}>
            Изменить
          </button>
        )}
      </h3>
      {draft === undefined ? (
        <Markdown text={epic[field]} empty={empty} />
      ) : (
        <div className="composer">
          <textarea aria-label={title} rows={8} value={draft} onChange={(e) => setDraft(e.target.value)} />
          <div className="bar">
            markdown
            <span className="grow" />
            <button type="button" className="btn ghost" onClick={() => setDraft(undefined)}>
              Отмена
            </button>
            <button
              type="button"
              className="btn primary"
              onClick={() => patch.mutate({ id: epic.id, patch: { [field]: draft } }, { onSuccess: () => setDraft(undefined), onError: (e) => toast(`Не удалось: ${e.message}`, "error") })}
            >
              Сохранить
            </button>
          </div>
        </div>
      )}
    </section>
  );
}

function Criteria({ epic }: { epic: Task }) {
  const check = useCheck();
  const toast = useToast();
  return (
    <section className="sec">
      <h3>
        Критерии успеха
        <span className="n">
          {epic.acceptance.filter((a) => a.done).length} / {epic.acceptance.length}
        </span>
      </h3>
      {epic.acceptance.length ? (
        <div className="criteria">
          {epic.acceptance.map((a) => (
            <label key={a.id} className={a.done ? "done" : ""}>
              <input type="checkbox" checked={a.done} onChange={(e) => check.mutate({ id: epic.id, n: a.id, done: e.target.checked }, { onError: (err) => toast(`Не удалось: ${err.message}`, "error") })} />
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
}: {
  epic: Task;
  tasks: TaskSummary[];
  all: TaskSummary[];
  teams: Map<string, Team>;
  /** What each task cost, when its agents spent anything. */
  costs?: SpendItem[];
  onNew: () => void;
}) {
  const priced = costs?.some((c) => c.spend.models.some((m) => m.cost !== null));
  const costOf = (id: string) => costs?.find((c) => c.id === id)?.spend;
  const navigate = useNavigate();
  const patch = usePatchTask();
  const toast = useToast();
  const order = PROGRESS.map((p) => p.id);
  const sorted = [...tasks].sort((a, b) => order.indexOf(progressOf(a.status)) - order.indexOf(progressOf(b.status)));
  const closed = tasks.filter((t) => progressOf(t.status) === "closed").length;
  const movable = all.filter((t) => t.type !== "epic" && !t.parent && t.status !== "done" && t.status !== "cancelled");
  const open = (id: string) => navigate(`/tasks?epic=${encodeURIComponent(epic.id)}&task=${encodeURIComponent(id)}`);

  return (
    <section className="sec">
      <h3>
        Задачи
        <span className="n">
          {closed} / {tasks.length} закрыто
        </span>
      </h3>
      <div className="epic-tasks">
        {sorted.map((t) => {
          const team = t.team ? teams.get(t.team) : undefined;
          const isClosed = progressOf(t.status) === "closed";
          return (
            <button type="button" key={t.id} className={`etask${isClosed ? " closed" : ""}${priced ? " with-cost" : ""}`} onClick={() => open(t.id)}>
              <StatusIcon status={t.status} />
              <span className="id">{t.id}</span>
              <span className="tt">
                {t.title}
                {t.needsOwner && <span className="q">{t.needsOwner.question}</span>}
                {!isClosed && t.openDeps.length > 0 && <span className="w">ждёт {t.openDeps.join(", ")}</span>}
              </span>
              <span className="tm">{team ? <Avatars members={team.members} max={4} /> : null}</span>
              {priced && <TaskCost spend={costOf(t.id)} />}
              <span className={`st${t.status === "needs_owner" ? " amber" : ""}`}>{STATUS_NAME[t.status]}</span>
            </button>
          );
        })}
        <div className="etask-add">
          <button type="button" className="btn ghost" onClick={onNew}>
            <Icon.plus size={13} />
            Новая задача в эпике
          </button>
          {movable.length > 0 && (
            <select
              aria-label="Перенести задачу в эпик"
              value=""
              onChange={(e) =>
                e.target.value &&
                patch.mutate(
                  { id: e.target.value, patch: { parent: epic.id } },
                  { onSuccess: () => toast(`${e.target.value} теперь в эпике ${epic.id}`), onError: (err) => toast(`Не удалось: ${err.message}`, "error") },
                )
              }
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

function EpicProps({ epic, tasks, teams, spend }: { epic: Task; tasks: TaskSummary[]; teams: Map<string, Team>; spend?: SpendItem["spend"] }) {
  const move = useMoveTask();
  const patch = usePatchTask();
  const toast = useToast();
  const fail = (e: Error) => toast(`Не удалось: ${e.message}`, "error");
  const active = tasks.filter((t) => t.team && teams.get(t.team)?.state === "active").map((t) => t.team!);
  return (
    <div className="props epic-props">
      <span className="k">Статус</span>
      <span style={{ display: "flex", alignItems: "center", gap: 7 }}>
        <StatusIcon status={epic.status} size={13} />
        <select
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
      </span>
      <span className="k">Приоритет</span>
      <span style={{ display: "flex", alignItems: "center", gap: 7 }}>
        <PriorityIcon priority={epic.priority} size={13} />
        <select aria-label="Приоритет" value={epic.priority} onChange={(e) => patch.mutate({ id: epic.id, patch: { priority: Number(e.target.value) } }, { onError: fail })}>
          {PRIORITY_NAME.map((p, i) => (
            <option key={p} value={i}>
              {p}
            </option>
          ))}
        </select>
      </span>
      <span className="k">Метки</span>
      <span>{epic.labels.length ? <Labels labels={epic.labels} /> : <span className="muted">нет</span>}</span>
      <span className="k">Команды</span>
      <span style={{ display: "flex", alignItems: "center", gap: 7, flexWrap: "wrap" }}>
        {active.length ? (
          <>
            <span className="spin" style={{ width: 10, height: 10 }} />
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
          <SpendText spend={spend} />
        </>
      )}
    </div>
  );
}
