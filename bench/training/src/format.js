// Printing money and inventory lines.

import { subtotal } from "./report.js";

/** Money with two decimals, e.g. `"12.30"`. */
export function money(value) {
  return value.toFixed(2);
}

/** One inventory line, e.g. `"Огурцы × 3 — 30.00"`. */
export function line(item) {
  return `${item.name} × ${item.quantity} — ${money(subtotal(item))}`;
}
