// Keeping the items and answering questions about them.

import { log } from "./log.js";

/** A store over the items it is given; it keeps them in order. */
export function createStore(items = []) {
  const all = items.map(normalise);
  return {
    /** Every item, in the order it was added. */
    all() {
      return all.slice();
    },
    /** Add one item and answer it as it was stored; `tags` are optional. */
    add(item) {
      const stored = normalise(item);
      all.push(stored);
      log("info", `added ${stored.name}`);
      return stored;
    },
    /** How many items the store holds. */
    count() {
      return all.length;
    },
  };
}

function normalise(item) {
  const name = String(item.name ?? "").trim();
  if (name === "") throw new Error("the name is empty");
  return {
    name,
    quantity: Number(item.quantity ?? 1),
    price: Number(item.price ?? 0),
    discount: Number(item.discount ?? 0),
    tags: Array.isArray(item.tags) ? item.tags.map(String) : [],
  };
}
