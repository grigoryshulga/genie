// Template diagrams use independent ports and obstacle-aware orthogonal routes.
import { handoffRanks, NODE_H, ON_TITLE, type GraphEdge, type GraphNode, type Relation } from "./model.ts";

export const TEMPLATE_NODE_W = 200;
export const TEMPLATE_GAP_X = 80;
const GAP_Y = 160;
const TRACK = 12;
type Point = { x: number; y: number };
type Rect = { left: number; right: number; top: number; bottom: number };
type Segment = { a: Point; b: Point };
type Side = "left" | "right" | "top" | "bottom";
type Connection = { relation: Relation; to: string; source: GraphNode; target: GraphNode; sides: [Side, Side]; ports: Point[]; stubs: Point[] };
const round = (value: number) => Math.round(value * 1000) / 1000;
const margin = (columns: number) => columns === 1 ? 64 : columns === 2 ? 40 : 20;
const widthFor = (columns: number) => columns * TEMPLATE_NODE_W + (columns - 1) * TEMPLATE_GAP_X + margin(columns) * 2;

export function templateColumns(width: number): number {
  let columns = Math.max(1, Math.floor((width + TEMPLATE_GAP_X - 40) / (TEMPLATE_NODE_W + TEMPLATE_GAP_X)));
  while (columns > 1 && widthFor(columns) > width) columns--;
  return columns;
}

const bounds = (n: GraphNode, pad = 0): Rect => ({ left: n.x - TEMPLATE_NODE_W / 2 - pad, right: n.x + TEMPLATE_NODE_W / 2 + pad, top: n.y - NODE_H / 2 - pad, bottom: n.y + NODE_H / 2 + pad });
const segments = (points: Point[]): Segment[] => points.slice(1).map((b, i) => ({ a: points[i], b }));
const horizontal = (s: Segment) => s.a.y === s.b.y;
const overlap = (a: number, b: number, c: number, d: number) => Math.min(Math.max(a, b), Math.max(c, d)) - Math.max(Math.min(a, b), Math.min(c, d));
function hitsRect(s: Segment, r: Rect): boolean {
  return horizontal(s) ? s.a.y > r.top && s.a.y < r.bottom && overlap(s.a.x, s.b.x, r.left, r.right) > 0 : s.a.x > r.left && s.a.x < r.right && overlap(s.a.y, s.b.y, r.top, r.bottom) > 0;
}
function conflicts(a: Segment, b: Segment, spacing: number): boolean {
  if (horizontal(a) !== horizontal(b)) return false;
  return horizontal(a) ? Math.abs(a.a.y - b.a.y) < spacing && overlap(a.a.x, a.b.x, b.a.x, b.b.x) > 0 : Math.abs(a.a.x - b.a.x) < spacing && overlap(a.a.y, a.b.y, b.a.y, b.b.y) > 0;
}
function crosses(a: Segment, b: Segment): boolean {
  if (horizontal(a) === horizontal(b)) return false;
  const h = horizontal(a) ? a : b, v = horizontal(a) ? b : a;
  return v.a.x >= Math.min(h.a.x, h.b.x) && v.a.x <= Math.max(h.a.x, h.b.x) && h.a.y >= Math.min(v.a.y, v.b.y) && h.a.y <= Math.max(v.a.y, v.b.y);
}

/** A small binary heap keeps interactive layouts independent of graph-library bundles. */
class Queue {
  private values: { id: number; cost: number; score: number }[] = [];
  push(value: { id: number; cost: number; score: number }) {
    const values = this.values;
    let index = values.length;
    values.push(value);
    while (index > 0) { const parent = (index - 1) >> 1; if (values[parent].score <= value.score) break; values[index] = values[parent]; index = parent; }
    values[index] = value;
  }
  pop() {
    const values = this.values, first = values[0], last = values.pop();
    if (!values.length || !last) return first;
    let index = 0;
    while (index * 2 + 1 < values.length) {
      let child = index * 2 + 1;
      if (child + 1 < values.length && values[child + 1].score < values[child].score) child++;
      if (values[child].score >= last.score) break;
      values[index] = values[child]; index = child;
    }
    values[index] = last;
    return first;
  }
}

