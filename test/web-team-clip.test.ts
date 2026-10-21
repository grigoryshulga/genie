import assert from "node:assert/strict";
import { test } from "node:test";
import { clipText } from "../web/src/entities/team/clip.ts";

test("a short output is not clipped", () => {
  assert.deepEqual(clipText("one\ntwo"), { head: "one\ntwo", hidden: 0 });
});

test("a long output keeps the first lines and counts the rest", () => {
  const text = Array.from({ length: 30 }, (_, i) => `line ${i}`).join("\n");
  const clip = clipText(text, 10);
  assert.equal(clip.head.split("\n").length, 10);
  assert.equal(clip.hidden, 20);
});

test("one very long line is cut by characters", () => {
  const clip = clipText("x".repeat(5000), 12, 100);
  assert.equal(clip.head.length, 100);
  assert.ok(clip.hidden >= 1);
});
