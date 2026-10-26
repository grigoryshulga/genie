import { type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";
import { anchorPopover } from "@/shared/lib";
import { cx } from "./cx.ts";
import { useModalFocus } from "./modal-focus.ts";
import "./popover.css";

/**
 * A toolbar button that opens a panel of controls underneath it (a bottom sheet on a phone).
 * The panel is `position: fixed`, so a scrolling toolbar cannot clip it; its place comes from the button
 * and follows the window and any scroll while it is open.
 */
export function Popover({ label, icon, count, children, className, iconOnly = false }: { label: string; icon?: ReactNode; count?: number; children: ReactNode; className?: string; iconOnly?: boolean }) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const btnRef = useRef<HTMLButtonElement>(null);
  const popRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    if (!open) return;
    const pop = popRef.current;
    const btn = btnRef.current;
    if (!pop || !btn) return;
    const place = () => {
      const b = btn.getBoundingClientRect();
      const { left, top, maxHeight } = anchorPopover({ left: b.left, bottom: b.bottom }, pop.offsetWidth, {
        width: window.innerWidth,
        height: window.innerHeight,
      });
      pop.style.setProperty("--fpop-top", `${top}px`);
      pop.style.setProperty("--fpop-left", `${left}px`);
      pop.style.setProperty("--fpop-max", `${maxHeight}px`);
    };
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [open]);

  // Focus goes in when it opens, Tab stays inside, Escape closes and hands focus back to the button.
  useModalFocus(popRef, open, () => setOpen(false));

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => !wrapRef.current?.contains(e.target as Node) && setOpen(false);
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  return (
    <div className={cx("fmenu", className)} ref={wrapRef}>
      <button type="button" ref={btnRef} className="g-chip fchip" aria-label={iconOnly ? label : undefined} title={iconOnly ? label : undefined} aria-expanded={open} aria-haspopup="dialog" onClick={() => setOpen(!open)}>
        {icon}
        {!iconOnly && <span className="k">{label}</span>}
        {!!count && <span className="g-tag solid-signal g-num">{count}</span>}
      </button>
      {open && (
        <div className="fpop" ref={popRef} role="dialog" aria-label={label}>
          {children}
        </div>
      )}
    </div>
  );
}

/** One option of a popover: a checkbox for sets, a radio for single choices. */
export function FilterOption({ kind, checked, onChange, children, hint }: { kind: "checkbox" | "radio"; checked: boolean; onChange: () => void; children: ReactNode; hint?: ReactNode }) {
  return (
    <label className="fopt">
      <input type={kind} checked={checked} onChange={onChange} />
      <span className="grow">{children}</span>
      {hint !== undefined && hint !== "" && <span className="hint">{hint}</span>}
    </label>
  );
}
