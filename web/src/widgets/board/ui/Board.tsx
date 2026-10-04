import {
  DndContext,
  type DragEndEvent,
  DragOverlay,
  type DragStartEvent,
  KeyboardSensor,
  PointerSensor,
  TouchSensor,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import { useState } from "react";
import { Avatars } from "@/entities/member";
import { PersonAvatar } from "@/entities/project";
import { COLUMNS, type Column, EpicChip, EpicIcon, Labels, PriorityIcon, type Status, STATUS_NAME, StatusIcon, type TaskSummary, useMoveTask } from "@/entities/task";
import type { Team } from "@/entities/team";
import { Modal, useToast } from "@/shared/ui";

const columnOf = (status: Status): Column => COLUMNS.find((c) => c.statuses.includes(status)) ?? COLUMNS[0];

function CardBody({ t, team }: { t: TaskSummary; team?: Team }) {
  return (
    <>
      <span className="meta">
        <span className="mono">{t.id}</span>
        {t.priority === 0 ? <span className="urgent-tag">срочно</span> : <PriorityIcon priority={t.priority} size={12} />}
        {t.status === "changes_requested" && <span style={{ color: "var(--amber)" }}>доработка</span>}
      </span>
      <span className="t" title={t.title}>
        {t.type === "epic" && (
          <span className="epic-mark">
            <EpicIcon size={12} />
            эпик
          </span>
        )}
        {t.title}
      </span>
      {t.needsOwner ? (
        <span className="note q" title={t.needsOwner.question}>
          {t.needsOwner.question}
        </span>
      ) : (
        <span className="note">{t.openDeps.length > 0 && `ждёт ${t.openDeps.join(", ")}`}</span>
      )}
      <span className="foot">
        <EpicChip id={t.parent} text />
        {/* Two labels at most: the rest is a count, so the footer never wraps. */}
        <Labels labels={t.labels.slice(0, 2)} />
        {t.labels.length > 2 && <span className="more">+{t.labels.length - 2}</span>}
        {t.acceptanceTotal > 0 && (
          <span className="ac">
            ✓ {t.acceptanceDone}/{t.acceptanceTotal}
          </span>
        )}
        <span className="grow" />
        {team && <Avatars members={team.members} />}
        {t.assignee && <PersonAvatar login={t.assignee} />}
      </span>
    </>
  );
}

function Card({ t, team, selected, onSelect }: { t: TaskSummary; team?: Team; selected: boolean; onSelect: () => void }) {
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({ id: t.id });
  const cls = ["card", selected ? "sel" : "", t.status === "needs_owner" ? "owner" : "", isDragging ? "dragging" : ""].filter(Boolean).join(" ");
  return (
    <button type="button" ref={setNodeRef} className={cls} onClick={onSelect} {...attributes} {...listeners} aria-pressed={selected} aria-roledescription="карточка задачи">
      <CardBody t={t} team={team} />
    </button>
  );
}

function BoardColumn({ col, tasks, teams, selected, onSelect, collapsed, onExpand }: {
  col: Column;
  tasks: TaskSummary[];
  teams: Map<string, Team>;
  selected?: string;
  onSelect: (id: string) => void;
  collapsed: boolean;
  onExpand: () => void;
}) {
  const { setNodeRef, isOver } = useDroppable({ id: col.id });
  const cls = ["col", col.id === "needs_owner" ? "owner" : "", isOver ? "over" : "", collapsed ? "collapsed" : ""].filter(Boolean).join(" ");
  return (
    <section ref={setNodeRef} className={cls} aria-label={col.name}>
      {collapsed ? (
        <button type="button" className="expand" onClick={onExpand} aria-label={`Показать колонку ${col.name}`}>
          <StatusIcon status={col.target} size={13} />
          <span className="v">
            {col.name} · {tasks.length}
          </span>
        </button>
      ) : (
        <>
          <header>
            <StatusIcon status={col.target} size={13} />
            {col.name}
            <span className="n">{tasks.length}</span>
          </header>
          <div className="cards">
            {tasks.map((t) => (
              <Card key={t.id} t={t} team={t.team ? teams.get(t.team) : undefined} selected={selected === t.id} onSelect={() => onSelect(t.id)} />
            ))}
            {!tasks.length && <div className="drop-here">Перетащите сюда</div>}
          </div>
        </>
      )}
    </section>
  );
}

/** A card click opens the task's preview beside the board (`?task=`), like a row in the list. */
export function Board({ tasks, teams, selected, showDone, onShowDone, onOpen }: {
  tasks: TaskSummary[];
  teams: Map<string, Team>;
  selected?: string;
  showDone: boolean;
  onShowDone: () => void;
  onOpen: (id: string) => void;
}) {
  const move = useMoveTask();
  const toast = useToast();
  const [dragging, setDragging] = useState<string | undefined>();
  const [ask, setAsk] = useState<{ id: string; note: string } | undefined>();
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(TouchSensor, { activationConstraint: { delay: 250, tolerance: 6 } }),
    useSensor(KeyboardSensor),
  );

  const doMove = (id: string, status: Status, note?: string) => {
    const t = tasks.find((x) => x.id === id);
    if (!t || t.status === status) return;
    move.mutate(
      { id, status, note },
      {
        onSuccess: () => toast(`${id} → ${STATUS_NAME[status]} · оркестратор уведомлён`),
        onError: (e) => toast(`Не удалось: ${e.message}`, "error"),
      },
    );
  };

  const onDragEnd = (e: DragEndEvent) => {
    setDragging(undefined);
    const id = String(e.active.id);
    const col = COLUMNS.find((c) => c.id === e.over?.id);
    const t = tasks.find((x) => x.id === id);
    if (!col || !t || col.statuses.includes(t.status)) return;
    if (col.target === "needs_owner") setAsk({ id, note: "" });
    else doMove(id, col.target);
  };

  const draggingTask = tasks.find((t) => t.id === dragging);

  return (
    <DndContext sensors={sensors} onDragStart={(e: DragStartEvent) => setDragging(String(e.active.id))} onDragEnd={onDragEnd} onDragCancel={() => setDragging(undefined)}>
      <div className="board">
        {COLUMNS.map((col) => (
          <BoardColumn
            key={col.id}
            col={col}
            tasks={tasks.filter((t) => columnOf(t.status).id === col.id)}
            teams={teams}
            selected={selected}
            onSelect={onOpen}
            collapsed={col.id === "done" && !showDone}
            onExpand={onShowDone}
          />
        ))}
      </div>
      <DragOverlay dropAnimation={null}>
        {draggingTask && (
          <div className="card" style={{ cursor: "grabbing", boxShadow: "var(--shadow)", width: 244 }}>
            <CardBody t={draggingTask} team={draggingTask.team ? teams.get(draggingTask.team) : undefined} />
          </div>
        )}
      </DragOverlay>
      {ask && (
        <Modal label="Вопрос к владельцу" onClose={() => setAsk(undefined)}>
          <div className="mh">
            <StatusIcon status="needs_owner" size={13} /> {ask.id} → Нужно решение
          </div>
          <div className="mb">
            <label className="field">
              Что нужно решить
              <textarea autoFocus rows={3} value={ask.note} onChange={(e) => setAsk({ ...ask, note: e.target.value })} />
            </label>
          </div>
          <div className="mf">
            <span className="grow" />
            <button type="button" className="btn ghost" onClick={() => setAsk(undefined)}>
              Отмена
            </button>
            <button
              type="button"
              className="btn amber"
              disabled={!ask.note.trim()}
              onClick={() => {
                doMove(ask.id, "needs_owner", ask.note.trim());
                setAsk(undefined);
              }}
            >
              Перенести
            </button>
          </div>
        </Modal>
      )}
    </DndContext>
  );
}
