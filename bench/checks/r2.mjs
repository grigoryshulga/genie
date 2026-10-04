// R2 — the store filters by tag, and the new behaviour is covered by a test.

import { readFileSync } from "node:fs";
import path from "node:path";

import { fail, jsFiles, load, pass, workspaces } from "./util.mjs";

const [ws] = workspaces();
const { createStore } = await load(ws, "src/store.js");
if (typeof createStore !== "function") fail("src/store.js does not export createStore()");

const store = createStore();
store.add({ name: "Яблоко", price: 10, tags: ["фрукты", "сладости"] });
store.add({ name: "Огурец", price: 5, tags: ["овощи"] });
store.add({ name: "Банан", price: 7, tags: ["фрукты"] });

if (typeof store.byTag !== "function") fail("the store has no byTag(tag)");
const fruits = store.byTag("фрукты").map((item) => item.name);
if (fruits.join(",") !== "Яблоко,Банан") fail(`byTag("фрукты") gave ${JSON.stringify(fruits)}, want [Яблоко, Банан]`);
if (store.byTag("нет такого тега").length !== 0) fail("byTag() must be empty for a tag nothing carries");
if (store.all().length !== 3) fail("byTag() must not change the store");
store.add({ name: "Груша", price: 8, tags: ["фрукты"] });
if (store.byTag("фрукты").length !== 3) fail("byTag() must see the items added later");

const covered = jsFiles(path.join(ws, "test")).some((file) => /byTag/.test(readFileSync(file, "utf8")));
if (!covered) fail("no test in test/ mentions byTag(): the new behaviour has to be covered");
pass("byTag(tag) filters by tag and a test covers it");
