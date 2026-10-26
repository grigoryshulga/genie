import { type ButtonHTMLAttributes, type KeyboardEvent, type ReactNode, useId, useRef } from "react";
import { Link } from "react-router";
import { cx } from "./cx.ts";
import { nextIndex } from "./menu-nav.ts";

type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";

/** Primary (orange) is "your move" and appears once per view; secondary has a green outline. */
export function Button({ variant = "secondary", sm, busy, className, children, ...rest }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: ButtonVariant; sm?: boolean; busy?: boolean }) {
  return (
    <button type="button" {...rest} className={cx("g-btn", variant !== "secondary" && variant, variant === "secondary" && "secondary", sm && "sm", className)} aria-busy={busy || undefined} disabled={rest.disabled || busy}>
      {children}
    </button>
  );
}

/** An icon-only button must carry a label. */
export function IconButton({ label, className, children, ...rest }: ButtonHTMLAttributes<HTMLButtonElement> & { label: string }) {
  return (
    <button type="button" {...rest} className={cx("g-icon-btn", className)} aria-label={label} title={rest.title ?? label}>
      {children}
    </button>
  );
}

export type TabItem<T extends string> = { id: T; label: ReactNode; to?: string; /** Tooltip for a tab whose label is collapsed to its icon. */ title?: string };

/** Tabs: a rail of pills; the selected one is the only filled pill. Arrow keys move focus, Home and End jump. */
export function Tabs<T extends string>({ items, value, onChange, label }: { items: TabItem<T>[]; value: T; onChange?: (id: T) => void; label: string }) {
  const refs = useRef<(HTMLElement | null)[]>([]);
  const base = useId();
  function onKey(e: KeyboardEvent, index: number) {
    const next = nextIndex(index, e.key, items.length, "horizontal");
    if (next === index) return;
    e.preventDefault();
    refs.current[next]?.focus();
    if (!items[next].to) onChange?.(items[next].id);
  }
  return (
    <div className="g-tabs" role={items.some((i) => i.to) ? undefined : "tablist"} aria-label={label}>
      {items.map((item, i) =>
        item.to ? (
          <Link key={item.id} to={item.to} title={item.title} ref={(el) => void (refs.current[i] = el)} aria-current={item.id === value ? "page" : undefined} onKeyDown={(e) => onKey(e, i)}>
            {item.label}
          </Link>
        ) : (
          <button key={item.id} type="button" role="tab" id={`${base}-${item.id}`} ref={(el) => void (refs.current[i] = el)} aria-selected={item.id === value} tabIndex={item.id === value ? 0 : -1} onClick={() => onChange?.(item.id)} onKeyDown={(e) => onKey(e, i)}>
            {item.label}
          </button>
        ),
      )}
    </div>
  );
}

/** A rail of toggle buttons where exactly one is pressed. */
export function Segmented<T extends string>({ items, value, onChange, label }: { items: { id: T; label: ReactNode }[]; value: T; onChange: (id: T) => void; label: string }) {
  return (
    <div className="g-seg" role="group" aria-label={label}>
      {items.map((item) => (
        <button key={item.id} type="button" aria-pressed={item.id === value} onClick={() => onChange(item.id)}>
          {item.label}
        </button>
      ))}
    </div>
  );
}

/** A filter chip: pressed means the filter is on. */
export function Chip({ pressed, className, children, ...rest }: ButtonHTMLAttributes<HTMLButtonElement> & { pressed?: boolean }) {
  return (
    <button type="button" {...rest} className={cx("g-chip", className)} aria-pressed={pressed}>
      {children}
    </button>
  );
}

type TagTone = "neutral" | "accent" | "signal" | "danger" | "solid-accent" | "solid-signal" | "outline";

export function Tag({ tone = "neutral", className, children, title }: { tone?: TagTone; className?: string; children: ReactNode; title?: string }) {
  return (
    <span className={cx("g-tag", tone !== "neutral" && tone, className)} title={title}>
      {children}
    </span>
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return <kbd className="g-kbd">{children}</kbd>;
}

export function Breadcrumbs({ items }: { items: { label: ReactNode; to?: string }[] }) {
  return (
    <nav className="g-crumbs" aria-label="Путь">
      {items.map((item, i) => {
        const last = i === items.length - 1;
        return (
          <span key={i} style={{ display: "contents" }}>
            {item.to && !last ? <Link to={item.to}>{item.label}</Link> : <span aria-current={last ? "page" : undefined}>{item.label}</span>}
            {!last && <span aria-hidden="true">/</span>}
          </span>
        );
      })}
    </nav>
  );
}

/** A tooltip shown on hover and focus of its child; the text is wired through aria-describedby. */
export function Tooltip({ text, children }: { text: string; children: ReactNode }) {
  const id = useId();
  return (
    <span className="g-tip-host" aria-describedby={id}>
      {children}
      <span role="tooltip" id={id} className="g-tip g-tip-pop">
        {text}
      </span>
    </span>
  );
}
