// The inventory as a text file.

import { subtotal } from "./report.js";

/** The header of the CSV export. */
export const CSV_HEADER = "name;quantity;price;total";

/** The items as CSV: the header and one line per item, `\n` between the lines. */
export function exportCsv(items) {
  const lines = [CSV_HEADER];
  for (const item of items) {
    lines.push([item.name, item.quantity, item.price, subtotal(item)].join(";"));
  }
  return lines.join("\n");
}
