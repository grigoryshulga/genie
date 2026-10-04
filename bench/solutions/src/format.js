import { subtotal } from "./report.js";
export function money(value) { return (Math.round((value + Number.EPSILON) * 100) / 100).toFixed(2); }
export function line(item) { return `${item.name} × ${item.quantity} — ${money(subtotal(item))}`; }
