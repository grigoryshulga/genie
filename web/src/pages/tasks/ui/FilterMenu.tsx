import { type ReactNode, useEffect, useRef, useState } from "react";
import { Icon } from "@/shared/ui";

/** A filter button of the task list with its options in a popover underneath. */
export function FilterMenu({ label, value, active, children }: { label: string; value: string; active: boolean; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);
  return (
    <div className="fmenu" ref={ref}>
      <button type="button" className={`fbtn${active ? " on" : ""}${open ? " open" : ""}`} aria-expanded={open} onClick={() => setOpen(!open)}>
        <span className="k">{label}</span>
        {value}
        <Icon.chevron size={10} style={{ transform: "rotate(90deg)" }} />
      </button>
      {open && (
        <div className="fpop" role="dialog" aria-label={label}>
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
      {hint !== undefined && <span className="hint">{hint}</span>}
    </label>
  );
}
