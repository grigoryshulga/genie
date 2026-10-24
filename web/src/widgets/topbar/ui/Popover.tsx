import { type ReactNode, useEffect, useId, useRef, useState } from "react";

/**
 * A button that opens a popover. Outside click and Escape close it and return focus to the button;
 * the popover content gets a `close` callback. The trigger carries aria-haspopup and aria-expanded.
 */
export function Popover({
  trigger,
  label,
  className,
  align = "left",
  popup = "menu",
  children,
}: {
  trigger: ReactNode;
  label: string;
  className?: string;
  align?: "left" | "right";
  popup?: "menu" | "dialog";
  children: (close: () => void) => ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const id = useId();
  const close = (restoreFocus = true) => {
    setOpen(false);
    if (restoreFocus) button.current?.focus();
  };
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <div className="g-popover-host" ref={root}>
      <button ref={button} type="button" className={className} aria-label={label} title={label} aria-haspopup={popup} aria-expanded={open} aria-controls={open ? id : undefined} onClick={() => setOpen(!open)}>
        {trigger}
      </button>
      {open && (
        <div id={id} className={`g-popover ${align}`}>
          {children(close)}
        </div>
      )}
    </div>
  );
}
