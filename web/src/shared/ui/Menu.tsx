// A "⋯" button with a small menu, for the rare actions on a row of a settings list.

import { useEffect, useRef, useState } from "react";
import { Icon } from "./icons.tsx";

/** A "⋯" button with a small menu under it; closes on a click outside or Escape. */
export function Menu({ label, children }: { label: string; children: (close: () => void) => React.ReactNode }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", away);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", away);
      document.removeEventListener("keydown", esc);
    };
  }, [open]);
  return (
    <div className="rp-menu-wrap" ref={ref}>
      <button type="button" className={open ? "icon-btn on" : "icon-btn"} aria-label={label} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen(!open)}>
        <Icon.dots size={14} />
      </button>
      {open && (
        <div className="rp-menu" role="menu">
          {children(() => setOpen(false))}
        </div>
      )}
    </div>
  );
}
