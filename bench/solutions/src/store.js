import { log } from "./log.js";
export function createStore(items = []) {
  const all = items.map(normalise);
  return {
    all() { return all.slice(); },
    add(item) {
      const stored = normalise(item);
      all.push(stored);
      log("info", `added ${stored.name}`);
      return stored;
    },
    count() { return all.length; },
    byTag(tag) { return all.filter((item) => item.tags.includes(tag)); },
    remove(name) {
      const at = all.findIndex((item) => item.name === name);
      if (at === -1) return false;
      all.splice(at, 1);
      return true;
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
