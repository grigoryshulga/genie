import type { ReactNode } from "react";
import { Dialog } from "./kit/index.ts";

/**
 * Thin wrapper over the kit Dialog (native <dialog>: focus trap, Escape, focus restore). The children draw their own
 * chrome with the .mh/.mb/.mf rows, styled in overlays.css; `label` is the accessible name.
 */
export function Modal({ label, onClose, wide, className, children }: { label: string; onClose: () => void; wide?: boolean; className?: string; children: ReactNode }) {
  return (
    <Dialog bare title={label} onClose={onClose} wide={wide} className={className}>
      {children}
    </Dialog>
  );
}
