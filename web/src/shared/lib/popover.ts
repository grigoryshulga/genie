// Placement of a popover that is `position: fixed` under its trigger.

/** The trigger's place in the window: its left edge and its bottom edge. */
export type TriggerRect = { left: number; bottom: number };

/** The window the popover must stay inside. */
export type Viewport = { width: number; height: number };

/** A fixed popover's window coordinates and the height it may not exceed. */
export type PopoverPlacement = { left: number; top: number; maxHeight: number };

const EDGE = 8; // keep this much room to the window's edges
const GAP = 6; // the space between the trigger and the popover
const MIN_HEIGHT = 160; // a popover cut below this would be unusable

/**
 * Where a popover goes: under the trigger, shifted left when it would run past
 * the window's right edge, and capped so it ends above the bottom edge. The
 * toolbar the trigger sits in may scroll sideways; the popover is
 * `position: fixed`, so it leaves that scroll box and is placed from here.
 */
export function anchorPopover(trigger: TriggerRect, popoverWidth: number, viewport: Viewport): PopoverPlacement {
  const left = Math.max(EDGE, Math.min(trigger.left, viewport.width - popoverWidth - EDGE));
  const top = trigger.bottom + GAP;
  return { left, top, maxHeight: Math.max(MIN_HEIGHT, viewport.height - top - EDGE) };
}
