const LEVELS = { debug: 0, info: 1, warn: 2, error: 3 };
let logger = null;
let threshold = "info";
export function setLogger(fn) { logger = fn; }
export function setLevel(level) { threshold = level; }
export function log(level, message) {
  if (!logger) return;
  if (LEVELS[level] < LEVELS[threshold]) return;
  logger(level, message);
}
