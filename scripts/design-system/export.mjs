import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";

export const sourcePath = new URL("../../web/src/shared/ui/g-tokens.css", import.meta.url);
export const outputPath = new URL("../../design-system/tokens.json", import.meta.url);

/** The two ink-and-paper themes. Ink is the base rule; paper overlays it on the same element. */
export const themes = ["ink", "paper"];

const OVERLAY = { ink: null, paper: '[data-theme="paper"]' };

/** A small reader: `:root` is the base layer, the theme's own rule is overlaid on top of it. */
export function readTokens(css, theme) {
  const values = {};
  const wanted = new Set([":root", OVERLAY[theme]].filter(Boolean));
  for (const [, selector, body] of css.replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (!wanted.has(selector.trim())) continue;
    for (const [, key, value] of body.matchAll(/--([\w-]+):\s*([^;]+);/g)) values[key] = value.trim();
  }
  return values;
}

export function resolveValue(key, values, seen = []) {
  if (seen.includes(key)) throw new Error(`Circular token: ${[...seen, key].join(" -> ")}`);
  if (!(key in values)) throw new Error(`Missing token: ${key}`);
  return values[key].replace(/var\(--([\w-]+)\)/g, (_, ref) => resolveValue(ref, values, [...seen, key]));
}

export function parseColor(value) {
  const hex = /^#([\da-f]{6})([\da-f]{2})?$/i.exec(value);
  const rgb = /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)(?:\s*[,/]\s*([\d.]+))?\s*\)$/i.exec(value);
  if (!hex && !rgb) throw new Error(`Unsupported color: ${value}`);
  const components = hex
    ? [1, 3, 5].map((i) => Number.parseInt(hex[1].slice(i - 1, i + 1), 16) / 255)
    : rgb.slice(1, 4).map((part) => Number(part) / 255);
  const alpha = hex ? (hex[2] ? Number.parseInt(hex[2], 16) / 255 : 1) : rgb[4] ? Number(rgb[4]) : 1;
  return { colorSpace: "srgb", components, alpha };
}

const SHADOW = /^0 ([\d.]+)px ([\d.]+)px ((?:rgba?|#)[^)]*\)?)$/;

/** A token's DTCG entry, or null for an expression the format cannot state (a ch measure, an easing curve). */
function token(key, value, expression) {
  const extensions = { "com.genie": { css: `var(--${key})`, expression } };
  const entry = (type, parsed) => ({ $type: type, $value: parsed, $extensions: extensions });
  if (/^#[\da-f]{6}([\da-f]{2})?$/i.test(value) || /^rgba?\(/i.test(value)) return entry("color", parseColor(value));
  const dimension = /^(-?[\d.]+)(px|rem)$/.exec(value);
  if (dimension) return entry("dimension", { value: Number.parseFloat(dimension[1]), unit: dimension[2] });
  if (/^[\d.]+$/.test(value)) return entry("number", Number(value));
  // Font stacks are a list of names; the scale's geometry is in CSS pixels like everything else.
  if (/-font-/.test(key)) return entry("fontFamily", value.split(",").map((part) => part.trim().replace(/^"|"$/g, "")));
  const shadow = SHADOW.exec(value);
  if (shadow) {
    const [, y, blur, color] = shadow;
    return entry("shadow", {
      color: parseColor(color),
      offsetX: { value: 0, unit: "px" },
      offsetY: { value: Number(y), unit: "px" },
      blur: { value: Number(blur), unit: "px" },
      spread: { value: 0, unit: "px" },
    });
  }
  // DTCG dimensions accept px/rem, not font-relative em/ch; clamp() has no single value either.
  // This Figma bundle also leaves motion values in CSS rather than exporting duration/easing tokens.
  if (/^-?[\d.]+(em|ch|ms|s)$/.test(value) || /^(cubic-bezier|steps)\(/.test(value) || /^clamp\(/.test(value)) return null;
  throw new Error(`Unsupported value for ${key}: ${value}`);
}

export function buildExport(css) {
  const result = { $description: "Generated from web/src/shared/ui/g-tokens.css. One group per theme (ink, paper), each fully resolved. Geometry is in CSS pixels. Regenerate: node scripts/design-system/export.mjs" };
  for (const theme of themes) {
    const values = readTokens(css, theme);
    result[theme] = Object.fromEntries(Object.entries(values).flatMap(([key, expression]) => {
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