/** Search the visibility grid; shared runs are forbidden, crossings are a last resort. */
function route(start: Point, end: Point, obstacles: Rect[], used: Segment[], reserved: Segment[], width: number, height: number, crossings: boolean, spacing = 8): Point[] | undefined {
  const xx = new Set([4, width - 4, start.x, end.x]), yy = new Set([4, height - 4, start.y, end.y]);
  for (const r of obstacles) for (const offset of [6, 18, 30, 42]) {
    xx.add(round(r.left - offset)); xx.add(round(r.right + offset));
    yy.add(round(r.top - offset)); yy.add(round(r.bottom + offset));
  }
  for (const s of [...used, ...reserved]) { xx.add(s.a.x); xx.add(s.b.x); yy.add(s.a.y); yy.add(s.b.y); }
  const xs = [...xx].filter((x) => x >= 4 && x <= width - 4).sort((a, b) => a - b);
  const ys = [...yy].filter((y) => y >= 4 && y <= height - 4).sort((a, b) => a - b);
  const nx = xs.length, size = nx * ys.length * 3;
  const costs = new Float64Array(size).fill(Infinity), previous = new Int32Array(size).fill(-1);
  const startId = (ys.indexOf(start.y) * nx + xs.indexOf(start.x)) * 3;
  const endCell = ys.indexOf(end.y) * nx + xs.indexOf(end.x);
  const queue = new Queue(); costs[startId] = 0; queue.push({ id: startId, cost: 0, score: 0 });
  const checked = new Map<string, number>();
  for (let state = queue.pop(); state; state = queue.pop()) {
    if (state.cost !== costs[state.id]) continue;
    const cell = Math.floor(state.id / 3), direction = state.id % 3, xi = cell % nx, yi = Math.floor(cell / nx);
    const a = { x: xs[xi], y: ys[yi] };
    if (cell === endCell) {
      const points: Point[] = [];
      for (let id = state.id; id >= 0; id = previous[id]) { const c = Math.floor(id / 3); points.push({ x: xs[c % nx], y: ys[Math.floor(c / nx)] }); }
      return points.reverse();
    }
    for (const [x, y, axis] of [[xi - 1, yi, 1], [xi + 1, yi, 1], [xi, yi - 1, 2], [xi, yi + 1, 2]]) {
      if (x < 0 || x >= nx || y < 0 || y >= ys.length) continue;
      const nextCell = y * nx + x, b = { x: xs[x], y: ys[y] }, segment = { a, b };
      const key = cell < nextCell ? `${cell}:${nextCell}` : `${nextCell}:${cell}`;
      let penalty = checked.get(key);
      if (penalty === undefined) {
        const blocked = obstacles.some((r) => hitsRect(segment, r)) || [...used, ...reserved].some((s) => conflicts(segment, s, spacing));
        const crossing = used.filter((s) => crosses(segment, s)).length;
        penalty = blocked || (!crossings && crossing) ? Infinity : crossing * 120;
        checked.set(key, penalty);
      }
      if (!Number.isFinite(penalty)) continue;
      const cost = state.cost + Math.abs(a.x - b.x) + Math.abs(a.y - b.y) + (direction && direction !== axis ? 24 : 0) + penalty;
      const id = nextCell * 3 + axis;
      if (cost >= costs[id]) continue;
      costs[id] = cost; previous[id] = state.id;
      queue.push({ id, cost, score: cost + Math.abs(b.x - end.x) + Math.abs(b.y - end.y) });
    }
  }
}

