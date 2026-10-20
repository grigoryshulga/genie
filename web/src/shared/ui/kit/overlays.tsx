import { type ReactNode, useEffect, useId, useRef } from "react";
import { cx } from "./cx.ts";
import { nextIndex } from "./menu-nav.ts";

/**
 * A dialog on the native <dialog> element: showModal() makes the page behind it inert, traps focus,
 * closes on Escape, and returns focus to the opener. Closing from any source calls onClose.
 */
export function Dialog({ title, onClose, footer, children, wide, sheet }: { title: ReactNode; onClose: () => void; footer?: ReactNode; children: ReactNode; wide?: boolean; /** A bottom sheet on phones. */ sheet?: boolean }) {
  const ref = useRef<HTMLDialogElement>(null);
  const heading = useId();
  useEffect(() => {
    const dialog = ref.current;
    if (dialog && !dialog.open) dialog.showModal();
    return () => dialog?.close();
  }, []);
  return (
    <dialog ref={ref} className={cx("g-dialog", sheet && "sheet")} aria-labelledby={heading} onClose={onClose} onClick={(e) => e.target === ref.current && onClose()}>
      <div className={cx("g-modal", wide && "wide")}>
        <header>
          <h2 id={heading}>{title}</h2>
          <button type="button" className="g-icon-btn" aria-label="Закрыть" onClick={onClose}>
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" aria-hidden="true">
              <path d="M4 4l8 8M12 4l-8 8" />
            </svg>
          </button>
        </header>
        <div className="g-modal-body">{children}</div>
        {footer && <footer>{footer}</footer>}
      </div>
    </dialog>
  );
}

export type MenuEntry = { id: string; label: ReactNode; hint?: ReactNode; checked?: boolean; disabled?: boolean; onSelect: () => void } | "separator";

/** Arrow keys move through the items, Enter or Space selects, Escape closes. The caller owns open/close and focus return. */
export function ActionMenu({ label, items, onClose }: { label: string; items: MenuEntry[]; onClose: () => void }) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const actionable = items.flatMap((item, i) => (item !== "separator" && !item.disabled ? [i] : []));
  useEffect(() => void refs.current[actionable[0]]?.focus(), []); // eslint-disable-line react-hooks/exhaustive-deps
  function onKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Escape") return onClose();
    const at = actionable.indexOf(refs.current.findIndex((el) => el === document.activeElement));
    const next = nextIndex(at, e.key, actionable.length);
    if (next !== at) {
      e.preventDefault();
      refs.current[actionable[next]]?.focus();
    }
  }
  return (
    <div className="g-menu" role="menu" aria-label={label} onKeyDown={onKeyDown}>
      {items.map((item, i) =>
        item === "separator" ? (
          <div key={i} className="g-menu-sep" role="separator" />
        ) : (
          <button
            key={item.id}
            type="button"
            role={item.checked === undefined ? "menuitem" : "menuitemcheckbox"}
            aria-checked={item.checked}
            disabled={item.disabled}
            className="g-menu-item"
            ref={(el) => void (refs.current[i] = el)}
            onClick={() => {
              item.onSelect();
              onClose();
            }}
          >
            {item.label}
            {item.hint && <span className="hint">{item.hint}</span>}
          </button>
        ),
      )}
    </div>
  );
}
