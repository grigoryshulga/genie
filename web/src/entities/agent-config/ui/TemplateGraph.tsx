import { type PointerEvent, type ReactNode, useId, useRef, useState } from "react";
import "./graph.css";
import { capitalized, type LiveState, layoutTeam, type MemberDef, NODE_H, ON_TITLE, type Relation, RELATION_TITLE, type RoleDef } from "../model.ts";

/** How a relation is drawn: by its kind in a template, by what has happened in a running team. */
type Look = "handoff" | "returns" | "reports" | "consults" | "done" | "pending" | "later";

const LOOK: Record<Look, { color: string; dash?: string; legend: string }> = {
  handoff: { color: "var(--accent-2)", legend: "передаёт работу" },
  returns: { color: "var(--amber)", legend: "возвращает" },
  reports: { color: "var(--muted)", dash: "4 4", legend: "докладывает оркестратору" },
  consults: { color: "var(--violet)", dash: "2 4", legend: "советуется" },
  done: { color: "var(--accent-2)", legend: "передача состоялась" },
  pending: { color: "var(--amber)", dash: "6 5", legend: "её ждут" },
  later: { color: "var(--off)", dash: "4 4", legend: "другие связи" },
};

const STATE_TITLE: Record<LiveState, string> = { working: "работает", waiting: "ждёт", idle: "свободен", error: "ошибка", stopped: "остановлен" };

/** Text left of this x is the avatar. */
const TEXT_X = 36;

let measurer: CanvasRenderingContext2D | null | undefined;
/** Width of a text in the graph's font (an estimate until the canvas is there). */
function textWidth(text: string, font: string): number {
  if (measurer === undefined) measurer = typeof document === "undefined" ? null : document.createElement("canvas").getContext("2d");
  if (!measurer) return text.length * 7;
  measurer.font = `${font} Geist, system-ui, sans-serif`;
  return measurer.measureText(text).width;
}

/** Cut a text to a width with an ellipsis. */
function fit(text: string, width: number, font: string): string {
  if (textWidth(text, font) <= width) return text;
  let s = text;
  while (s.length > 1 && textWidth(`${s}…`, font) > width) s = s.slice(0, -1);
  return `${s.trimEnd()}…`;
}

/** A label in one line, or two when there is room (no second line under it). */
function lines(text: string, width: number, two: boolean): string[] {
  const font = "500 12.5px";
  if (!two || textWidth(text, font) <= width) return [fit(text, width, font)];
  const words = text.split(" ");
  let first = words[0];
  let i = 1;
  while (i < words.length && textWidth(`${first} ${words[i]}`, font) <= width) first += ` ${words[i++]}`;
  const rest = words.slice(i).join(" ");
  return rest ? [fit(first, width, font), fit(rest, width, font)] : [fit(first, width, font)];
}

/**
 * The team of a template as a graph: who hands work to whom, who returns it,
 * who reports. With `live`, a running team: each member's state (and whom it
 * waits for); `pending` marks the handoffs still awaited and `done` the ones
 * that have happened (`from->to` keys). With `onConnect`, a relation is drawn by
 * dragging from one member to another (or to the orchestrator).
 */
