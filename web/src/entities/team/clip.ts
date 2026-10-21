/** A long tool output is shown clipped to a few lines behind a «Show more» control. */
export const CLIP_LINES = 12;
export const CLIP_CHARS = 1200;

export interface Clipped {
  head: string;
  /** How many lines are hidden; 0 when the text fits. */
  hidden: number;
}

/** The first lines of `text` that fit the limits, and how many lines were cut. */
export function clipText(text: string, maxLines = CLIP_LINES, maxChars = CLIP_CHARS): Clipped {
  const lines = text.split("\n");
  let head = lines.slice(0, maxLines).join("\n");
  if (head.length > maxChars) head = head.slice(0, maxChars);
  if (head.length === text.length) return { head: text, hidden: 0 };
  const shown = head.split("\n").length;
  return { head, hidden: Math.max(1, lines.length - shown) };
}
