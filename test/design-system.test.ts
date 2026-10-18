import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";
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

type Variable = {
  id: string; name: string; resolvedType: string; variableCollectionId: string;
  values: Record<string, unknown>; syntax?: string;
  setValueForMode: (mode: string, value: unknown) => void;
  setVariableCodeSyntax: (platform: string, value: string) => void;
};

function plugin() {
  const variables: Variable[] = [];
  const collections: { id: string; name: string; defaultModeId: string }[] = [];
  const styles: { name?: string; effects?: unknown }[] = [];
  const messages: { ok: boolean; text: string }[] = [];
  const figma = {
    showUI: () => {},
    ui: { postMessage: (message: { ok: boolean; text: string }) => messages.push(message), onmessage: async (_: unknown) => {} },
    variables: {
      getLocalVariableCollectionsAsync: async () => collections,
      getLocalVariablesAsync: async () => variables,
      createVariableCollection: (name: string) => {
        const collection = { id: String(collections.length), name, defaultModeId: "default" };
        collections.push(collection);
        return collection;
      },
      createVariable: (name: string, collection: { id: string }, resolvedType: string) => {
        const variable: Variable = {
          id: String(variables.length), name, resolvedType, variableCollectionId: collection.id, values: {},
          setValueForMode(mode, value) { this.values[mode] = value; },
          setVariableCodeSyntax(_platform, value) { this.syntax = value; },
        };
        variables.push(variable);
        return variable;
      },
    },
    getLocalEffectStylesAsync: async () => styles,
    createEffectStyle: () => { const style = {}; styles.push(style); return style; },
  };
  runInNewContext(readFileSync(new URL("../design-system/figma/code.js", import.meta.url), "utf8"), { figma, __html__: "" });
  return { figma, variables, collections, styles, messages };
}

test("Figma importer preserves IDs, aliases and unrelated collections on reimport", async () => {
  const p = plugin();
  const other = p.figma.variables.createVariableCollection("Other library");
  const unrelated = p.figma.variables.createVariable("brand", other, "COLOR");
  unrelated.setValueForMode("default", "unchanged");
  const request = { type: "import", mode: "light-teal", tokens: bundle["light-teal"] };
  await p.figma.ui.onmessage(request);
  assert.equal(p.messages.at(-1)?.ok, true, p.messages.at(-1)?.text ?? "Importer returned no message");
  const ids = p.variables.map((v) => v.id);
  await p.figma.ui.onmessage(request);
  assert.equal(p.messages.at(-1)?.ok, true);
  assert.deepEqual(p.variables.map((v) => v.id), ids);
  assert.equal(p.collections.length, 2);
  assert.equal(p.styles.length, 1);
  assert.equal(unrelated.values.default, "unchanged");
  const review = p.variables.find((v) => v.name === "status/review");
  const green = p.variables.find((v) => v.name === "green");
  assert.deepEqual(JSON.parse(JSON.stringify(review?.values.default)), { type: "VARIABLE_ALIAS", id: green?.id });
  assert.equal(review?.syntax, "var(--status-review)");
});

test("Figma importer rejects malformed values before creating document assets", async () => {
  const p = plugin();
  const tokens = structuredClone(bundle["dark-violet"]);
  tokens.bg.$value.components[0] = 2;
  await p.figma.ui.onmessage({ type: "import", mode: "dark-violet", tokens });
  assert.equal(p.messages.at(-1)?.ok, false);
  assert.equal(p.collections.length, 0);
  assert.equal(p.variables.length, 0);
});
