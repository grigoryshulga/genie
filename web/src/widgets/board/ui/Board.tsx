import {
  DndContext,
  type DragEndEvent,
  DragOverlay,
  type DragStartEvent,
  KeyboardSensor,
  PointerSensor,
  TouchSensor,
  useDroppable,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import { useEffect, useRef, useState } from "react";
import {
  groupByLane,
  epicTone,
  LANES,
  type Lane,
  type LaneId,
  laneDropId,
  laneOf,
  needsQuestion,
  resolveDrop,
  type Status,
  STATUS_NAME,
  statusDropId,
  type TaskSummary,
  useMoveTask,
  useEpicMap,
} from "@/entities/task";
import type { Team } from "@/entities/team";
import { plural } from "@/shared/lib";
import { Button, Chip, Dialog, Field, Icon, Textarea, cx, useToast } from "@/shared/ui";
import { BoardCard, CardBody } from "./BoardCard.tsx";
import { StackedList } from "./StackedList.tsx";
import "./board.css";

const countText = (n: number) => `${n} ${plural(n, "задача", "задачи", "задач")}`;

/** One exact status a dragged card can be dropped on, shown only while a card is in the air. */
function DropChip({ status, dragging }: { status: Status; dragging?: Status }) {
  const { setNodeRef, isOver } = useDroppable({ id: statusDropId(status) });
  return (
    <span ref={setNodeRef} className={cx("g-lane-drop", isOver && "over", dragging === status && "here")}>
      {STATUS_NAME[status]}
    </span>
  );
}

function LaneSection({
  lane,
  tasks,
  teams,
  selected,
  collapsed,
  dragging,
  offline,
  onToggle,
  onOpen,
  onMove,
  laneRef,
}: {
  lane: Lane;
  tasks: TaskSummary[];
  teams: Map<string, Team>;
  selected?: string;
  collapsed: boolean;
  /** The status of the card in the air, if any: it turns on the drop chips. */
  dragging?: Status;
  offline: boolean;
  onToggle: () => void;
  onOpen: (id: string) => void;
  onMove: (id: string, status: Status) => void;
  laneRef: (node: HTMLElement | null) => void;
}) {
  const { setNodeRef, isOver } = useDroppable({ id: laneDropId(lane) });
  const n = tasks.length;
  return (
    <section
      ref={(node) => {
        setNodeRef(node);
        laneRef(node);
      }}
      className={cx("g-lane", `g-lane-${lane.id}`, n > 0 && "has", collapsed && "shut", isOver && dragging && "over")}
      data-lane={lane.id}
      aria-label={`${lane.name}, ${countText(n)}`}
    >
      <header className="g-lane-head">
        <button
          type="button"
          className="g-lane-toggle"
          onClick={onToggle}
          aria-expanded={!collapsed}
          title={collapsed ? `Развернуть дорожку «${lane.name}», ${countText(n)}` : `Свернуть дорожку «${lane.name}»`}
        >
          <span className="g-lane-n">{n}</span>
          <span className="g-lane-name">{lane.name}</span>
          <Icon.chevron size={14} className="g-lane-chev" />
        </button>
        {!collapsed && <p className="g-lane-hint">{lane.hint}</p>}
        {dragging && lane.drops.length > 1 && (
          <div className="g-lane-drops" role="group" aria-label={`Точный статус в дорожке «${lane.name}»`}>
            {lane.drops.map((s) => (
              <DropChip key={s} status={s} dragging={dragging} />
            ))}
          </div>
        )}
      </header>
      {!collapsed && (
        <StackedList>
          {tasks.map((t) => (
            <BoardCard
              key={t.id}
              t={t}
              team={t.team ? teams.get(t.team) : undefined}
              selected={selected === t.id}
              offline={offline}
              onSelect={() => onOpen(t.id)}
              onMove={(status) => onMove(t.id, status)}
            />
          ))}
          {!n && <p className="g-lane-empty">{dragging ? "Перетащите сюда" : "Ничего"}</p>}
        </StackedList>
      )}
    </section>
  );
}

/** Cancelled tasks belong to no lane: they wait outside the lanes, where a card can still be dragged out. */
function CancelledStrip({
  tasks,
  teams,
  selected,
  offline,
  onOpen,
  onMove,
}: {
  tasks: TaskSummary[];
  teams: Map<string, Team>;
  selected?: string;
  offline: boolean;
  onOpen: (id: string) => void;
  onMove: (id: string, status: Status) => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <section className={cx("g-lane", "g-lane-cancelled", !open && "shut")} data-lane="cancelled" aria-label={`Отменено, ${countText(tasks.length)}`}>
      <header className="g-lane-head">
        <button type="button" className="g-lane-toggle" onClick={() => setOpen(!open)} aria-expanded={open} title={open ? "Свернуть отменённые" : "Показать отменённые"}>
          <span className="g-lane-n">{tasks.length}</span>
          <span className="g-lane-name">Отменено</span>
          <Icon.chevron size={14} className="g-lane-chev" />
        </button>
        {!open && <p className="g-lane-hint">Вне дорожек</p>}
      </header>
      {open && (
        <StackedList>
          {tasks.map((t) => (
            <BoardCard
              key={t.id}
              t={t}
              team={t.team ? teams.get(t.team) : undefined}
              selected={selected === t.id}
              offline={offline}
              onSelect={() => onOpen(t.id)}
              onMove={(status) => onMove(t.id, status)}
            />
          ))}
        </StackedList>
      )}
    </section>
  );
}

/**
 * «Доска»: the six lanes the owner decided on. A card click opens the task's preview beside the board
 * (`?task=`), like a row in the list. On phones the lanes slide one per screen and a Move-to menu
 * replaces dragging; every exact status the old eight columns allowed is still reachable.
 */
export function Board({
  tasks,
  teams,
  selected,
  showDone,
  onDoneVisibilityChange,
  onOpen,
  offline = false,
}: {
  tasks: TaskSummary[];
  teams: Map<string, Team>;
  selected?: string;
  showDone: boolean;
  onDoneVisibilityChange: (visible: boolean) => void;
  onOpen: (id: string) => void;
  offline?: boolean;
}) {
  const move = useMoveTask();
  const epics = useEpicMap();
  const toast = useToast();
  const [dragging, setDragging] = useState<TaskSummary | undefined>();
  // Every dnd-kit sensor cancels on the physical Escape code, including pending gestures.
  // Leave the logical key empty so modal/menu Escape handlers do not consume this sensor cancel.
  useEffect(() => {
    if (offline) {
      document.dispatchEvent(new KeyboardEvent("keydown", { code: "Escape", bubbles: true, cancelable: true }));
      setDragging(undefined);
    }
  }, [offline]);
  const [ask, setAsk] = useState<{ id: string; note: string } | undefined>();
  // «Может быть» is a manual backlog: it starts folded and never moves anything by itself.
  const [folded, setFolded] = useState<Set<LaneId>>(() => new Set<LaneId>(["maybe"]));
  const scroller = useRef<HTMLDivElement>(null);
  const lanes = useRef<Partial<Record<LaneId, HTMLElement | null>>>({});
  const { lanes: byLane, cancelled } = groupByLane(tasks);
  // Every lane is on the board; «Готово» is folded unless the owner shows it, and «Может быть» starts folded.
  const listed = LANES;
  const [phoneLane, setPhoneLane] = useState<LaneId>(() => (LANES.find((l) => byLane[l.id].length) ?? LANES[0]).id);

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(TouchSensor, { activationConstraint: { delay: 250, tolerance: 6 } }),
    useSensor(KeyboardSensor),
  );

  const isShut = (lane: Lane) => (lane.id === "done" ? !showDone : folded.has(lane.id));
  const toggleLane = (lane: Lane) => {
    if (lane.id === "done") {
      onDoneVisibilityChange(!showDone);
      return;
    }
    setFolded((current) => {
      const next = new Set(current);
      if (next.has(lane.id)) next.delete(lane.id);
      else next.add(lane.id);
      return next;
    });
  };

  const doMove = (id: string, status: Status, note?: string) => {
    const t = tasks.find((x) => x.id === id);
    if (offline || !t || t.status === status) return;
    move.mutate(
      { id, status, note },
      {
        onSuccess: () => toast(`${id} → ${STATUS_NAME[status]} · оркестратор уведомлён`),
        onError: (e) => toast(`Не удалось: ${e.message}`, "error"),
      },
    );
  };

  // Moving into the owner's lane asks the owner's question first, exactly as the eight-column board did.
  const moveTo = (id: string, status: Status) => {
    if (offline) return;
    if (needsQuestion(status)) setAsk({ id, note: "" });
    else doMove(id, status);
  };

  const onDragEnd = (e: DragEndEvent) => {
    const card = dragging;
    setDragging(undefined);
    if (offline || !card) return;
    const status = resolveDrop(e.over ? String(e.over.id) : undefined, card.status);
    if (status) moveTo(card.id, status);
  };

  // Phone: the chip row above the lanes scrolls the board and follows the lane in view.
  const goLane = (id: LaneId) => {
    setPhoneLane(id);
    lanes.current[id]?.scrollIntoView({ behavior: "smooth", inline: "start", block: "nearest" });
  };
  const onScroll = () => {
    const box = scroller.current;
    if (!box) return;
    const left = box.getBoundingClientRect().left;
    let best = phoneLane;
    let nearest = Infinity;
    for (const lane of listed) {
      const node = lanes.current[lane.id];
      if (!node) continue;
      const distance = Math.abs(node.getBoundingClientRect().left - left);
      if (distance < nearest) {
        nearest = distance;
        best = lane.id;
      }
    }
    if (best !== phoneLane) setPhoneLane(best);
  };

  return (
    <DndContext
      sensors={sensors}
      onDragStart={(e: DragStartEvent) => {
        if (!offline) setDragging(tasks.find((t) => t.id === String(e.active.id)));
      }}
      onDragEnd={onDragEnd}
      onDragCancel={() => setDragging(undefined)}
    >
      <div className="g-board-wrap">
        <div className="g-lane-switch m-only" role="group" aria-label="Дорожки">
          {listed.map((lane) => (
            <Chip key={lane.id} pressed={phoneLane === lane.id} onClick={() => goLane(lane.id)}>
              {lane.name}
              <span className="g-lane-switch-n">{byLane[lane.id].length}</span>
            </Chip>
          ))}
        </div>
        <div className="g-board" ref={scroller} onScroll={onScroll}>
          {listed.map((lane) => (
            <LaneSection
              key={lane.id}
              lane={lane}
              tasks={byLane[lane.id]}
              teams={teams}
              selected={selected}
              collapsed={isShut(lane)}
              dragging={dragging?.status}
              offline={offline}
              onToggle={() => toggleLane(lane)}
              onOpen={onOpen}
              onMove={moveTo}
              laneRef={(node) => {
                lanes.current[lane.id] = node;
              }}
            />
          ))}
          {cancelled.length > 0 && <CancelledStrip tasks={cancelled} teams={teams} selected={selected} offline={offline} onOpen={onOpen} onMove={moveTo} />}
        </div>
      </div>
      <DragOverlay dropAnimation={null}>
        {dragging && (
          <div className={cx("g-sheet bd-card bd-ghost", dragging.needsOwner && "bd-decision")} data-lane-tone={laneOf(dragging.status)?.id} data-epic-tone={dragging.parent && epics.has(dragging.parent) ? epicTone(dragging.parent) : undefined}>
            <CardBody t={dragging} team={dragging.team ? teams.get(dragging.team) : undefined} />
          </div>
        )}
      </DragOverlay>
      {ask && (
        <Dialog
          title="Вопрос к владельцу"
          onClose={() => setAsk(undefined)}
          footer={
            <>
              <Button variant="ghost" onClick={() => setAsk(undefined)}>
                Отмена
              </Button>
              <Button
                variant="primary"
                disabled={offline || !ask.note.trim()}
                onClick={() => {
                  doMove(ask.id, "needs_owner", ask.note.trim());
                  setAsk(undefined);
                }}
              >
                Перенести
              </Button>
            </>
          }
        >
          <p className="g-muted">
            {ask.id} → {STATUS_NAME.needs_owner}
          </p>
          <Field label="Что нужно решить">
            <Textarea autoFocus rows={3} value={ask.note} onChange={(e) => setAsk({ ...ask, note: e.target.value })} />
          </Field>
        </Dialog>
      )}
    </DndContext>
  );
}
