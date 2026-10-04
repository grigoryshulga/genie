// Logging: what the library says.

/** Say something at `level` (`debug`, `info`, `warn`, `error`). */
export function log(level, message) {
  console.log(`[${level}] ${message}`);
}