export function TemplateGraph({
  members,
  relations,
  roles,
  problems = [],
  live,
  pending = [],
  done = [],
  head,
  onConnect,
}: {
  members: MemberDef[];
  relations: Relation[];
  roles: RoleDef[];
  problems?: string[];
  live?: Record<string, { state: LiveState; note?: string }>;
  pending?: string[];
  done?: string[];
  /** A line above the graph (what the team waits for). */
  head?: ReactNode;
  onConnect?: (from: string, to: string) => void;
}) {
  const uid = useId().replace(/[^a-zA-Z0-9]/g, "");
  const svgRef = useRef<SVGSVGElement>(null);
  const [drag, setDrag] = useState<{ from: string; x: number; y: number }>();
  const role = (m: MemberDef) => roles.find((r) => r.id === m.role);
  const title = (m: MemberDef) => role(m)?.title ?? m.role;
  const g = layoutTeam(
    members.map((m) => {
      const l = live?.[m.key];
      // A running team: the member's name, and its state when it is not just free.
      const sub = l
        ? l.state === "waiting" && l.note
          ? l.note
          : l.state === "idle" || l.state === "stopped"
            ? title(m).toLowerCase()
            : STATE_TITLE[l.state]
        : m.name
          ? title(m)
          : m.key !== m.role
            ? m.key
            : "";
      return { key: m.key, label: m.name ? capitalized(m.name) : title(m), sub };
    }),
    relations,
    problems,
  );
  const w = g.nodeW;
  const classOf = (key: string) => (key === "orchestrator" ? "orchestrator" : (role(members.find((m) => m.key === key)!)?.class ?? ""));
  const nameOf = (key: string) => g.nodes.find((n) => n.key === key)?.label ?? key;
  const lookOf = (e: (typeof g.edges)[number]): Look => {
    if (!live) return e.type;
    const k = `${e.from}->${e.to}`;
    return pending.includes(k) ? "pending" : done.includes(k) ? "done" : "later";
  };
  const looks = [...new Set(g.edges.map(lookOf))].sort((a, b) => Object.keys(LOOK).indexOf(a) - Object.keys(LOOK).indexOf(b));
  /** The pointer in the graph's coordinates. */
  const at = (e: PointerEvent) => {
    const m = svgRef.current?.getScreenCTM();
    if (!m) return { x: 0, y: 0 };
    const p = new DOMPoint(e.clientX, e.clientY).matrixTransform(m.inverse());
    return { x: p.x, y: p.y };
  };
  const nodeAt = (x: number, y: number) => g.nodes.find((n) => Math.abs(n.x - x) <= (n.orchestrator ? 75 : w / 2) && Math.abs(n.y - y) <= NODE_H / 2);
  const start = onConnect
    ? (key: string) => (e: PointerEvent) => {
        if (key === "orchestrator") return;
        svgRef.current?.setPointerCapture(e.pointerId);
        setDrag({ from: key, ...at(e) });
      }
    : undefined;
  const from = drag && g.nodes.find((n) => n.key === drag.from);
  return (
    <figure className="team-graph">
      {head && <div className="team-graph-head">{head}</div>}
      <div className="team-graph-scroll">
        <svg
          ref={svgRef}
          width={g.width}
          height={g.height}
          viewBox={`0 0 ${g.width} ${g.height}`}
          style={{ maxWidth: g.width, minWidth: Math.round(g.width * 0.75) }}
          role="img"
          aria-label="Связи участников"
          className={onConnect ? "connectable" : undefined}
          onPointerMove={drag ? (e) => setDrag({ ...drag, ...at(e) }) : undefined}
          onPointerUp={
            drag
              ? (e) => {
                  const p = at(e);
                  const target = nodeAt(p.x, p.y);
                  if (target && target.key !== drag.from) onConnect?.(drag.from, target.key);
                  setDrag(undefined);
                }
              : undefined
          }
          onPointerCancel={() => setDrag(undefined)}
        >
          <defs>
            {(Object.keys(LOOK) as Look[]).map((k) => (
              <marker key={k} id={`${uid}-${k}`} viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
                <path d="M 0 0 L 10 5 L 0 10 z" style={{ fill: LOOK[k].color }} />
              </marker>
            ))}
          </defs>
          {g.edges.map((e, i) => {
            const look = lookOf(e);
            // One relation to several members is labelled once.
            const again = g.edges.slice(0, i).some((x) => x.from === e.from && x.type === e.type && x.on === e.on && lookOf(x) === look);
            const label = e.on && look !== "later" && !again ? (ON_TITLE[e.on] ?? e.on) : undefined;
            return (
              <g key={i} className={`edge ${look}`}>
                <title>{`${nameOf(e.from)} → ${nameOf(e.to)}: ${RELATION_TITLE[e.type]}${e.on ? `, когда задача «${ON_TITLE[e.on] ?? e.on}»` : ""}${e.note ? ` — ${e.note}` : ""}`}</title>
                <path d={e.d} fill="none" style={{ stroke: LOOK[look].color }} strokeWidth={1.6} strokeDasharray={LOOK[look].dash} markerEnd={`url(#${uid}-${look})`} />
                {label && (
                  <text x={e.lx} y={e.ly + 4} textAnchor="middle" className="edge-label" style={{ fill: look === "done" ? "var(--muted)" : LOOK[look].color }}>
                    {label}
                  </text>
                )}
              </g>
            );
          })}
          {from && drag && <line className="drag-line" x1={from.x} y1={from.y} x2={drag.x} y2={drag.y} markerEnd={`url(#${uid}-handoff)`} />}
          {g.nodes.map((n) => {
            const nw = n.orchestrator ? 150 : w;
            const state = live?.[n.key]?.state;
            const spin = state === "working";
            const room = nw - TEXT_X - 10;
            const label = lines(n.label, room, !n.sub);
            return (
              <g
                key={n.key}
                className={`node${n.orchestrator ? " orch" : ""}${n.flagged ? " flagged" : ""}${state ? ` live-${state}` : ""}`}
                transform={`translate(${n.x - nw / 2} ${n.y - NODE_H / 2})`}
                onPointerDown={start?.(n.key)}
              >
                <title>{`${n.label}${n.sub ? ` — ${n.sub}` : ""}${state && state !== "waiting" && live?.[n.key].note ? `\n${live[n.key].note}` : ""}`}</title>
                <rect width={nw} height={NODE_H} rx={10} />
                <circle cx={19} cy={NODE_H / 2} r={9.5} className={`dot c-${classOf(n.key)}`} />
                <text x={19} y={NODE_H / 2 + 3.8} textAnchor="middle" className="dl">
                  {n.label.slice(0, 1).toUpperCase()}
                </text>
                {n.sub ? (
                  <>
                    <text x={TEXT_X} y={20} className="nl">
                      {label[0]}
                    </text>
                    {spin && <circle className="nspin" cx={TEXT_X + 4} cy={31} r={3.6} />}
                    <text x={TEXT_X + (spin ? 12 : 0)} y={35} className="ns">
                      {fit(n.sub, room - (spin ? 12 : 0), "11px")}
                    </text>
                  </>
                ) : (
                  <text className="nl">
                    {label.map((line, i) => (
                      <tspan key={i} x={TEXT_X} y={label.length > 1 ? 20 + i * 14.5 : NODE_H / 2 + 4.5}>
                        {line}
                      </tspan>
                    ))}
                  </text>
                )}
              </g>
            );
          })}
        </svg>
      </div>
      {looks.length > 0 && (
        <figcaption className="team-graph-legend">
          {looks.map((k) => (
            <span key={k}>
              <i style={{ borderTop: `2px ${LOOK[k].dash ? (k === "consults" ? "dotted" : "dashed") : "solid"} ${LOOK[k].color}` }} />
              {LOOK[k].legend}
            </span>
          ))}
        </figcaption>
      )}
    </figure>
  );
}
