// What the older inventories rely on: it must not change.

import assert from "node:assert/strict";
import { test } from "node:test";

import { parseLine } from "../src/parse.js";
import { subtotal } from "../src/report.js";

test("a quantity of zero is a quantity, not a missing one", () => {
  const item = parseLine("Огурцы;0;10");
  assert.equal(item.quantity, 0);
  assert.equal(subtotal(item), 0);
});

test("the discount the old files carry is a per cent, zero when missing", () => {
  assert.equal(parseLine("Огурцы;2;50;10").discount, 10);
  assert.equal(parseLine("Огурцы;2;50").discount, 0);
});

test("a name keeps the spaces inside it", () => {
  assert.equal(parseLine("  Красная капуста ;1;30").name, "Красная капуста");
});
