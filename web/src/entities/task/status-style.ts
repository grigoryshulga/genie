// How a status and a priority look: the shape is the primary signal, the colour role only reinforces it.
// Roles follow the design system: neutral is quiet, accent (green) is system state, signal (orange) is
// "your move". Mirrors the icon set in design-system/brand/components.html.

import type { Status } from "./model.ts";

export type StatusRole = "neutral" | "accent" | "signal";
export type StatusShape =
  | "tray"
  | "ring-dashed"
  | "ring-dashed-dot"
  | "ring"
  | "ring-half"
  | "return-arrow"
  | "ring-dot"
  | "disc-check"
  | "disc-alert"
  | "ring-slash";

export const STATUS_STYLE: Record<Status, { shape: StatusShape; role: StatusRole }> = {
  inbox: { shape: "tray", role: "neutral" },
  draft: { shape: "ring-dashed", role: "neutral" },
  refining: { shape: "ring-dashed-dot", role: "accent" },
  ready: { shape: "ring", role: "accent" },
  in_progress: { shape: "ring-half", role: "accent" },
  changes_requested: { shape: "return-arrow", role: "accent" },
  review: { shape: "ring-dot", role: "accent" },
  approved: { shape: "disc-check", role: "accent" },
  needs_owner: { shape: "disc-alert", role: "signal" },
  done: { shape: "disc-check", role: "neutral" },
  cancelled: { shape: "ring-slash", role: "neutral" },
};

/** Number of filled bars (of three) for a priority, or "urgent" for the orange square, or "none" for dashes. */
export function priorityBars(priority: number): 3 | 2 | 1 | "urgent" | "none" {
  if (priority === 0) return "urgent";
  if (priority === 1) return 3;
  if (priority === 2) return 2;
  if (priority === 3) return 1;
  return "none";
}
