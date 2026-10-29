// Where the board's row of lanes starts, so that its open lanes sit in the middle of the viewport.

/** One lane of the row as laid out: its width in px and whether it is unfolded. */
export interface RowLane {
  width: number;
  open: boolean;
}

export interface RowGeometry {
  /** Visible width of the scroller, padding included. */
  viewport: number;
  paddingStart: number;
  paddingEnd: number;
  gap: number;
  lanes: readonly RowLane[];
}

/**
 * The px the first lane moves right, on top of the scroller's start padding.
 *
 * The axis is the midpoint between the left edge of the first open lane and the right edge of the last
 * open lane; folded lanes between them are part of that span. With no open lane the whole row is the span.
 * The offset is 0 when the shifted row would overflow the viewport: an overflowing row starts at its first
 * lane and scrolls as it always did.
 */
export function leadingOffset({ viewport, paddingStart, paddingEnd, gap, lanes }: RowGeometry): number {
  if (lanes.length === 0) return 0;

  // Each lane's left edge, measured from the start of the row's content (after the start padding).
  const edges: number[] = [];
  let content = 0;
  for (const lane of lanes) {
    edges.push(content);
    content += lane.width + gap;
  }
  content -= gap;

  const first = lanes.findIndex((lane) => lane.open);
  const last = lanes.findLastIndex((lane) => lane.open);
  const spanStart = first < 0 ? 0 : edges[first];
  const spanEnd = first < 0 ? content : edges[last] + lanes[last].width;
  const middle = (spanStart + spanEnd) / 2;

  const lead = viewport / 2 - paddingStart - middle;
  if (lead <= 0 || paddingStart + lead + content + paddingEnd > viewport) return 0;
  return lead;
}
