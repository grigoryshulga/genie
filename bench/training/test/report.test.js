import assert from "node:assert/strict";
import { test } from "node:test";

import { subtotal, total } from "../src/report.js";

test("a subtotal takes the discount off", () => {
  assert.equal(subtotal({ price: 100, quantity: 2, discount: 10 }), 180);
  assert.equal(subtotal({ price: 9.99, quantity: 3, discount: 0 }), 29.97);
  assert.equal(subtotal({ price: 10, quantity: 0, discount: 0 }), 0);
});

test("the total adds the items up", () => {
  assert.equal(total([{ price: 10, quantity: 2, discount: 0 }, { price: 5, quantity: 1, discount: 0 }]), 25);
  assert.equal(total([]), 0);
});
