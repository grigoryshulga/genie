import assert from "node:assert/strict";
import { test } from "node:test";

import { parse, parseLine } from "../src/parse.js";

test("a line is name;quantity;price with an optional discount", () => {
  assert.deepEqual(parseLine("Огурцы;3;10"), { name: "Огурцы", quantity: 3, price: 10, discount: 0 });
  assert.deepEqual(parseLine("Огурцы;3;10;15"), { name: "Огурцы", quantity: 3, price: 10, discount: 15 });
  assert.deepEqual(parseLine("  Морковь ; 2 ; 25.5 "), { name: "Морковь", quantity: 2, price: 25.5, discount: 0 });
});

test("the text keeps the comments out and the order in", () => {
  assert.deepEqual(parse("# овощи\nОгурцы;1;10\n\nМорковь;2;25\n").map((item) => item.name), ["Огурцы", "Морковь"]);
  assert.deepEqual(parseLine("# овощи"), null, "a comment carries nothing");
  assert.deepEqual(parseLine("   "), null, "a blank line carries nothing");
});

test("a broken line is refused", () => {
  assert.throws(() => parseLine("Огурцы"), /too few fields/);
  assert.throws(() => parseLine("Огурцы;3;10;5;1"), /too many fields/);
  assert.throws(() => parseLine(";3;10"), /name is empty/);
  assert.throws(() => parseLine("Огурцы;;10"), /quantity is empty/);
  assert.throws(() => parseLine("Огурцы;-1;10"), /not a number/);
});
