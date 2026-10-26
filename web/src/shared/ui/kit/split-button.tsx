import { type ButtonHTMLAttributes, useEffect, useRef, useState } from "react";
import { Icon } from "../icons.tsx";
import { cx } from "./cx.ts";
import { ActionMenu, type MenuEntry } from "./overlays.tsx";
import "./split-button.css";

/**
 * An action with options: the main part does it the chosen way, the chevron beside it opens the
 * options as a menu above. The options stay out of sight until someone needs them; the chevron
 * works even while the main part is disabled, so a choice can be made before there is anything to
 * act on. The menu returns focus to the chevron when it closes.
 */
export function SplitButton({ menuLabel, items, className, children, ...main }: ButtonHTMLAttributes<HTMLButtonElement> & { menuLabel: string; items: MenuEntry[] }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLSpanElement>(null);
  const more = useRef<HTMLButtonElement>(null);
  const close = () => {
    setOpen(false);
    more.current?.focus();
  };
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => !root.current?.contains(e.target as Node) && setOpen(false);
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);
  return (
    <span className={cx("g-split", className)} ref={root}>
      <button type="button" {...main} className="g-split-main">
        {children}
      </button>
      <button type="button" ref={more} className="g-split-more" aria-label={menuLabel} title={menuLabel} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen(!open)}>
        <Icon.chevron size={10} />
      </button>
      {open && (
        <span className="g-split-menu">
          <ActionMenu label={menuLabel} items={items} onClose={close} />
        </span>
      )}
    </span>
  );
}
