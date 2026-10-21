import { useState } from "react";
import { Link, useSearchParams } from "react-router";
import {
  EpicArcs,
  EpicProgress,
  PRIORITY_NAME,
  PriorityIcon,
  progressOf,
  STATUS_NAME,
  StatusIcon,
  type Status,
  type TaskSummary,
  useTask,
  useTasks,
} from "@/entities/task";
import { type Team, useTeamMap } from "@/entities/team";
import type { NewTaskPreset } from "@/features/create-task";
import { plural, timeAgo, useTick } from "@/shared/lib";
import { ContextSearch } from "@/widgets/topbar";
import { Button, EmptyState, ErrorState, Icon, Skeleton, Tag, Tabs } from "@/shared/ui";

const closed = (t: TaskSummary) => t.status === "done" || t.status === "cancelled";

/** How a status reads on a card: green is system state, orange is "your move", grey is quiet. */
function statusTone(status: Status): "accent" | "signal" | "outline" {
  if (status === "needs_owner") return "signal";
  if (status === "refining" || status === "ready" || status === "in_progress" || status === "changes_requested" || status === "review" || status === "approved") return "accent";
  return "outline";
}

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
  const empty = !tasksQ.isPending && !tasksQ.isError && !shown.length && !(tab === "open" && !q);

  return (
    <main className="main epics-page" aria-label="Эпики">
      <header className="ep-head">
        <div className="ep-head-text">
          <h1 className="ep-title">Эпики</h1>
          <p className="ep-lede">Крупные вехи: цель, критерии успеха и общие материалы. Оркестратор разбивает эпик на задачи.</p>
        </div>
        <Tabs
          label="Какие эпики показать"
          value={tab}
          onChange={setTab}
          items={[
            { id: "open", label: <>Открытые · {open.length}</> },
            { id: "closed", label: <>Завершённые · {done.length}</> },
          ]}
        />
        <span className="grow" />
        <ContextSearch inputRef={searchRef} className="mobile-context-search" />
      </header>

      <div className="scroll epics-scroll">
        {tasksQ.isPending ? (
          <div className="epic-grid" aria-busy="true" aria-label="Эпики загружаются">
            {Array.from({ length: 3 }, (_, i) => (
              <EpicCardSkeleton key={i} />
            ))}
          </div>
        ) : tasksQ.isError ? (
          <ErrorState message={`Не удалось загрузить эпики: ${tasksQ.error.message}`} onRetry={() => void tasksQ.refetch()} />
        ) : (
          <>
            {empty && q && <EmptyState title="Ничего не нашлось">По запросу «{query.trim()}» эпиков нет.</EmptyState>}
            {empty && !q && (
              <EmptyState
                title={tab === "open" ? "Эпиков пока нет" : "Завершённых эпиков пока нет"}
                character="sleep"
                characterSize={72}
                characterLabel="Эпики спят"
                action={
                  tab === "open" ? (
                    <Button variant="primary" onClick={() => onNew({ type: "epic" })}>
                      <Icon.plus size={13} />
                      Новый эпик
                    </Button>
                  ) : undefined
                }
              >
                {tab === "open" ? "Первая веха задаёт цель и критерии успеха; задачи к ней соберутся сами." : "Закрытый эпик оставит здесь свою историю и общие материалы."}
              </EmptyState>
            )}
            <div className="epic-grid">
              {shown.map((e) => (
                <EpicCard key={e.id} epic={e} tasks={all.filter((t) => t.parent === e.id)} teams={teams} />
              ))}
              {tab === "open" && !q && (
                <button type="button" className="epic-card new" onClick={() => onNew({ type: "epic" })}>
                  <span className="epic-card-new-mark">
                    <Icon.plus size={18} />
                  </span>
                  <b>Новый эпик</b>
                  <span>Крупная веха: цель, критерии успеха и общие материалы. Оркестратор разобьёт его на задачи.</span>
                </button>
              )}
            </div>
          </>
        )}
      </div>
    </main>
  );
}

function EpicCard({ epic, tasks, teams }: { epic: TaskSummary; tasks: TaskSummary[]; teams: Map<string, Team> }) {
  const full = useTask(epic.id).data;
  const goal = full?.description.replace(/[#*_`>]/g, "").trim();
  const activeTeams = tasks.filter((t) => t.team && teams.get(t.team)?.state === "active").length;
  const done = tasks.filter((t) => progressOf(t.status) === "closed").length;
  const opened = tasks.length - done;
  const isClosed = closed(epic);
  return (
    <Link to={`/epic/${encodeURIComponent(epic.id)}`} className={`epic-card g-card${isClosed ? " closed" : ""}${!tasks.length && !isClosed ? " empty-epic" : ""}`}>
      <span className="epic-card-head">
        <EpicArcs done={done} total={tasks.length} size={56} />
        <span className="epic-card-open">
          <span className="num">{opened}</span>
          <span className="cap">{plural(opened, "открытая задача", "открытые задачи", "открытых задач")}</span>
        </span>
        <span className="epic-card-status">
          <Tag tone={statusTone(epic.status)}>
            <StatusIcon status={epic.status} size={12} />
            {STATUS_NAME[epic.status]}
          </Tag>
        </span>
      </span>
      <span className="epic-card-title">{epic.title}</span>
      <span className="epic-card-meta">
        <span className="mono">{epic.id}</span>
        <span className="prio">
          <PriorityIcon priority={epic.priority} size={12} />
          {PRIORITY_NAME[epic.priority]}
        </span>
      </span>
      {goal ? <span className="epic-card-goal">{goal}</span> : <span className="epic-card-goal muted">Цель ещё не записана</span>}
      {tasks.length > 0 ? (
        <EpicProgress tasks={tasks} legend="inline" />
      ) : (
        !isClosed && (
          <span className="epic-card-waiting">
            {epic.status === "inbox" ? "Во входящих — оркестратор возьмёт эпик в работу" : "Задач пока нет — оркестратор пишет дорожную карту и разбивает эпик"}
          </span>
        )
      )}
      <span className="epic-card-foot">
        <span>
          <Icon.file size={13} />
          {epic.artifacts} {plural(epic.artifacts, "общий артефакт", "общих артефакта", "общих артефактов")}
        </span>
        {activeTeams > 0 && (
          <span>
            <Icon.user size={13} />
            {activeTeams} {plural(activeTeams, "команда", "команды", "команд")}
          </span>
        )}
        <span className="grow" />
        <span>{timeAgo(epic.updated)}</span>
      </span>
    </Link>
  );
}

function EpicCardSkeleton() {
  return (
    <div className="epic-card g-card" aria-hidden="true">
      <span className="epic-card-head">
        <Skeleton width={56} height={56} />
        <Skeleton width={72} height={30} />
      </span>
      <Skeleton width="70%" height={20} />
      <Skeleton width="92%" height={14} />
      <Skeleton width="60%" height={14} />
      <Skeleton height={6} />
    </div>
  );
}
