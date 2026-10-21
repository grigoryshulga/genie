import type { ReactNode } from "react";
import { cx } from "./cx.ts";

type Tone = "neutral" | "accent" | "signal" | "danger";

/** A banner reports state in words; role="status" for information, role="alert" for danger. */
export function Banner({ tone = "neutral", title, children, action }: { tone?: Tone; title?: ReactNode; children?: ReactNode; action?: ReactNode }) {
  return (
    <div className={cx("g-banner", tone !== "neutral" && tone)} role={tone === "danger" ? "alert" : "status"}>
      <div style={{ flex: 1 }}>
        {title && <strong>{title}</strong>}
        {children}
      </div>
      {action}
    </div>
  );
}

/** One shared empty state: a title, one line, an optional action. */
export function EmptyState({ title, children, action }: { title: ReactNode; children?: ReactNode; action?: ReactNode }) {
  return (
    <div className="g-empty">
      {/* A level-2 heading: an empty state always follows the page title, and h1 → h3 skips a level. */}
      <h2 className="g-h3">{title}</h2>
      {children && <p>{children}</p>}
      {action}
    </div>
  );
}

/** A placeholder block that shimmers; size it with width and height. Not announced to screen readers. */
export function Skeleton({ width = "100%", height = 14, className }: { width?: number | string; height?: number | string; className?: string }) {
  return <div className={cx("g-skeleton", className)} style={{ width, height }} aria-hidden="true" />;
}

/** One shared error block: the server's message and a retry. */
export function ErrorState({ message, onRetry }: { message: string; onRetry?: () => void }) {
  return (
    <Banner tone="danger" title="Не получилось" action={onRetry && <button type="button" className="g-btn sm secondary" onClick={onRetry}>Повторить</button>}>
      {message}
    </Banner>
  );
}

/** Concentric arcs: an agent is working here. */
export function WorkingArcs({ size = 20, label = "Идёт работа" }: { size?: number; label?: string }) {
  return (
    <svg className="g-working st-accent" width={size} height={size} viewBox="0 0 16 16" fill="currentColor" role="img" aria-label={label}>
      <path d="M1 15A14 14 0 0 1 15 1v3.2A10.8 10.8 0 0 0 4.2 15z" />
      <path d="M5.8 15A9.2 9.2 0 0 1 15 5.8V9a6 6 0 0 0-6 6z" />
      <path d="M10.2 15A4.8 4.8 0 0 1 15 10.2V15z" />
    </svg>
  );
}
