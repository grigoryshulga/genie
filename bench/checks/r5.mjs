// R5 — a two-field line is name and price with quantity 1, and nothing else moves.

import { fail, load, pass, testsPass, workspaces } from "./util.mjs";

const [ws] = workspaces();
const { parseLine } = await load(ws, "src/parse.js");
if (typeof parseLine !== "function") fail("src/parse.js does not export parseLine()");

let omitted = null;
try {
  omitted = parseLine("Огурцы;10");
} catch (e) {
  fail(`parseLine("Огурцы;10") threw ${e.message}: a two-field line is the name and the price`);
}
if (omitted?.name !== "Огурцы" || omitted?.quantity !== 1 || omitted?.price !== 10 || omitted?.discount !== 0) {
  fail(`parseLine("Огурцы;10") gave ${JSON.stringify(omitted)}, want {name:"Огурцы",quantity:1,price:10,discount:0}`);
}

const zero = parseLine("Огурцы;0;10");
if (zero?.quantity !== 0) fail(`parseLine("Огурцы;0;10").quantity is ${zero?.quantity}: a zero quantity is a quantity`);

const four = parseLine("Огурцы;3;10;15");
if (four?.quantity !== 3 || four?.price !== 10 || four?.discount !== 15) fail("a four-field line must still parse");
if (parseLine("# comment") !== null) fail("a comment must still carry nothing");
try {
  parseLine("Огурцы");
  fail("a one-field line must still be refused");
} catch (e) {
  if (!/too few fields/.test(String(e.message))) fail(`a one-field line gave "${e.message}", want "too few fields"`);
}

testsPass(ws);
pass("a two-field line is name and price, and the suite is green");
