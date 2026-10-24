import assert from "node:assert/strict";
import { test } from "node:test";
import { STATUS_NAME } from "../web/src/entities/task/model.ts";
import { priorityBars, STATUS_STYLE } from "../web/src/entities/task/status-style.ts";

test("every status has a shape and a colour role", () => {
  for (const status of Object.keys(STATUS_NAME)) {
    const style = STATUS_STYLE[status as keyof typeof STATUS_STYLE];
    assert.ok(style, `missing style for ${status}`);
  }
});

test("only a status that needs the owner uses the signal role", () => {
  const signal = Object.entries(STATUS_STYLE).filter(([, s]) => s.role === "signal").map(([k]) => k);
  assert.deepEqual(signal, ["needs_owner"]);
});

test("statuses sharing a role are told apart by shape, except the two check discs", () => {
  const seen = new Map<string, string>();
  for (const [status, style] of Object.entries(STATUS_STYLE)) {
    const key = `${style.role}:${style.shape}`;
    assert.equal(seen.has(key), false, `${status} duplicates ${seen.get(key)}`);
    seen.set(key, status);
  }
});

test("priority maps to ascending bars, an urgent mark, or dashes", () => {
  assert.deepEqual([0, 1, 2, 3, 4].map(priorityBars), ["urgent", 3, 2, 1, "none"]);
});
