import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";

export const sourcePath = new URL("../../web/src/shared/ui/tokens.css", import.meta.url);
export const outputPath = new URL("../../design-system/tokens.json", import.meta.url);
export const accents = ["violet", "blue", "teal", "green", "orange", "rose"];

/** This intentionally small reader accepts only the declarations used by tokens.css.
 * Unsupported expressions fail the export instead of silently approximating a token. */
export function readTokens(css, theme, accent) {
  const values = {};
  for (const [, selector, body] of css.replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const matches = selector.trim() === ":root"
      || selector.trim() === `:root[data-theme="${theme}"]`
      || selector.trim() === `:root[data-accent="${accent}"]`;
    if (matches) for (const [, key, value] of body.matchAll(/--([\w-]+):\s*([^;]+);/g)) values[key] = value.trim();
  }
  return values;
}

export function resolveValue(key, values, seen = []) {
  if (seen.includes(key)) throw new Error(`Circular token: ${[...seen, key].join(" -> ")}`);
  if (!(key in values)) throw new Error(`Missing token: ${key}`);
  const raw = values[key].replace(/var\(--([\w-]+)\)/g, (_, ref) => resolveValue(ref, values, [...seen, key]));
  const mix = /^color-mix\(in srgb, (#[\da-f]{6}) ([\d.]+)%, (#[\da-f]{6}|white|black)\)$/i.exec(raw);
  if (mix) {
    const a = parseColor(mix[1]);
    const b = parseColor(mix[3] === "white" ? "#ffffff" : mix[3] === "black" ? "#000000" : mix[3]);
    const weight = Number(mix[2]) / 100;
    return "#" + a.components.map((v, i) => Math.round((v * weight + b.components[i] * (1 - weight)) * 255).toString(16).padStart(2, "0")).join("");
  }
  if (raw.includes("color-mix(") || raw.includes("var(")) throw new Error(`Unsupported expression for ${key}: ${raw}`);
  return raw;
}

export function parseColor(hex) {
  if (!/^#[\da-f]{6}([\da-f]{2})?$/i.test(hex)) throw new Error(`Unsupported color: ${hex}`);
  return {
    colorSpace: "srgb",
    components: [1, 3, 5].map((i) => Number.parseInt(hex.slice(i, i + 2), 16) / 255),
    alpha: hex.length === 9 ? Number.parseInt(hex.slice(7), 16) / 255 : 1,
  };
}

function token(key, value, expression) {
  let type;
  let parsed;
  if (value.startsWith("#")) { type = "color"; parsed = parseColor(value); }
  else if (/^[\d.]+px$/.test(value)) { type = "dimension"; parsed = { value: Number.parseFloat(value), unit: "px" }; }
  else if (/^[\d.]+$/.test(value)) { type = "number"; parsed = Number(value); }
  else if (key === "sans" || key === "mono") {
    type = "fontFamily";
    parsed = value.split(",").map((part) => part.trim().replace(/^"|"$/g, ""));
  } else if (key === "shadow") {
    type = "shadow";
    const match = /^0 (\d+)px (\d+)px (?:rgba\(0, 0, 0, (0\.\d+)\)|rgb\(30 35 45 \/ (\d+)%\))$/.exec(value);
    if (!match) throw new Error(`Unsupported shadow: ${value}`);
    parsed = { color: { colorSpace: "srgb", components: match[3] ? [0, 0, 0] : [30 / 255, 35 / 255, 45 / 255], alpha: match[3] ? Number(match[3]) : Number(match[4]) / 100 }, offsetX: { value: 0, unit: "px" }, offsetY: { value: Number(match[1]), unit: "px" }, blur: { value: Number(match[2]), unit: "px" }, spread: { value: 0, unit: "px" } };
  } else if (key === "measure") {
    // ch depends on font metrics and cannot be represented as a fixed Figma dimension.
    return null;
  } else throw new Error(`Unsupported value for ${key}: ${value}`);
  return { $type: type, $value: parsed, $extensions: { "com.genie": { css: `var(--${key})`, expression } } };
}

export function buildExport(css) {
  const result = { $description: "Generated from web/src/shared/ui/tokens.css. Each group is a fully resolved theme/accent pair. Geometry is in CSS pixels. Regenerate: node scripts/design-system/export.mjs" };
  for (const theme of ["dark", "light"]) for (const accent of accents) {
    const values = readTokens(css, theme, accent);
    result[`${theme}-${accent}`] = Object.fromEntries(Object.entries(values).flatMap(([key, expression]) => {
      const entry = token(key, resolveValue(key, values), expression);
      return entry ? [[key, entry]] : [];
    }));
  }
  return result;
}

export function serializeExport(data) {
  // One token per line keeps generated changes reviewable.
  return "{\n" + Object.entries(data).map(([key, value]) => typeof value === "string"
    ? `  ${JSON.stringify(key)}: ${JSON.stringify(value)}`
    : `  ${JSON.stringify(key)}: {\n${Object.entries(value).map(([name, entry]) => `    ${JSON.stringify(name)}: ${JSON.stringify(entry)}`).join(",\n")}\n  }`).join(",\n") + "\n}\n";
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const output = serializeExport(buildExport(readFileSync(sourcePath, "utf8")));
  if (process.argv.includes("--check")) {
    if (readFileSync(outputPath, "utf8") !== output) throw new Error("Design token export is stale; run node scripts/design-system/export.mjs");
  } else writeFileSync(outputPath, output);
}
