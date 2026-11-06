import { type KeyboardEvent, type ReactNode, useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Link } from "react-router";
import { anchorPopover } from "@/shared/lib";
import { Icon } from "../icons.tsx";
import { nextIndex } from "./menu-nav.ts";
import "./nav-menu.css";

export type NavMenuEntry = { id: string; label: ReactNode; to: string; icon?: ReactNode; hint?: ReactNode; count?: ReactNode; current?: boolean };

/** How long the panel keeps playing its closing motion before it leaves the DOM; matches `--g-duration`. */
const EXIT_MS = 150;

/**
 * The chevron beside a navigation tab and the page list it opens. The panel is portalled to the body and
 * `position: fixed` under the tab, so neither a scrolling bar nor an animated (transformed) one can clip or
 * shift it. Arrow keys move through the pages, Escape and Tab close it,
 * and closing returns focus to the chevron.
 */
export function NavMenu({ label, items, align = "left" }: { label: string; items: NavMenuEntry[]; align?: "left" | "right" }) {
  const [state, setState] = useState<"closed" | "open" | "closing">("closed");
  const button = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const id = useId();
  const open = state === "open";
  const close = (restoreFocus: boolean) => {
    setState((s) => (s === "open" ? "closing" : s));
    if (restoreFocus) button.current?.focus();
  };

  useEffect(() => {
    if (state !== "closing") return;
    const timer = window.setTimeout(() => setState("closed"), EXIT_MS);
    return () => window.clearTimeout(timer);
  }, [state]);

  useLayoutEffect(() => {
    if (state === "closed") return;
    const pop = panel.current;
    const btn = button.current;
    if (!pop || !btn) return;
    const place = () => {
      // Hang the panel from the whole tab (label and chevron), not only the chevron.
      const tab = (btn.parentElement ?? btn).getBoundingClientRect();
      const anchorLeft = align === "right" ? tab.right - pop.offsetWidth : tab.left;
      const below = anchorPopover({ left: anchorLeft, bottom: tab.bottom }, pop.offsetWidth, { width: window.innerWidth, height: window.innerHeight });
      // A tab in the phone's bottom dock has no room under it: the panel opens upwards instead.
      const up = below.top + pop.scrollHeight > window.innerHeight && tab.top > window.innerHeight - tab.bottom;
      const gap = below.top - tab.bottom;
      const { left } = below;
      const maxHeight = up ? tab.top - 2 * gap : below.maxHeight;
      const top = up ? Math.max(gap, tab.top - Math.min(pop.offsetHeight, maxHeight) - gap) : below.top;
      pop.dataset.side = up ? "top" : "bottom";
      pop.style.setProperty("--nav-menu-top", `${top}px`);
      pop.style.setProperty("--nav-menu-left", `${left}px`);
      pop.style.setProperty("--nav-menu-max", `${maxHeight}px`);
    };
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [state, align]);

  useEffect(() => {
    if (!open) return;
    const links = [...(panel.current?.querySelectorAll<HTMLAnchorElement>("a") ?? [])];
    (links.find((a) => a.getAttribute("aria-current") === "page") ?? links[0])?.focus();
    const away = (e: MouseEvent) => {
      if (!panel.current?.contains(e.target as Node) && !button.current?.contains(e.target as Node)) close(false);
    };
    document.addEventListener("mousedown", away);
    return () => document.removeEventListener("mousedown", away);
  }, [open]); // eslint-disable-line react-hooks/exhaustive-deps

  function onPanelKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      return close(true);
    }
    if (e.key === "Tab") return close(false);
    const links = [...(panel.current?.querySelectorAll<HTMLAnchorElement>("a") ?? [])];
    const at = links.findIndex((a) => a === document.activeElement);
    const next = nextIndex(at, e.key, links.length);
    if (next === at) return;
    e.preventDefault();
    links[next]?.focus();
  }

  return (
    <>
      <button
        ref={button}
        type="button"
        className="g-nav-menu-toggle"
        aria-label={label}
        title={label}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={state !== "closed" ? id : undefined}
        onClick={() => (open ? close(false) : setState("open"))}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown" && !open) {
            e.preventDefault();
            setState("open");
          }
        }}
      >
        <Icon.chevron size={12} />
      </button>
      {state !== "closed" && createPortal(
        <div ref={panel} id={id} className="g-nav-menu" data-state={state} role="menu" aria-label={label} onKeyDown={onPanelKey}>
          {items.map((item, i) => (
            <Link
              key={item.id}
              to={item.to}
              role="menuitem"
              className="g-nav-menu-item"
              aria-current={item.current ? "page" : undefined}
              style={{ "--g-nav-menu-index": i } as React.CSSProperties}
              onClick={() => close(false)}
            >
              {item.icon && <span className="g-nav-menu-icon" aria-hidden="true">{item.icon}</span>}
              <span className="g-nav-menu-text">
                <span className="g-nav-menu-label">{item.label}</span>
                {item.hint && <span className="g-nav-menu-hint">{item.hint}</span>}
              </span>
              {item.count}
            </Link>
          ))}
        </div>,
        document.body,
      )}
    </>
  );
}
