import { log } from "./log.js";
function gross(item) { return item.price * item.quantity; }
export function subtotal(item) { return round(gross(item) - (gross(item) * item.discount) / 100); }
export function total(items) {
  log("debug", `totalling ${items.length} item(s)`);
  return round(items.reduce((sum, item) => sum + subtotal(item), 0));
}
export function render(items) {
  const lines = items.map((item) => `${item.name} × ${item.quantity} — ${subtotal(item).toFixed(2)}`);
  lines.push(`Итого: ${total(items).toFixed(2)}`);
  return lines.join("\n");
}
function round(value) { return Math.round((value + Number.EPSILON) * 100) / 100; }
