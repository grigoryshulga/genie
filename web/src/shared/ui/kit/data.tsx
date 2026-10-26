import type { ReactNode } from "react";
import { cx } from "./cx.ts";

export function Card({ flush, className, children }: { flush?: boolean; className?: string; children: ReactNode }) {
  return <div className={cx("g-card", flush && "flush", className)}>{children}</div>;
}

/** The contrasting surface for work items on the canvas: paper on ink, ink on paper. */
export function Sheet({ interactive, className, children }: { interactive?: boolean; className?: string; children: ReactNode }) {
  return <div className={cx("g-sheet", interactive && "interactive", className)}>{children}</div>;
}

/** A big count with a caption, for lane and section totals. */
export function Stat({ value, label }: { value: ReactNode; label: ReactNode }) {
  return (
    <div>
      <div className="g-stat">{value}</div>
      <div className="g-caption">{label}</div>
    </div>
  );
}

/** Agents are rounded squares, people are circles, so who acted reads at a glance. */
/** `tone`: an agent's role colour (`--g-agent-<tone>`); without it, or for a person, the kind's own colour. */
export function Avatar({ kind, name, sm, tone }: { kind: "agent" | "human"; name: string; sm?: boolean; tone?: string }) {
  const initials = name.trim().split(/\s+/).slice(0, 2).map((p) => p[0]?.toUpperCase() ?? "").join("");
  return (
    <span className={cx("g-avatar", kind, sm && "sm")} data-tone={kind === "agent" ? tone : undefined} role="img" aria-label={name} title={name}>
      {initials}
    </span>
  );
}

export function Progress({ value, label }: { value: number; label: string }) {
  const pct = Math.max(0, Math.min(100, Math.round(value * 100)));
  return (
    <div className="g-progress" role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct}>
      <i style={{ width: `${pct}%` }} />
    </div>
  );
}

/** One segment per criterion or task; `on` marks the finished ones. */
export function Segments({ total, done, label }: { total: number; done: number; label: string }) {
  return (
    <div className="g-segs" role="img" aria-label={`${label}: ${done} из ${total}`}>
      {Array.from({ length: total }, (_, i) => (
        <i key={i} className={i < done ? "on" : undefined} />
      ))}
    </div>
  );
}

export function GroupHead({ children, count }: { children: ReactNode; count?: number }) {
  return (
    <div className="g-group-head">
      {children}
      {count !== undefined && <span className="n">{count}</span>}
    </div>
  );
}

/** Rendered Markdown (documents, task descriptions, agent output) in the reading style. */
export function Prose({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={cx("g-prose", className)}>{children}</div>;
}
