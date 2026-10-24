// Keyboard navigation for menus, tabs and segmented controls (WAI-ARIA roving focus): which item an
// arrow, Home or End key moves to. Pure so it can be tested without a DOM.

export type NavKey = "ArrowDown" | "ArrowUp" | "ArrowRight" | "ArrowLeft" | "Home" | "End";

export function nextIndex(current: number, key: string, count: number, orientation: "vertical" | "horizontal" = "vertical"): number {
  if (count <= 0) return -1;
  const forward = orientation === "vertical" ? "ArrowDown" : "ArrowRight";
  const backward = orientation === "vertical" ? "ArrowUp" : "ArrowLeft";
  if (key === "Home") return 0;
  if (key === "End") return count - 1;
  if (key === forward) return current < 0 ? 0 : (current + 1) % count;
  if (key === backward) return current < 0 ? count - 1 : (current - 1 + count) % count;
  return current;
}
