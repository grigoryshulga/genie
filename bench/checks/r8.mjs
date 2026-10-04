// R8 — the three unrelated changes are all there, wherever each was made.

import { fail, pass, tryLoad, workspaces } from "./util.mjs";

const dirs = workspaces();
const missing = [];
let removed = false;
let exported = false;
let rounded = false;

for (const ws of dirs) {
  const storeModule = await tryLoad(ws, "src/store.js");
  if (storeModule?.createStore && !removed) {
    const store = storeModule.createStore();
    store.add({ name: "Огурцы", price: 10 });
    if (typeof store.remove === "function") {
      const answer = store.remove("Огурцы");
      if (answer !== true) fail(`store.remove("Огурцы") answered ${JSON.stringify(answer)}, want true`);
      if (store.count() !== 0) fail("store.remove() must drop the item from the store");
      if (store.remove("Огурцы") !== false) fail("store.remove() must answer false for an item that is not there");
      removed = true;
    }
  }

  const exportModule = await tryLoad(ws, "src/export.js");
  if (exportModule?.exportJson && !exported) {
    const text = exportModule.exportJson([{ name: "Огурцы", price: 10 }]);
    if (typeof text !== "string") fail("exportJson() must return a string");
    let parsed;
    try {
      parsed = JSON.parse(text);
    } catch (e) {
      fail(`exportJson() gave ${JSON.stringify(text)}, which is not JSON: ${e.message}`);
    }
    if (parsed?.[0]?.name !== "Огурцы") fail(`exportJson() gave ${text}: the items have to be in it`);
    exported = true;
  }

  const formatModule = await tryLoad(ws, "src/format.js");
  if (formatModule?.money && !rounded) {
    if (formatModule.money(1.005) !== "1.01") fail(`money(1.005) = ${formatModule.money(1.005)}, want "1.01"`);
    if (formatModule.money(2) !== "2.00") fail(`money(2) = ${formatModule.money(2)}, want "2.00"`);
    rounded = true;
  }
}

if (!removed) missing.push("store.remove(name)");
if (!exported) missing.push("export.exportJson(items)");
if (!rounded) missing.push('money() rounding 1.005 to "1.01"');
if (missing.length > 0) fail(`still missing: ${missing.join(", ")}`);
pass("all three changes are present");
