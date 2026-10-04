// R4 — logging follows docs/logging.md: no console, level honoured, logger installed.

import path from "node:path";

import { fail, jsFiles, load, pass, read, workspaces } from "./util.mjs";

const [ws] = workspaces();

for (const file of jsFiles(path.join(ws, "src"))) {
  const rel = path.relative(ws, file);
  if (/\bconsole\s*\./.test(read(ws, rel))) fail(`${rel} calls console.*; docs/logging.md forbids it`);
}

const log = await load(ws, "src/log.js");
for (const name of ["log", "setLogger", "setLevel"]) {
  if (typeof log[name] !== "function") fail(`src/log.js must export ${name}()`);
}

const seen = [];
log.setLogger((level, message) => seen.push(`${level}:${message}`));
log.setLevel("warn");
log.log("debug", "d");
log.log("info", "i");
log.log("warn", "w");
log.log("error", "e");
if (seen.join(",") !== "warn:w,error:e") fail(`setLevel("warn") passed ${JSON.stringify(seen)}, want ["warn:w","error:e"]`);

seen.length = 0;
log.setLevel("info");
log.log("debug", "d");
log.log("info", "i");
log.log("error", "e");
if (seen.join(",") !== "info:i,error:e") fail(`the default level passed ${JSON.stringify(seen)}, want ["info:i","error:e"]`);

seen.length = 0;
log.setLevel("debug");
log.log("debug", "d");
if (seen.join(",") !== "debug:d") fail(`setLevel("debug") passed ${JSON.stringify(seen)}, want ["debug:d"]`);
pass("src/ never touches the console and src/log.js honours the level");
