import { type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";
import { anchorPopover } from "@/shared/lib";
import { Icon } from "@/shared/ui";

/** A filter chip of the task list with its options in a popover underneath (a bottom sheet on a phone). */
export function FilterMenu({ label, value, active, children }: { label: string; value: string; active: boolean; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const btnRef = useRef<HTMLButtonElement>(null);
  const popRef = useRef<HTMLDivElement>(null);

  // The filter toolbar scrolls sideways (`.g-toolbar` is overflow-x: auto), which
  // would clip an absolutely positioned popover. The popover is `position: fixed`
  // instead, so it leaves that scroll box; its place under the chip is set here
  // and kept up to date while the window or the toolbar scrolls. On narrow
  // screens the stylesheet turns it into a bottom sheet and drops these values.
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

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => !wrapRef.current?.contains(e.target as Node) && setOpen(false);
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);
  return (
    <div className="fmenu" ref={wrapRef}>
      <button
        type="button"
        ref={btnRef}
        className="g-chip fchip"
        aria-expanded={open}
        aria-pressed={active}
        aria-haspopup="dialog"
        onClick={() => setOpen(!open)}
      >
        <span className="k">{label}</span>
        <span className="v">{value}</span>
        <Icon.chevron size={10} className="chev" />
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
