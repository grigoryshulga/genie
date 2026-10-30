import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { anchorPopover } from "@/shared/lib";
import { cx } from "./cx.ts";
import { IconButton } from "./actions.tsx";
import { ActionMenu, type MenuEntry } from "./overlays.tsx";
import "./menu-button.css";

/**
 * An icon button that opens a menu of actions under it. The panel is portalled and
 * `position: fixed`, so a scrolling or animated top row cannot clip it; arrows and
 * Home/End move inside the menu, Escape, an outside click or choosing an item closes
 * it and returns focus to the button.
 */
export function MenuButton({
  label,
  icon,
  items,
  className,
  disabled,
}: {
  label: string;
  icon: React.ReactNode;
  items: MenuEntry[];
  className?: string;
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);

  const close = (restoreFocus = true) => {
    setOpen(false);
    if (restoreFocus) button.current?.focus();
  };

  useLayoutEffect(() => {
    if (!open) return;
    const pop = panel.current;
    const btn = button.current;
    if (!pop || !btn) return;
    const place = () => {
      const box = btn.getBoundingClientRect();
      // Right-aligned to the button: the action sits at the end of a page toolbar.
      const { left, top, maxHeight } = anchorPopover({ left: box.right - pop.offsetWidth, bottom: box.bottom }, pop.offsetWidth, {
        width: window.innerWidth,
        height: window.innerHeight,
      });
      pop.style.setProperty("--menu-button-left", `${left}px`);
      pop.style.setProperty("--menu-button-top", `${top}px`);
      pop.style.setProperty("--menu-button-max", `${maxHeight}px`);
    };
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onDown = (event: MouseEvent) => {
      if (!wrap.current?.contains(event.target as Node) && !panel.current?.contains(event.target as Node)) close(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  return (
    <div ref={wrap} className={cx("menu-button", className)}>
      <IconButton ref={button} label={label} aria-haspopup="menu" aria-expanded={open} disabled={disabled} onClick={() => (open ? close(false) : setOpen(true))}>
        {icon}
      </IconButton>
      {open &&
        createPortal(
          <div ref={panel} className="menu-button-panel">
            <ActionMenu label={label} items={items} onClose={() => close()} />
          </div>,
          document.body,
        )}
    </div>
  );
}
