// R7 — render() gives a human-readable report and the suite stays green.

import { fail, load, pass, testsPass, workspaces } from "./util.mjs";

const [ws] = workspaces();
const { render } = await load(ws, "src/report.js");
if (typeof render !== "function") fail("src/report.js must export render(items)");

const items = [
  { name: "Огурцы", quantity: 2, price: 10, discount: 0 },
  { name: "Морковь", quantity: 1, price: 25, discount: 0 },
];
const text = render(items);
if (typeof text !== "string" || text.trim() === "") fail("render(items) must return a document");
if (!text.includes("Огурцы") || !text.includes("Морковь")) fail(`render() = ${JSON.stringify(text)}: every item has to be named`);
if (!text.includes("45.00")) fail(`render() = ${JSON.stringify(text)}: the total 45.00 has to be there`);
if (!/Итого/.test(text)) fail(`render() = ${JSON.stringify(text)}: the closing line has to say Итого`);

const empty = render([]);
if (typeof empty !== "string" || !empty.includes("0.00")) fail(`render([]) = ${JSON.stringify(empty)}, want a document with the zero total`);

testsPass(ws);
pass("render() names the items, closes with the total and the suite is green");
