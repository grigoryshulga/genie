import assert from "node:assert/strict";
import { test } from "node:test";
import { characterState } from "../web/src/shared/ui/character/character-state.ts";

test("with no signal the character sleeps", () => {
  assert.equal(characterState({ working: 0, waiting: 0 }), "sleep");
});

test("working agents make it think, and a resume makes it wake first", () => {
  assert.equal(characterState({ working: 2, waiting: 0 }), "think");
  assert.equal(characterState({ working: 1, waiting: 0, resumed: true }), "wake");
});

test("a decision for the owner takes priority over everything", () => {
  assert.equal(characterState({ working: 3, waiting: 1 }), "surprise");
  assert.equal(characterState({ working: 0, waiting: 2 }), "surprise");
});
