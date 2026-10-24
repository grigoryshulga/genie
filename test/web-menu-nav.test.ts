import assert from "node:assert/strict";
import { test } from "node:test";
import { nextIndex } from "../web/src/shared/ui/kit/menu-nav.ts";

test("vertical menus wrap with arrow keys and jump with Home and End", () => {
  assert.equal(nextIndex(0, "ArrowDown", 3), 1);
  assert.equal(nextIndex(2, "ArrowDown", 3), 0);
  assert.equal(nextIndex(0, "ArrowUp", 3), 2);
  assert.equal(nextIndex(1, "Home", 3), 0);
  assert.equal(nextIndex(1, "End", 3), 2);
});

test("nothing focused starts at the first or last item", () => {
  assert.equal(nextIndex(-1, "ArrowDown", 3), 0);
  assert.equal(nextIndex(-1, "ArrowUp", 3), 2);
});

test("horizontal rails use left and right, and ignore other keys", () => {
  assert.equal(nextIndex(0, "ArrowRight", 4, "horizontal"), 1);
  assert.equal(nextIndex(0, "ArrowLeft", 4, "horizontal"), 3);
  assert.equal(nextIndex(0, "ArrowDown", 4, "horizontal"), 0);
  assert.equal(nextIndex(0, "a", 4), 0);
  assert.equal(nextIndex(0, "ArrowDown", 0), -1);
});
