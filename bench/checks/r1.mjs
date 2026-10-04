// R1 — the total must take each item's discount off.

import { fail, load, pass, workspaces } from "./util.mjs";

const [ws] = workspaces();
const { total } = await load(ws, "src/report.js");
if (typeof total !== "function") fail("src/report.js does not export total()");

const cases = [
  [[], 0, "nothing is nothing"],
  [[{ price: 100, quantity: 2, discount: 0 }], 200, "no discount"],
  [[{ price: 100, quantity: 2, discount: 10 }], 180, "ten per cent off"],
  [[{ price: 9.99, quantity: 3, discount: 5 }], 28.47, "a discount off a fractional price"],
  [
    [
      { price: 100, quantity: 1, discount: 50 },
      { price: 10, quantity: 2, discount: 0 },
    ],
    70,
    "a discounted and a plain item together",
  ],
];

for (const [items, want, what] of cases) {
  const got = total(items);
  if (typeof got !== "number" || Math.abs(got - want) > 1e-9) fail(`total(): ${what}: got ${got}, want ${want}`);
}
pass("total() takes the discount off every item");
