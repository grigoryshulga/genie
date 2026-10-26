import { useCallback, useEffect, useRef } from "react";

/** Motion belongs to the board's task overlay; all other Dialog consumers keep their own behaviour. */
export function useTaskModalMotion(id: string | undefined, board: boolean, onClose: () => void) {
  const origin = useRef<HTMLElement | null>(null);
  const animations = useRef<Animation[]>([]);
  const closing = useRef(false);

  useEffect(() => {
    closing.current = false;
    if (!id || !board) return;
    // Dialog's showModal effect has run before this parent effect. Data attributes also find the
    // card when a task URL was opened directly; no click coordinates or router state are required.
    origin.current = document.querySelector<HTMLElement>(`[data-task-id="${CSS.escape(id)}"]`);
    const dialog = document.querySelector<HTMLDialogElement>(".task-modal");
    const card = origin.current;
    if (dialog?.open && card && !reducedMotion()) {
      const from = originTransform(dialog, card);
      if (from) {
        const style = getComputedStyle(dialog);
        animations.current = [dialog.animate(
          [{ transform: from, opacity: 0.5 }, { transform: "none", opacity: 1 }],
          { duration: duration(style) * 4 / 3, easing: style.getPropertyValue("--g-ease").trim() },
        )];
        const detail = dialog.querySelector(".detail");
        // Keep the text from stretching while the sheet grows from the source card.
        if (detail) animations.current.push(detail.animate(
          [{ opacity: 0 }, { opacity: 1 }],
          { duration: duration(style), delay: duration(style) / 3, fill: "backwards" },
        ));
      }
    }
    return () => {
      for (const animation of animations.current) animation.cancel();
      animations.current = [];
      origin.current = null;
      closing.current = false;
    };
  }, [id, board]);

  return useCallback(async () => {
    if (closing.current) return;
    const dialog = document.querySelector<HTMLDialogElement>(".task-modal");
    const card = origin.current;
    if (!board || !dialog || !card?.isConnected || reducedMotion()) {
      onClose();
      return;
    }
    // A fast Escape can arrive during entry; measure the resting dialog after cancelling it.
    for (const animation of animations.current) animation.cancel();
    const to = originTransform(dialog, card);
    if (!to) {
      onClose();
      return;
    }
    closing.current = true;
    const style = getComputedStyle(dialog);
    const animation = dialog.animate(
      [{ transform: "none", opacity: 1 }, { transform: to, opacity: 0 }],
      { duration: duration(style), easing: style.getPropertyValue("--g-ease").trim(), fill: "forwards" },
    );
    animations.current = [animation];
    const detail = dialog.querySelector(".detail");
    if (detail) animations.current.push(detail.animate(
      [{ opacity: 1 }, { opacity: 0 }],
      { duration: duration(style) / 2, fill: "forwards" },
    ));
    try {
      await animation.finished;
      onClose();
    } catch {
      // A route change or unmount cancelled the animation and already owns navigation.
    }
  }, [board, onClose]);
}

const reducedMotion = () => matchMedia("(prefers-reduced-motion: reduce)").matches;
const duration = (style: CSSStyleDeclaration) => {
  // The production CSS optimizer may serialize 150ms as .15s; Web Animations always takes ms.
  const value = style.getPropertyValue("--g-duration").trim();
  return parseFloat(value) * (value.endsWith("ms") ? 1 : 1000);
};

function originTransform(dialog: HTMLElement, card: HTMLElement): string | undefined {
  const from = card.getBoundingClientRect();
  const to = dialog.getBoundingClientRect();
  // Folded, tucked or filtered-away cards should never pull the overlay off screen.
  if (!from.width || !from.height || !to.width || !to.height || getComputedStyle(card).pointerEvents === "none" || from.bottom < 0 || from.top > innerHeight || from.right < 0 || from.left > innerWidth) return;
  return `translate(${from.x + from.width / 2 - to.x - to.width / 2}px, ${from.y + from.height / 2 - to.y - to.height / 2}px) scale(${from.width / to.width}, ${from.height / to.height})`;
}
