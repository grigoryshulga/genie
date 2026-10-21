import assert from "node:assert/strict";
import test from "node:test";
import { projectMonogram } from "../web/src/entities/project/model.ts";

test("project monograms ignore punctuation and support Cyrillic names", () => {
  assert.equal(projectMonogram("Genie — redesign"), "GR");
  assert.equal(projectMonogram("Мой проект"), "МП");
  assert.equal(projectMonogram("Genie"), "GE");
  assert.equal(projectMonogram("  —  "), "?");
  assert.equal(projectMonogram("Я"), "Я");
});
