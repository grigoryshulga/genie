// The board's lanes: which statuses each one gathers, and where a dropped card lands. Pure data and
// functions, so the mapping is tested without a browser. Cancelled tasks belong to no lane.

import type { Status, TaskSummary } from "./model.ts";

export type LaneId = "maybe" | "decision" | "prep" | "queue" | "work" | "done";

export interface Lane {
  id: LaneId;
  name: string;
  /** One line under the name. */
  hint: string;
  /** The statuses shown in the lane. */
  statuses: Status[];
  /** The status a card gets when dropped on the lane itself. */
  target: Status;
  /** The exact statuses offered as drop chips while a card is dragged. */
  drops: Status[];
}

export const LANES: Lane[] = [
  { id: "maybe", name: "Может быть", hint: "Входящие: ничего не начнётся само", statuses: ["inbox"], target: "inbox", drops: ["inbox"] },
  { id: "decision", name: "Нужно решение", hint: "Ваш ход", statuses: ["needs_owner"], target: "needs_owner", drops: ["needs_owner"] },
  { id: "prep", name: "Подготовка", hint: "Менеджер проекта готовит задачу", statuses: ["draft", "refining"], target: "refining", drops: ["draft", "refining"] },
  { id: "queue", name: "Очередь", hint: "Готово к работе", statuses: ["ready"], target: "ready", drops: ["ready"] },
  { id: "work", name: "В работе", hint: "Работа · ревью · доработка", statuses: ["in_progress", "review", "changes_requested"], target: "in_progress", drops: ["in_progress", "review"] },
  { id: "done", name: "Готово", hint: "Одобрено · принято", statuses: ["approved", "done"], target: "done", drops: ["approved", "done"] },
];

export const laneOf = (status: Status): Lane | undefined => LANES.find((l) => l.statuses.includes(status));

/** The tasks of every lane, in the given order, plus the cancelled ones that sit outside the lanes. */
export function groupByLane(tasks: TaskSummary[]): { lanes: Record<LaneId, TaskSummary[]>; cancelled: TaskSummary[] } {
  const lanes: Record<LaneId, TaskSummary[]> = { maybe: [], decision: [], prep: [], queue: [], work: [], done: [] };
  const cancelled: TaskSummary[] = [];
  for (const t of tasks) {
    const lane = laneOf(t.status);
    if (lane) lanes[lane.id].push(t);
    else cancelled.push(t);
  }
  return { lanes, cancelled };
}

const LANE_PREFIX = "lane:";
const STATUS_PREFIX = "status:";
export const laneDropId = (lane: Lane): string => `${LANE_PREFIX}${lane.id}`;
export const statusDropId = (status: Status): string => `${STATUS_PREFIX}${status}`;

/**
 * The status a card dragged from `from` gets when dropped on `overId`, or nothing when the drop
 * changes nothing. A lane takes the card to its default status, a status chip to exactly that one.
 */
export function resolveDrop(overId: string | undefined, from: Status): Status | undefined {
  if (!overId) return undefined;
  if (overId.startsWith(STATUS_PREFIX)) {
    const status = overId.slice(STATUS_PREFIX.length) as Status;
    return LANES.some((l) => l.drops.includes(status)) && status !== from ? status : undefined;
  }
  const lane = LANES.find((l) => laneDropId(l) === overId);
  return lane && !lane.statuses.includes(from) ? lane.target : undefined;
}

/** Every status a card can be moved to from `from`, by lane: the "Move to" menu. */
export function moveTargets(from: Status): { lane: Lane; status: Status }[] {
  return LANES.flatMap((lane) => lane.drops.filter((s) => s !== from).map((status) => ({ lane, status })));
}

/** Moving into needs-owner asks the owner's question first. */
export const needsQuestion = (status: Status): boolean => status === "needs_owner";
