# Logging

The library says nothing by itself: the host decides where the messages go.

- The library never writes to the console. `console.*` in `src/` is forbidden.
- Messages go through `src/log.js`: `log(level, message)` passes them to the logger
  installed with `setLogger(logger)`; before one is installed nothing is called.
- Levels, from the least to the most severe: `debug`, `info`, `warn`, `error`.
  `setLevel(level)` sets the threshold, the default is `info`: `debug` is dropped,
  `info` and above are passed on. `setLevel("warn")` drops `debug` and `info`.
- The logger receives the level name first and the message second: `logger("warn", "…")`.
