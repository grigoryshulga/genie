import type { ReactNode } from "react";
import { Character, type CharacterState } from "../character/index.ts";
import { cx } from "./cx.ts";
import "./loading.css";

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

/**
 * One shared empty state: an optional character, a title, one line, an optional action. The character
 * is a pose, not art: `character="sleep"` for "nothing here yet", `character="wake"` only when work
 * has just resumed. Keep it out of dense lists and toolbars, and never show two in the same view.
 */
export function EmptyState({
  title,
  children,
  action,
  character,
  characterSize = 64,
  characterLabel,
  className,
}: {
  title: ReactNode;
  children?: ReactNode;
  action?: ReactNode;
  character?: CharacterState;
  characterSize?: number;
  characterLabel?: string;
  className?: string;
}) {
  return (
    <div className={cx("g-empty", className)}>
      {character && <Character state={character} size={characterSize} label={characterLabel} />}
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

/**
 * What a region shows instead of its content while that content loads: `lines` bars shaped like the
 * text they stand in for, or `children` made of Skeleton blocks when the content is not text (a page
 * header, a card, a tree). Only `label` reaches assistive technology — the wrapper is a polite status
 * and the bars are decorative — so a screen reader hears "Загрузка" once instead of the placeholders.
 * Pass `width` when the parent would otherwise stretch the bars across a whole pane.
 */
export function LoadingState({
  label = "Загрузка",
  lines = 3,
  width,
  className,
  children,
}: {
  label?: string;
  lines?: number;
  width?: number | string;
  className?: string;
  children?: ReactNode;
}) {
  return (
    <div className={cx("g-loading", className)} style={width === undefined ? undefined : { maxWidth: width }} role="status">
      <span className="g-sr">{label}</span>
      {children ?? Array.from({ length: lines }, (_, line) => <Skeleton key={line} height={14} width={line % 2 ? "62%" : "88%"} />)}
    </div>
  );
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
