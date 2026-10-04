import assert from "node:assert/strict";
import { test } from "node:test";

import { line, money } from "../src/format.js";

test("money keeps two decimals", () => {
  assert.equal(money(2), "2.00");
  assert.equal(money(12.3), "12.30");
});

test("a line names the item, its count and its worth", () => {
  assert.equal(line({ name: "Огурцы", quantity: 3, price: 10, discount: 0 }), "Огурцы × 3 — 30.00");
});
