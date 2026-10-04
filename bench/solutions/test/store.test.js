import assert from "node:assert/strict";
import { test } from "node:test";

import { createStore } from "../src/store.js";

test("a store keeps the items in order", () => {
  const store = createStore([{ name: "Огурцы", quantity: 2, price: 10 }]);
  store.add({ name: "Морковь", price: 25 });
  assert.deepEqual(store.all().map((item) => item.name), ["Огурцы", "Морковь"]);
  assert.equal(store.count(), 2);
});

test("the defaults are filled in", () => {
  const store = createStore();
  const item = store.add({ name: "Репа", price: 30, tags: ["овощи"] });
  assert.deepEqual(item, { name: "Репа", quantity: 1, price: 30, discount: 0, tags: ["овощи"] });
});

test("the store filters by tag", () => {
  const store = createStore();
  store.add({ name: "Яблоко", price: 10, tags: ["фрукты"] });
  assert.deepEqual(store.byTag("фрукты").map((item) => item.name), ["Яблоко"]);
});
