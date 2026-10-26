// Inline rename for sections, articles and roles: a pencil shows on hover or focus, a click turns the name
// into a field. Enter or leaving the field saves; Escape cancels.

import "./rename.css";
import { useRef, useState, type KeyboardEvent } from "react";
import { Icon } from "../icons.tsx";

export function RenameField({ value, label, className = "", onCommit, onCancel }: {
  value: string; label: string; className?: string; onCommit: (title: string) => void; onCancel: () => void;
}) {
  const [draft, setDraft] = useState(value);
  // Enter and blur can both fire on the way out; only the first one counts.
  const finished = useRef(false);
  const finish = (save: boolean) => {
    if (finished.current) return;
    finished.current = true;
    const next = draft.trim();
    if (save && next && next !== value) onCommit(next);
    else onCancel();
  };
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      event.preventDefault();
      finish(true);
    } else if (event.key === "Escape") {
      event.preventDefault();
      finish(false);
    }
  };
  return (
    <input
      className={`g-rename-input ${className}`.trim()}
      aria-label={label}
      autoFocus
      value={draft}
      onChange={(event) => setDraft(event.target.value)}
      onKeyDown={onKeyDown}
      onBlur={() => finish(true)}
      onFocus={(event) => event.currentTarget.select()}
    />
  );
}

export function RenameButton({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      type="button"
      className="g-rename-btn"
      aria-label={label}
      title={label}
      onClick={(event) => {
        event.stopPropagation();
        onClick();
      }}
    >
      <Icon.pencil size={12} />
    </button>
  );
}
