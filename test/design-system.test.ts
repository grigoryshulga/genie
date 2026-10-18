import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { buildExport, outputPath, readTokens, resolveValue, serializeExport, sourcePath } from "../scripts/design-system/export.mjs";

const css = readFileSync(sourcePath, "utf8");
const bundle = JSON.parse(readFileSync(outputPath, "utf8"));

test("portable tokens stay synchronized with CSS across all twelve appearances", () => {
  assert.equal(readFileSync(outputPath, "utf8"), serializeExport(buildExport(css)));
  const groups = Object.keys(bundle).filter((key) => !key.startsWith("$"));
  assert.equal(groups.length, 12);
  const baseline = Object.keys(bundle[groups[0]]).sort();
  for (const group of groups) assert.deepEqual(Object.keys(bundle[group]).sort(), baseline);
  const dark = readTokens(css, "dark", "violet");
  const light = readTokens(css, "light", "violet");
  assert.notEqual(resolveValue("yellow", dark), resolveValue("yellow", light));
  assert.notEqual(resolveValue("role-text-analyst", dark), resolveValue("role-text-analyst", light));
  assert.equal(resolveValue("status-review", light), resolveValue("green", light));
  assert.equal(resolveValue("status-approved", light), resolveValue("blue", light));
  assert.throws(() => resolveValue("a", { a: "var(--b)", b: "var(--a)" }), /Circular/);
  assert.throws(() => resolveValue("a", { a: "var(--missing)" }), /Missing/);
});

test("filled actions retain readable foregrounds in every exported appearance", () => {
  const luminance = (components: number[]) => components.map((c) => c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)
    .reduce((sum, c, i) => sum + c * [0.2126, 0.7152, 0.0722][i], 0);
  for (const group of Object.keys(bundle)) {
    if (group.startsWith("$")) continue;
    for (const [background, foreground] of [["accent", "on-accent"], ["amber", "amber-foreground"], ["red", "on-danger"]]) {
      const entries: Record<string, { $value: { components: number[] } }> = bundle[group];
      const a = luminance(entries[background].$value.components);
      const b = luminance(entries[foreground].$value.components);
      const contrast = (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
      assert.ok(contrast >= 4.5, `${group}: ${foreground}/${background} contrast ${contrast}`);
    }
  }
});
