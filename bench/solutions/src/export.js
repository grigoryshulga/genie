import { subtotal } from "./report.js";
export const CSV_HEADER = "name;quantity;price;total";
export function exportCsv(items, options = {}) {
  const limit = Number.isInteger(options.limit) ? options.limit : items.length;
  const lines = [CSV_HEADER];
  for (const item of items.slice(0, limit)) {
    lines.push([item.name, item.quantity, item.price, subtotal(item)].join(";"));
  }
  return lines.join("\n");
}
export function exportJson(items) { return JSON.stringify(items); }
