import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";
import { buildPlugin, pluginOutput } from "../scripts/design-system/build-plugin.mjs";
import { createFigmaPluginHost } from "./fixtures/figma-plugin.mjs";

const bundle = JSON.parse(readFileSync(new URL("../design-system/tokens.json", import.meta.url), "utf8"));
const source = readFileSync(pluginOutput, "utf8");
function plugin() {
  const host = createFigmaPluginHost();
  runInNewContext(source, { figma: host.figma, __html__: "" });
  return host;
}

test("the distributable plugin contains the current source files", () => {
  assert.equal(source, buildPlugin());
});

test("full import builds native component families, styles and an instance-based screen", async () => {
  const p = plugin();
  await p.figma.ui.onmessage({ type: "import", mode: "dark-violet", tokens: bundle["dark-violet"], buildLibrary: true });
  assert.equal(p.messages.at(-1)?.ok, true, p.messages.at(-1)?.text);
  assert.equal(p.textStyles.length, 14);
  const library = p.page.children.find((n) => n.name.endsWith(" / Library"));
  assert.ok(library);
  const sets = library.children.filter((n) => n.type === "COMPONENT_SET");
  assert.equal(sets.length, 5);
  assert.deepEqual(sets.map((n) => n.children.length), [20, 20, 3, 4, 2]);
  for (const set of sets) {
    assert.ok(set.width > 0 && set.height > 0);
    assert.equal(new Set(set.children.map((n) => `${n.x},${n.y}`)).size, set.children.length);
  }
  const screen = p.page.children.find((n) => n.name.endsWith(" / Task screen"));
  assert.ok(screen);
  assert.equal(screen.width, 1440);
  const task = screen.children.find((n) => n.name === "Task");
  const body = task?.children.find((n) => n.name === "Body");
  assert.equal(body?.primaryAxisSizingMode, "FIXED");
  assert.equal(body?.counterAxisSizingMode, "AUTO");
  assert.ok(screen.findAllWithCriteria({ types: ["INSTANCE"] }).length >= 6);
  const label = sets[0].children[0].children.find((n) => n.name === "Label");
  assert.ok(label?.componentPropertyReferences.characters);
  assert.ok(label?.textStyleId);
  const button = sets[0].children[0];
  assert.ok(button.boundVariables.cornerRadius);
  assert.ok(button.boundVariables.height);
  assert.ok(p.textStyles.every((s) => s.boundVariables.fontSize));
});

test("reimport preserves asset and instance IDs plus user content and screen position", async () => {
  const p = plugin();
  const request = { type: "import", mode: "light-teal", tokens: bundle["light-teal"], buildLibrary: true };
  await p.figma.ui.onmessage(request);
  assert.equal(p.messages.at(-1)?.ok, true, p.messages.at(-1)?.text);
  const before = [...p.nodes.keys()];
  const styleIds = p.textStyles.map((s) => s.id);
  const screen = p.page.children.find((n) => n.name.endsWith(" / Task screen"));
  assert.ok(screen);
  screen.x = 10000;
  const userLayer = p.figma.createFrame(); userLayer.name = "My annotation"; screen.appendChild(userLayer);
  await p.figma.ui.onmessage(request);
  assert.equal(p.messages.at(-1)?.ok, true, p.messages.at(-1)?.text);
  assert.deepEqual([...p.nodes.keys()], [...before, userLayer.id]);
  assert.deepEqual(p.textStyles.map((s) => s.id), styleIds);
  assert.equal(screen.x, 10000);
  assert.ok(screen.children.includes(userLayer));
});

test("missing fonts fail before creating assets; explicit fallback permits the build", async () => {
  const p = plugin();
  p.figma.availableFonts = p.figma.availableFonts.filter((f) => f.fontName.family === "Inter");
  const request = { type: "import", mode: "dark-violet", tokens: bundle["dark-violet"], buildLibrary: true };
  await p.figma.ui.onmessage(request);
  assert.equal(p.messages.at(-1)?.ok, false);
  assert.match(p.messages.at(-1)?.text ?? "", /Missing font Geist/);
  assert.equal(p.collections.length, 0);
  assert.equal(p.textStyles.length, 0);
  assert.equal(p.page.children.length, 0);
  await p.figma.ui.onmessage({ ...request, allowFallback: true });
  assert.equal(p.messages.at(-1)?.ok, true, p.messages.at(-1)?.text);
  assert.ok(p.textStyles.every((s) => s.fontName.family === "Inter"));
});

test("tokens-only import does not need fonts or create canvas content", async () => {
  const p = plugin();
  p.figma.availableFonts = [];
  await p.figma.ui.onmessage({ type: "import", mode: "dark-violet", tokens: bundle["dark-violet"] });
  assert.equal(p.messages.at(-1)?.ok, true);
  assert.equal(p.page.children.length, 0);
  assert.equal(p.textStyles.length, 0);
});

test("separate appearance imports use separate libraries on the same page", async () => {
  const p = plugin();
  for (const mode of ["dark-violet", "light-teal"]) {
    await p.figma.ui.onmessage({ type: "import", mode, tokens: bundle[mode], buildLibrary: true });
    assert.equal(p.messages.at(-1)?.ok, true, p.messages.at(-1)?.text);
  }
  assert.equal(p.collections.length, 2);
  assert.equal(p.textStyles.length, 28);
  assert.equal(p.page.children.length, 4);
  const libraries = p.page.children.filter((n) => n.name.endsWith(" / Library"));
  assert.ok(libraries[1].x > libraries[0].x + libraries[0].width);
});

test("conflicting user layers are retained and a corrected partial import can resume", async () => {
  const p = plugin();
  const conflict = p.figma.createFrame();
  conflict.name = "Genie / dark-violet / Library";
  p.page.appendChild(conflict);
  const userLayer = p.figma.createFrame(); userLayer.name = "Button/Default"; conflict.appendChild(userLayer);
  const request = { type: "import", mode: "dark-violet", tokens: bundle["dark-violet"], buildLibrary: true };
  await p.figma.ui.onmessage(request);
  assert.equal(p.messages.at(-1)?.ok, false);
  assert.match(p.messages.at(-1)?.text ?? "", /Name conflict.*Button\/Default/);
  assert.ok(conflict.children.includes(userLayer));
  userLayer.name = "My button sketch";
  await p.figma.ui.onmessage(request);
  assert.equal(p.messages.at(-1)?.ok, true, p.messages.at(-1)?.text);
  assert.equal(p.textStyles.length, 14);
  assert.equal(p.collections.length, 1);
  assert.ok(conflict.children.includes(userLayer));
});
