// Focus behaviour for an overlay drawn outside a native <dialog>: the first control takes focus when it
// opens, Tab stays inside it while it is open, Escape asks to close, and closing puts focus back on
// whatever opened it. A dialog that claims aria-modal without this traps keyboard users behind it.

import { type RefObject, useEffect, useRef } from "react";

const FOCUSABLE = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  '[tabindex]:not([tabindex="-1"])',
].join(",");

/** Controls a keyboard user can reach inside `root`, in tab order, skipping hidden ones. */
export function focusStops(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => el.getClientRects().length > 0);
}

export function useModalFocus(ref: RefObject<HTMLElement | null>, open: boolean, onClose: () => void) {
  // The callback changes on every render; keeping it in a ref means the listeners below are not torn
  // down and re-added (which would also re-run the focus move) while the overlay stays open.
  const close = useRef(onClose);
  useEffect(() => {
    close.current = onClose;
  }, [onClose]);

  useEffect(() => {
    if (!open) return;
    const root = ref.current;
    const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    // A native <dialog> has already moved focus inside by the time this effect runs; only an overlay it
    // does not know about (a sheet drawn in the page) has to be entered by hand.
    const opener = root && active && root.contains(active) ? null : active;
    if (root && !(active && root.contains(active))) {
      const first = focusStops(root)[0];
      if (first) first.focus();
      else {
        root.tabIndex = -1;
        root.focus();
      }
    }

    const onKeyDown = (e: KeyboardEvent) => {
      // Every open overlay listens on the document, so a dialog opened inside this one would otherwise
      // see its Escape answered twice: once by itself and once by the dialog it covers. The innermost
      // open dialog around the focused control is the one that speaks for the keyboard.
      const focused = document.activeElement;
      const host = focused instanceof HTMLElement ? focused.closest("dialog[open]") : null;
      if (host && host !== root) return;
      if (e.key === "Escape") {
        // A native select picker consumes its own first Escape. Capturing it here would close the
        // task underneath instead of cancelling the pending option; a closed select still delegates
        // Escape to the dialog as usual. Customizable pickers focus an option rather than the host.
        if (CSS.supports("selector(select:open)") && focused instanceof HTMLElement && focused.closest("select:open")) return;
        // Capture phase: the page behind must not answer the same Escape. The default is cancelled too:
        // the browser would otherwise close the topmost dialog itself and, with two of them up, carry the
        // same Escape on to the one underneath. Closing goes through the owner of the state, once.
        e.stopPropagation();
        e.preventDefault();
        close.current();
        return;
      }
      if (e.key !== "Tab" || !root) return;
      const inside = focusStops(root);
      // Nothing to move to yet: a dialog whose content is still loading. Tab must not leak to the page
      // behind it, so focus stays on the dialog until its first control appears.
      if (inside.length === 0) {
        e.preventDefault();
        root.tabIndex = -1;
        root.focus();
        return;
      }
      const first = inside[0];
      const last = inside[inside.length - 1];
      const at = document.activeElement;
      if (!root.contains(at)) {
        e.preventDefault();
        (e.shiftKey ? last : first).focus();
      } else if (!e.shiftKey && at === last) {
        e.preventDefault();
        first.focus();
      } else if (e.shiftKey && at === first) {
        e.preventDefault();
        last.focus();
      }
    };

    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("keydown", onKeyDown, true);
      opener?.focus();
    };
  }, [open, ref]);
}