function simplify(points: Point[]): Point[] {
  const out: Point[] = [];
  for (const point of points) {
    const last = out.at(-1), before = out.at(-2);
    if (last && point.x === last.x && point.y === last.y) continue;
    if (before && last && ((before.x === last.x && last.x === point.x) || (before.y === last.y && last.y === point.y))) out.pop();
    out.push(point);
  }
  return out;
}
function path(points: Point[]): string {
  let d = `M ${points[0].x} ${points[0].y}`;
  for (let i = 1; i < points.length; i++) {
    const p = points[i], before = points[i - 1], after = points[i + 1];
    if (!after) { d += ` L ${p.x} ${p.y}`; continue; }
    const distance = Math.min(6, (Math.abs(p.x - before.x) + Math.abs(p.y - before.y)) / 2, (Math.abs(after.x - p.x) + Math.abs(after.y - p.y)) / 2);
    const entry = { x: p.x + Math.sign(before.x - p.x) * distance, y: p.y + Math.sign(before.y - p.y) * distance };
    const exit = { x: p.x + Math.sign(after.x - p.x) * distance, y: p.y + Math.sign(after.y - p.y) * distance };
    d += ` L ${entry.x} ${entry.y} Q ${p.x} ${p.y} ${exit.x} ${exit.y}`;
  }
  return d;
}

