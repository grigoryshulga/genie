import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { buildExport, outputPath, readTokens, resolveValue, serializeExport, sourcePath, themes } from "../scripts/design-system/export.mjs";

const css = readFileSync(sourcePath, "utf8");
const bundle = JSON.parse(readFileSync(outputPath, "utf8"));

type Entry = { $value: { components: number[] } };

test("portable tokens stay synchronized with the CSS in both themes", () => {
  assert.equal(readFileSync(outputPath, "utf8"), serializeExport(buildExport(css)));
  const groups = Object.keys(bundle).filter((key) => !key.startsWith("$"));
  assert.deepEqual(groups, themes);
  const baseline = Object.keys(bundle[groups[0]]).sort();
  assert.ok(baseline.includes("g-text"));
  for (const group of groups) assert.deepEqual(Object.keys(bundle[group]).sort(), baseline);
  const ink = readTokens(css, "ink");
  const paper = readTokens(css, "paper");
  assert.notEqual(resolveValue("g-accent-text", ink), resolveValue("g-accent-text", paper));
  assert.notEqual(resolveValue("g-bg", ink), resolveValue("g-bg", paper));
  // Paper overrides ink; a token only ink names is deliberately inherited by paper.
  assert.equal(resolveValue("g-green", ink), resolveValue("g-green", paper));
  assert.throws(() => resolveValue("a", { a: "var(--b)", b: "var(--a)" }), /Circular/);
  assert.throws(() => resolveValue("a", { a: "var(--missing)" }), /Missing/);
});

test("filled actions retain readable foregrounds in both themes", () => {
  const luminance = (components: number[]) => components.map((c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4))
    .reduce((sum, c, i) => sum + c * [0.2126, 0.7152, 0.0722][i], 0);
  for (const group of Object.keys(bundle)) {
    if (group.startsWith("$")) continue;
    for (const [background, foreground] of [["g-accent", "g-on-accent"], ["g-signal", "g-on-signal"], ["g-danger", "g-on-danger"]]) {
      const entries = bundle[group] as Record<string, Entry>;
      const a = luminance(entries[background].$value.components);
      const b = luminance(entries[foreground].$value.components);
      const contrast = (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
      assert.ok(contrast >= 4.5, `${group}: ${foreground}/${background} contrast ${contrast}`);
    }
  }
});
