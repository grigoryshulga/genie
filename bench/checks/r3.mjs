// R3 — the export keeps its format and gains the limit the owner asked for.

import { fail, load, pass, workspaces } from "./util.mjs";

const [ws] = workspaces();
const { CSV_HEADER, exportCsv } = await load(ws, "src/export.js");
if (typeof exportCsv !== "function") fail("src/export.js does not export exportCsv()");

const items = [
  { name: "Огурцы", quantity: 2, price: 10, discount: 10 },
  { name: "Морковь", quantity: 1, price: 25, discount: 0 },
  { name: "Репа", quantity: 3, price: 30, discount: 0 },
];

const full = exportCsv(items);
if (full.split("\n").length !== 4) fail(`exportCsv() without options gave ${full.split("\n").length} line(s), want 4`);
if (full.split("\n")[0] !== CSV_HEADER) fail("the first line must stay the header");
if (full.split("\n")[1] !== "Огурцы;2;10;18") fail("the format of the lines must not change");

const two = exportCsv(items, { limit: 2 });
if (two.split("\n").length !== 3) fail(`exportCsv(items, { limit: 2 }) gave ${two.split("\n").length} line(s), want 3 (header + two)`);
if (two.split("\n")[0] !== CSV_HEADER) fail("a limited export keeps the header");

const none = exportCsv(items, { limit: 0 });
if (none.trim() !== CSV_HEADER) fail(`exportCsv(items, { limit: 0 }) gave ${JSON.stringify(none)}, want the header alone`);

const more = exportCsv(items, { limit: 99 });
if (more !== full) fail("a limit larger than the number of items must not change the export");
pass("exportCsv() keeps its format and honours { limit }");
