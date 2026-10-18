// Where a fixed popover is placed relative to its trigger and the window.

import assert from "node:assert/strict";
import { test } from "node:test";
import { anchorPopover } from "../web/src/shared/lib/popover.ts";

const viewport = { width: 1200, height: 800 };

test("a popover opens just below its trigger", () => {
  assert.deepEqual(anchorPopover({ left: 300, bottom: 64 }, 240, viewport), { left: 300, top: 70, maxHeight: 722 });
});

test("a popover near the right edge shifts left to stay inside the window", () => {
  assert.equal(anchorPopover({ left: 1100, bottom: 64 }, 240, viewport).left, 1200 - 240 - 8);
  assert.equal(anchorPopover({ left: 1100, bottom: 64 }, 340, viewport).left, 1200 - 340 - 8);
});

test("a popover never leaves the left edge of the window", () => {
  assert.equal(anchorPopover({ left: -20, bottom: 64 }, 240, viewport).left, 8);
});

test("a low trigger still leaves the popover a usable height", () => {
  assert.equal(anchorPopover({ left: 40, bottom: 300 }, 340, viewport).maxHeight, 800 - 306 - 8);
  assert.equal(anchorPopover({ left: 40, bottom: 700 }, 240, viewport).maxHeight, 160, "the floor wins near the bottom edge");
});
