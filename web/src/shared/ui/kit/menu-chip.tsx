import { useEffect, useRef, useState } from "react";
import { Icon } from "../icons.tsx";
import { cx } from "./cx.ts";
import { ActionMenu, type MenuEntry } from "./overlays.tsx";
import "./menu-chip.css";

/**
 * A small chip that names the current choice and opens the options as a menu above it: the control at the
 * bottom of a message box ("how to deliver", "who to"). The menu returns focus to the chip when it closes.
 */
export function MenuChip({ label, items, className, children }: { label: string; items: MenuEntry[]; className?: string; children: React.ReactNode }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLSpanElement>(null);
  const chip = useRef<HTMLButtonElement>(null);
  const close = () => {
    setOpen(false);
    chip.current?.focus();
  };
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => !root.current?.contains(e.target as Node) && setOpen(false);
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);
  return (
    <span className={cx("g-menu-chip", className)} ref={root}>
      <button type="button" ref={chip} className="g-menu-chip-btn" aria-label={label} title={label} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen(!open)}>
        {children}
        <Icon.chevron size={10} />
      </button>
      {open && (
        <span className="g-menu-chip-menu">
          <ActionMenu label={label} items={items} onClose={close} />
        </span>
      )}
    </span>
  );
}
