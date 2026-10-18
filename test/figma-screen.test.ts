import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";
import { pluginOutput } from "../scripts/design-system/build-plugin.mjs";
import { createFigmaPluginHost } from "./fixtures/figma-plugin.mjs";

const bundle = JSON.parse(readFileSync(new URL("../design-system/tokens.json", import.meta.url), "utf8"));
const source = readFileSync(pluginOutput, "utf8");
function host() {
  const p = createFigmaPluginHost(); runInNewContext(source, { figma: p.figma, __html__: "" }); return p;
}
function capture() {
  const frame = (key: string, x = 0) => ({ kind: "frame", key, name: key, x, y: 0, width: 120, height: 40, fill: [0, 0, 0, 0], opacity: 1,
    border: Array.from({ length: 4 }, () => ({ width: 0, color: [0, 0, 0, 0] })), radius: [0, 0, 0, 0], clip: true, children: [] as object[] });
  const root = frame("root"); root.width = 1440; root.height = 960;
  const button = { ...frame("button", 20), component: "button.btn" };
  button.children.push({ kind: "text", key: "label", name: "Label", x: 30, y: 5, width: 80, height: 20, text: "Настоящая задача",
    typography: { family: "Geist", weight: 400, size: 13, lineHeight: 19.5, letterSpacing: 0, color: [...bundle["dark-violet"].text.$value.components, 1] } });
  root.children.push(button);
  return { format: "genie-screen", version: 1, name: "/task/GNE-25", mode: "dark-violet", root, warnings: [] };
}
test("screen import preserves editable text, coordinates, source components and token bindings", async () => {
  const p = host();
  await p.figma.ui.onmessage({ type: "import", mode: "dark-violet", tokens: bundle["dark-violet"], buildLibrary: true });
  const existing = [...p.page.children];
  await p.figma.ui.onmessage({ type: "import-screen", screen: capture() });
  const result = p.messages.at(-1); assert.equal(result.ok, true, result.text);
  assert.equal(result.screen.componentInstances, 1); assert.ok(result.screen.boundPaints >= 1);
  const screen = p.nodes.get(result.screen.screenId); assert.equal(screen.width, 1440);
  assert.match(screen.name, /Genie \/ dark-violet \/ Capture/);
  const instance = screen.children[0]; assert.equal(instance.type, "INSTANCE"); assert.equal(instance.x, 20);
  const main = await instance.getMainComponentAsync();
  assert.equal(main.children[0].characters, "Настоящая задача"); assert.ok(main.children[0].textStyleId);
  assert.equal(main.children[0].textAutoResize, "WIDTH_AND_HEIGHT");
  assert.ok(main.children[0].componentPropertyReferences.characters);
  assert.ok(existing.every((n) => p.page.children.includes(n)));
  await p.figma.ui.onmessage({ type: "import-screen", screen: capture() });
  assert.notEqual(p.messages.at(-1).screen.screenId, screen.id); assert.ok(p.nodes.has(screen.id));
});
test("invalid geometry, SVG and duplicate keys fail before canvas mutation", async () => {
  for (const mutate of [
    (x: ReturnType<typeof capture>) => { x.root.width = NaN; },
    (x: ReturnType<typeof capture>) => { x.root.children.push({ ...x.root, children: [] }); },
    (x: ReturnType<typeof capture>) => { x.root.children.push({ kind: "svg", key: "svg", name: "svg", x: 0, y: 0, width: 10, height: 10, svg: '<svg><image href="https://example.com"/></svg>' }); },
  ]) {
    const p = host(); const screen = capture(); mutate(screen);
    await p.figma.ui.onmessage({ type: "import-screen", screen });
    assert.equal(p.messages.at(-1).ok, false); assert.equal(p.page.children.length, 0);
  }
});
test("identical captured controls share one source component", async () => {
  const p = host(); const screen = capture();
  screen.root.children = [0, 100].map((x, i) => ({ ...screen.root, key: `control-${i}`, name: "button", component: "button", x, width: 80, height: 32, children: [] }));
  await p.figma.ui.onmessage({ type: "import-screen", screen });
  const result = p.messages.at(-1); assert.equal(result.ok, true, result.text);
  assert.equal(result.screen.componentInstances, 2); assert.equal(result.screen.sourceComponents, 1);
  const imported = p.nodes.get(result.screen.screenId);
  assert.equal(await imported.children[0].getMainComponentAsync(), await imported.children[1].getMainComponentAsync());
});
test("missing source font fails before writes; fallback is explicit", async () => {
  const p = host(); p.figma.availableFonts = p.figma.availableFonts.filter((f) => f.fontName.family === "Inter");
  await p.figma.ui.onmessage({ type: "import-screen", screen: capture() });
  assert.equal(p.messages.at(-1).ok, false); assert.equal(p.page.children.length, 0);
  await p.figma.ui.onmessage({ type: "import-screen", screen: capture(), allowFallback: true });
  assert.equal(p.messages.at(-1).ok, true);
});
test("construction errors remove only the new partial screen", async () => {
  const p = host(); const existing = p.figma.createFrame(); p.page.appendChild(existing);
  const screen = capture(); screen.root.children.push({ kind: "svg", key: "svg", name: "svg", x: 0, y: 0, width: 10, height: 10, svg: '<svg width="10" height="10"/>' });
  p.figma.createNodeFromSvg = () => { throw new Error("Native SVG import failed"); };
  await p.figma.ui.onmessage({ type: "import-screen", screen });
  assert.equal(p.messages.at(-1).ok, false); assert.deepEqual(p.page.children, [existing]);
});
