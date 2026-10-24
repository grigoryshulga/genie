import { type ReactNode, useState } from "react";
import { Checkbox } from "./kit/forms.tsx";
import { Modal } from "./Modal.tsx";

/** Confirmation for destructive actions, with an optional extra checkbox. */
export function ConfirmDialog({
  title,
  children,
  confirmLabel,
  danger,
  option,
  busy,
  onConfirm,
  onClose,
}: {
  title: string;
  children: ReactNode;
  confirmLabel: string;
  danger?: boolean;
  option?: string;
  busy?: boolean;
  onConfirm: (optionChecked: boolean) => void;
  onClose: () => void;
}) {
  const [checked, setChecked] = useState(false);
  return (
    <Modal label={title} onClose={onClose}>
      <div className="mh">{title}</div>
      <div className="mb">
        <div className="cf-text">{children}</div>
        {option && <Checkbox label={option} checked={checked} onChange={(e) => setChecked(e.target.checked)} />}
      </div>
      <div className="mf">
        <span className="grow" />
        <button type="button" className="g-btn ghost" onClick={onClose}>
          Отмена
        </button>
        <button type="button" className={`g-btn ${danger ? "danger" : "primary"}`} disabled={busy} onClick={() => onConfirm(checked)} autoFocus>
          {confirmLabel}
        </button>
      </div>
    </Modal>
  );
}
