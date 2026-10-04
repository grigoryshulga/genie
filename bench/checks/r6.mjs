// R6 — the package stays ESM and the notification speaks the project's field names.

import { readFileSync } from "node:fs";
import path from "node:path";

import { fail, jsFiles, load, pass, read, workspaces } from "./util.mjs";

const [ws] = workspaces();

let manifest = {};
try {
  manifest = JSON.parse(readFileSync(path.join(ws, "package.json"), "utf8"));
} catch (e) {
  fail(`package.json cannot be read: ${e.message}`);
}
if (manifest.type !== "module") fail('the package must stay ESM: package.json needs "type": "module"');

for (const file of jsFiles(path.join(ws, "src"))) {
  const rel = path.relative(ws, file);
  const text = read(ws, rel);
  if (/\bmodule\.exports\b/.test(text)) fail(`${rel} uses module.exports; the package is ESM`);
  if (/\brequire\s*\(/.test(text)) fail(`${rel} uses require(); the package is ESM`);
  if (/\bmsg\s*:/.test(text)) fail(`${rel} sets a "msg" field; the project's notifications carry "text"`);
}

const { notify } = await load(ws, "src/notify.js");
if (typeof notify !== "function") fail("src/notify.js must export notify(item)");

const note = notify({ name: "Огурцы", quantity: 0 });
if (typeof note?.title !== "string" || note.title === "") fail(`notify() gave ${JSON.stringify(note)}: it needs a non-empty "title"`);
if (typeof note?.text !== "string" || note.text === "") fail(`notify() gave ${JSON.stringify(note)}: it needs a non-empty "text"`);
if (note?.level !== "info") fail(`notify() defaults to level ${JSON.stringify(note?.level)}, want "info"`);
if ("msg" in note) fail('notify() must not carry a "msg" field');
const warn = notify({ name: "Огурцы", quantity: 0 }, "warn");
if (warn?.level !== "warn") fail("the second argument of notify() is the level");
pass("the package stays ESM and notify() carries title/text/level");
