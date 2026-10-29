// The board's row is centred on its open lanes: the leading offset that puts their midpoint in the viewport's middle.

import test from "node:test";
import assert from "node:assert/strict";

import { leadingOffset, type RowLane } from "../web/src/widgets/board/model/centering.ts";

const OPEN = 344;
const FOLDED = 64;
const GAP = 12;
const PAD = 32;

const lane = (open: boolean): RowLane => ({ width: open ? OPEN : FOLDED, open });

/** Where the open lanes' midpoint lands on screen, with the lead applied and the row unscrolled. */
function midpointOnScreen(viewport: number, lanes: RowLane[], paddingStart = PAD) {
  const lead = leadingOffset({ viewport, paddingStart, paddingEnd: PAD, gap: GAP, lanes });
  let x = 0;
  let first = -1;
  let last = -1;
  lanes.forEach((l, i) => {
    if (l.open) {
      if (first < 0) first = x;
      last = x + l.width;
    }
    x += l.width + GAP;
  });
  return paddingStart + lead + (first + last) / 2;
}

test("one open lane sits exactly in the middle of the viewport, folded lanes beside it", () => {
  const lanes = [lane(false), lane(true), lane(false), lane(false), lane(false)];
  assert.equal(midpointOnScreen(1200, lanes), 600);
  assert.ok(leadingOffset({ viewport: 1200, paddingStart: PAD, paddingEnd: PAD, gap: GAP, lanes }) > 0);
});

test("two open lanes with a folded one between centre on the span, folded width included", () => {
  const lanes = [lane(true), lane(false), lane(true), lane(false), lane(false)];
  // the span runs from the first open lane's left edge to the second's right edge
  const span = OPEN + GAP + FOLDED + GAP + OPEN;
  const lead = leadingOffset({ viewport: 1200, paddingStart: PAD, paddingEnd: PAD, gap: GAP, lanes });
  assert.equal(PAD + lead + span / 2, 600);
  assert.equal(midpointOnScreen(1200, lanes), 600);
});

test("with every lane folded the whole row is centred", () => {
  const lanes = Array.from({ length: 5 }, () => lane(false));
  const content = 5 * FOLDED + 4 * GAP;
  const lead = leadingOffset({ viewport: 1200, paddingStart: PAD, paddingEnd: PAD, gap: GAP, lanes });
  assert.equal(PAD + lead + content / 2, 600);
});

test("a negative offset clamps to 0: the row keeps its normal left padding", () => {
  // all five lanes open are wider than the viewport's half: the midpoint is already past the centre
  const lanes = Array.from({ length: 5 }, () => lane(true));
  assert.equal(leadingOffset({ viewport: 800, paddingStart: PAD, paddingEnd: PAD, gap: GAP, lanes }), 0);
  // the open lane sits past the centre with the folded lanes all on its left: nothing can pull it back
  assert.equal(leadingOffset({ viewport: 500, paddingStart: PAD, paddingEnd: PAD, gap: GAP, lanes: [lane(false), lane(false), lane(false), lane(false), lane(true)] }), 0);
  // a single open lane in a viewport too narrow for the centre to reach its midpoint
  assert.equal(leadingOffset({ viewport: 360, paddingStart: PAD, paddingEnd: PAD, gap: GAP, lanes: [lane(true), lane(false), lane(false), lane(false), lane(false)] }), 0);
});

test("a row that overflows the viewport starts at its first lane", () => {
  // the row alone is wider than the viewport
  const wide = [lane(true), lane(false), lane(false), lane(false), lane(false)];
  assert.equal(leadingOffset({ viewport: 500, paddingStart: PAD, paddingEnd: PAD, gap: GAP, lanes: wide }), 0);
  // the row fits unshifted (712px), but centred its folded lanes would run past the viewport's right edge
  const pushed = [lane(true), lane(false), lane(false), lane(false), lane(false)];
  assert.equal(leadingOffset({ viewport: 900, paddingStart: PAD, paddingEnd: PAD, gap: GAP, lanes: pushed }), 0);
  // the same row is centred once the viewport holds the shifted row
  assert.ok(leadingOffset({ viewport: 1100, paddingStart: PAD, paddingEnd: PAD, gap: GAP, lanes: pushed }) > 0);
});

test("no lanes means no offset", () => {
  assert.equal(leadingOffset({ viewport: 1200, paddingStart: PAD, paddingEnd: PAD, gap: GAP, lanes: [] }), 0);
});
