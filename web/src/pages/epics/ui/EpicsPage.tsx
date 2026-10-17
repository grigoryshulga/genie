import { useState } from "react";
import { Link, useSearchParams } from "react-router";
import { EpicIcon, EpicProgress, PRIORITY_NAME, PriorityIcon, STATUS_NAME, StatusIcon, type TaskSummary, useTask, useTasks } from "@/entities/task";
import { type Team, useTeamMap } from "@/entities/team";
import type { NewTaskPreset } from "@/features/create-task";
import { plural, timeAgo, useTick } from "@/shared/lib";
import { ContextSearch } from "@/widgets/sidebar";
import { Icon } from "@/shared/ui";

const closed = (t: TaskSummary) => t.status === "done" || t.status === "cancelled";

export function EpicsPage({ onNew, searchRef }: { onNew: (preset?: NewTaskPreset) => void; searchRef: React.RefObject<HTMLInputElement | null> }) {
  useTick();
  const tasksQ = useTasks();
  const teams = useTeamMap();
  const [tab, setTab] = useState<"open" | "closed">("open");
  const [params] = useSearchParams();
  const query = params.get("q") ?? "";
  const all = tasksQ.data ?? [];
  const epics = all.filter((t) => t.type === "epic");
  const open = epics.filter((e) => !closed(e));
  const done = epics.filter(closed);
  const q = query.trim().toLowerCase();
  const shown = (tab === "open" ? open : done).filter((e) => !q || e.title.toLowerCase().includes(q) || e.id.toLowerCase().includes(q));

  return (
    <main className="main" aria-label="Эпики">
      <header className="topbar page-toolbar">
        <div className="seg" role="group" aria-label="Какие эпики показать">
          <button type="button" className={tab === "open" ? "on" : ""} aria-pressed={tab === "open"} onClick={() => setTab("open")}>
            Открытые · {open.length}
          </button>
          <button type="button" className={tab === "closed" ? "on" : ""} aria-pressed={tab === "closed"} onClick={() => setTab("closed")}>
            Завершённые · {done.length}
          </button>
        </div>
        <span className="grow" />
        <ContextSearch inputRef={searchRef} className="mobile-context-search" />
      </header>

      <div className="scroll">
        {tasksQ.isPending ? (
          <div className="empty">Загрузка…</div>
        ) : tasksQ.isError ? (
          <div className="empty">Не удалось загрузить эпики: {tasksQ.error.message}</div>
        ) : (
          <div className="epic-grid">
            {shown.map((e) => (
              <EpicCard key={e.id} epic={e} tasks={all.filter((t) => t.parent === e.id)} teams={teams} />
            ))}
            {tab === "open" && !q && (
              <button type="button" className="epic-card new" onClick={() => onNew({ type: "epic" })}>
                <Icon.plus size={18} />
                <b>Новый эпик</b>
                <span>Крупная веха: цель, критерии успеха и общие материалы. Оркестратор разобьёт его на задачи.</span>
              </button>
            )}
            {tab === "closed" && !shown.length && <div className="empty">Завершённых эпиков пока нет</div>}
          </div>
        )}
      </div>
    </main>
  );
}

function EpicCard({ epic, tasks, teams }: { epic: TaskSummary; tasks: TaskSummary[]; teams: Map<string, Team> }) {
  const full = useTask(epic.id).data;
  const goal = full?.description.replace(/[#*_`>]/g, "").trim();
  const activeTeams = tasks.filter((t) => t.team && teams.get(t.team)?.state === "active").length;
  const isClosed = closed(epic);
  return (
    <Link to={`/epic/${encodeURIComponent(epic.id)}`} className={`epic-card${isClosed ? " closed" : ""}${!tasks.length && !isClosed ? " empty-epic" : ""}`}>
      <span className="meta">
        <EpicIcon filled={epic.status === "done"} empty={!tasks.length && !isClosed} />
        <span className="mono">{epic.id}</span>
        <StatusIcon status={epic.status} size={12} />
        {STATUS_NAME[epic.status]}
        <span className="prio">
          <PriorityIcon priority={epic.priority} size={12} />
          {PRIORITY_NAME[epic.priority]}
        </span>
      </span>
      <span className="t">{epic.title}</span>
      {goal ? <span className="goal">{goal}</span> : <span className="goal muted">Цель ещё не записана</span>}
      {tasks.length > 0 ? (
        <EpicProgress tasks={tasks} legend="inline" />
      ) : (
        !isClosed && (
          <span className="waiting">
            <span className="spin" style={{ borderTopColor: "var(--violet)" }} />
            {epic.status === "inbox" ? "Во входящих — оркестратор возьмёт эпик в работу" : "Задач пока нет — оркестратор пишет дорожную карту и разбивает эпик"}
          </span>
        )
      )}
      <span className="foot">
        <span>
          <Icon.file size={13} />
          {epic.artifacts} {plural(epic.artifacts, "общий артефакт", "общих артефакта", "общих артефактов")}
        </span>
        {activeTeams > 0 && (
          <span>
            <span className="spin" style={{ width: 10, height: 10 }} />
            {activeTeams} {plural(activeTeams, "команда", "команды", "команд")}
          </span>
        )}
        <span className="grow" />
        <span>{timeAgo(epic.updated)}</span>
      </span>
    </Link>
  );
}
