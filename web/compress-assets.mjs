import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { extname, resolve } from "node:path";
import { brotliCompressSync, constants, gzipSync } from "node:zlib";

const root = resolve(process.argv[2] ?? "web/dist");
const compressibleExtensions = new Set([".css", ".html", ".js", ".json", ".mjs", ".svg", ".txt"]);
const minimumBytes = 1024;
const maximumRatioPercent = 95;
if (!existsSync(resolve(root, "index.html"))) {
  throw new Error(`No web build found in ${root}; run npm run build:web first.`);
}

let compressedFiles = 0;
let variants = 0;

function filesUnder(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = resolve(dir, entry.name);
    return entry.isDirectory() ? filesUnder(path) : entry.isFile() ? [path] : [];
  });
}

for (const path of filesUnder(root)) {
  if (!compressibleExtensions.has(extname(path))) continue;
  const raw = readFileSync(path);
  if (raw.byteLength <= minimumBytes) continue;

  const compressed = [
    ["gz", gzipSync(raw, { level: constants.Z_BEST_COMPRESSION })],
    [
      "br",
      brotliCompressSync(raw, {
        params: {
          [constants.BROTLI_PARAM_QUALITY]: 11,
          [constants.BROTLI_PARAM_LGWIN]: 22,
        },
      }),
    ],
  ];
  let wroteVariant = false;

  for (const [encoding, data] of compressed) {
    if (data.byteLength * 100 > raw.byteLength * maximumRatioPercent) continue;
    writeFileSync(`${path}.${encoding}`, data);
    variants += 1;
    wroteVariant = true;
  }
  if (wroteVariant) compressedFiles += 1;
}

console.log(`Wrote ${variants} precompressed variants across ${compressedFiles} web assets.`);