export function layoutTemplate(members: { key: string; label: string; sub: string }[], relations: Relation[], problems: string[] = [], columns = 3) {
  const ranks = handoffRanks(members.map((m) => m.key), relations);
  // Preserve the handoff ranks, including parallel participants, while wrapping wide teams.
  const stagesCount = Math.max(1, ...[...ranks.values()].map((rank) => rank + 1));
  const cols = Math.max(1, Math.min(stagesCount, columns)), inset = margin(cols);
  const parallel = Math.max(1, ...Array.from({ length: stagesCount }, (_, rank) => [...ranks.values()].filter((value) => value === rank).length));
  const rowPitch = parallel * (NODE_H + GAP_Y);
  const x = (rank: number) => inset + TEMPLATE_NODE_W / 2 + (rank % cols) * (TEMPLATE_NODE_W + TEMPLATE_GAP_X);
  const y = (rank: number) => 96 + Math.floor(rank / cols) * rowPitch;
  const rows = new Map<number, number>();
  const nodes: GraphNode[] = members.map((m) => {
    const rank = ranks.get(m.key)!, row = rows.get(rank) ?? 0;
    rows.set(rank, row + 1);
    return { ...m, x: x(rank), y: y(rank) + row * (NODE_H + GAP_Y), flagged: problems.some((p) => p.includes(`\`${m.key}\``)) };
  });
  nodes.push({ key: "orchestrator", label: "Менеджер задачи", sub: "", x: x(stagesCount), y: y(stagesCount), orchestrator: true });
  const stages = Array.from({ length: stagesCount }, (_, rank) => ({ x: x(rank), y: y(rank) - NODE_H / 2 - 32, label: `Этап ${rank + 1}` }));
  let width = widthFor(cols), height = Math.max(...nodes.map((n) => n.y)) + NODE_H / 2 + 64;
  const at = new Map(nodes.map((n) => [n.key, n]));
  const connections: Connection[] = relations.flatMap((relation) => relation.to.flatMap((to): Connection[] => {
    const source = at.get(relation.from), target = at.get(to);
    if (!source || !target || source === target) return [];
    let sides: [Side, Side];
    if (source.y === target.y) {
      const between = nodes.some((n) => n !== source && n !== target && n.y === source.y && n.x > Math.min(source.x, target.x) && n.x < Math.max(source.x, target.x));
      sides = between ? ["bottom", "bottom"] : target.x > source.x ? ["right", "left"] : ["left", "right"];
    } else sides = target.y > source.y ? ["bottom", "top"] : ["top", "bottom"];
    return [{ relation, to, source, target, sides, ports: [], stubs: [] }];
  }));
  const ports = new Map<string, { edge: Connection; end: number }[]>();
  for (const edge of connections) for (const end of [0, 1]) {
    const key = `${end ? edge.target.key : edge.source.key}:${edge.sides[end]}`;
    ports.set(key, [...ports.get(key) ?? [], { edge, end }]);
  }
  for (const group of ports.values()) group.forEach(({ edge, end }, index) => {
    const node = end ? edge.target : edge.source, side = edge.sides[end];
    const across = side === "left" || side === "right";
    const offset = ((index + 1) / (group.length + 1) - 0.5) * (across ? NODE_H - 32 : TEMPLATE_NODE_W - 40);
    const port = { x: round(node.x + (across ? (side === "left" ? -1 : 1) * TEMPLATE_NODE_W / 2 : offset)), y: round(node.y + (across ? offset : (side === "top" ? -1 : 1) * NODE_H / 2)) };
    edge.ports[end] = port;
    edge.stubs[end] = { x: port.x + (side === "left" ? -TRACK : side === "right" ? TRACK : 0), y: port.y + (side === "top" ? -TRACK : side === "bottom" ? TRACK : 0) };
  });
  const labels: Rect[] = stages.map((s) => ({ left: s.x - 44, right: s.x + 44, top: s.y - 16, bottom: s.y + 4 }));
  const obstacles = nodes.map((n) => bounds(n, 6));
  const used: Segment[] = [], edges: (GraphEdge & { points: Point[] })[] = [];
  const shown = new Set<string>();
  for (const edge of connections) {
    const reserved = connections.filter((c) => c !== edge).flatMap((c) => c.ports.map((port, end) => ({ a: port, b: c.stubs[end] })));
    let middle: Point[] | undefined;
    for (const [crossings, spacing] of [[false, 8], [true, 8], [true, 1]] as const) {
      middle = route(edge.stubs[0], edge.stubs[1], [...obstacles, ...labels], used, reserved, width, height, crossings, spacing);
      if (middle) break;
    }
    // More outside tracks remain available for unusually dense, user-authored teams.
    for (let retry = 0; !middle && retry < 3; retry++) {
      width += TRACK * 4; height += TRACK * 4;
      middle = route(edge.stubs[0], edge.stubs[1], [...obstacles, ...labels], used, reserved, width, height, true, 1);
    }
    // Keep card clearance even if an exceptionally dense graph exhausts separate tracks.
    middle ??= route(edge.stubs[0], edge.stubs[1], obstacles, [], [], width, height, true, 1);
    const points = simplify([edge.ports[0], ...middle ?? [edge.stubs[0], { x: width - TRACK, y: edge.stubs[0].y }, { x: width - TRACK, y: edge.stubs[1].y }, edge.stubs[1]], edge.ports[1]]);
    const own = segments(points), group = `${edge.relation.from}:${edge.relation.type}:${edge.relation.on}`;
    let label: Point | undefined;
    if (edge.relation.on && !shown.has(group)) {
      const labelWidth = (ON_TITLE[edge.relation.on] ?? edge.relation.on).length * 7 + 12;
      const candidates = own.filter(horizontal).sort((a, b) => Math.abs(b.b.x - b.a.x) - Math.abs(a.b.x - a.a.x));
      for (const segment of candidates) {
        if (Math.abs(segment.b.x - segment.a.x) < labelWidth) continue;
        const point = { x: (segment.a.x + segment.b.x) / 2, y: segment.a.y };
        const box = { left: point.x - labelWidth / 2, right: point.x + labelWidth / 2, top: point.y - 10, bottom: point.y + 10 };
        if ([...obstacles, ...labels].some((r) => overlap(r.left, r.right, box.left, box.right) > 0 && overlap(r.top, r.bottom, box.top, box.bottom) > 0) || [...used, ...reserved, ...own.filter((s) => s !== segment)].some((s) => hitsRect(s, box))) continue;
        label = point; labels.push(box); shown.add(group); break;
      }
    }
    edges.push({ from: edge.relation.from, to: edge.to, type: edge.relation.type, on: edge.relation.on, note: edge.relation.note, points, d: path(points), lx: label?.x ?? 0, ly: label?.y ?? 0, showLabel: !!label });
    used.push(...own);
  }
  return { width, height, nodeW: TEMPLATE_NODE_W, nodes, edges, stages };
}
