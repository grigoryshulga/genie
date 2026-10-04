// The report: what one item is worth and what everything is worth together.

import { log } from "./log.js";

/** What one item is worth before its discount. */
function gross(item) {
  return item.price * item.quantity;
}

/**
 * What one item is worth: `price × quantity` less its `discount` per cent,
 * rounded to cents.
 */
export function subtotal(item) {
  return round(gross(item) - (gross(item) * item.discount) / 100);
}

/** The sum of the items. */
export function total(items) {
  log("debug", `totalling ${items.length} item(s)`);
  return round(items.reduce((sum, item) => sum + gross(item), 0));
}

function round(value) {
  return Math.round((value + Number.EPSILON) * 100) / 100;
}
