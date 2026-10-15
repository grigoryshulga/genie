import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { ModelList } from "./ModelList.tsx";
import "./model-menu.css";

const short = (id: string) => id.replace(/^[^/]+\//, "");
const provider = (id: string) => (id.includes("/") ? id.slice(0, id.indexOf("/")) : "");
/** Where the list opens: under the field or over it, as wide as the field. */
type Pos = { top?: number; bottom?: number; left: number; width: number; maxHeight: number };
const MARGIN = 12;
const ROOMY = 420;
const TALLEST = 460;

/**
 * A model field for a form: the button shows the chosen model (or the default
 * one), the list under it picks another from the models pi offers. Picking
 * closes the list; `""` is the default.
 */
export function ModelPicker({ id, value, onChange, fallback }: { id?: string; value: string; onChange: (model: string) => void; fallback?: string }) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<Pos>();
  const btn = useRef<HTMLButtonElement>(null);

  useLayoutEffect(() => {
    if (!open || !btn.current) return;
    const place = () => {
      const r = btn.current!.getBoundingClientRect();
      const below = window.innerHeight - r.bottom - 6 - MARGIN;
      const above = r.top - 6 - MARGIN;
      const base = { left: r.left, width: r.width };
      setPos(below >= Math.min(ROOMY, above) ? { ...base, top: r.bottom + 6, maxHeight: Math.min(TALLEST, below) } : { ...base, bottom: window.innerHeight - r.top + 6, maxHeight: Math.min(TALLEST, above) });
    };
    place();
    window.addEventListener("resize", place);
    return () => window.removeEventListener("resize", place);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    // Escape closes the list, not the dialog the field is in.
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopImmediatePropagation();
      close();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open]);

  const close = () => {
    setOpen(false);
    btn.current?.focus();
  };
  const pick = (model: string) => {
    onChange(model);
    close();
  };

  return (
    <>
      <button ref={btn} id={id} type="button" className={`mp-trigger${open ? " open" : ""}`} aria-haspopup="dialog" aria-expanded={open} onClick={() => setOpen(!open)}>
        <span className={`mono${value ? "" : " def"}`}>{value ? short(value) : "по умолчанию"}</span>
        <span className="sub">{value ? provider(value) : fallback ? `${short(fallback)} — модель роли` : "у роли нет модели — назначьте её"}</span>
        <svg width="10" height="10" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
          <path d="M4 6l4 4 4-4" />
        </svg>
      </button>
      {open && pos && (
        <>
          <div className="mm-scrim" onMouseDown={close} />
          <div className="mm-pop mp-pop" role="dialog" aria-label="Модель роли" style={pos}>
            <ModelList
              current={value || undefined}
              pick={value}
              onPick={pick}
              note={() => ""}
              autoFocus
              first={
                <button type="button" role="radio" aria-checked={!value} className={`mm-opt role${!value ? " on" : ""}`} onClick={() => pick("")}>
                  <span className="mm-radio" />
                  <span className="mm-role">
                    <span>По умолчанию</span>
                    <span className="mono muted">{fallback ? `${short(fallback)} — модель роли` : "у роли нет модели — назначьте её"}</span>
                  </span>
                </button>
              }
            />
          </div>
        </>
      )}
    </>
  );
}
