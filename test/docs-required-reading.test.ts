import assert from "node:assert/strict";
import { mkdtemp, mkdir, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test, { type TestContext } from "node:test";
import { checkRequiredReading } from "../site/scripts/required-reading.mjs";

async function fixture(t: TestContext, pages: Record<string, string> = {}, required = ["genie/guide.md"]) {
  const vault = await mkdtemp(path.join(tmpdir(), "genie-required-reading-"));
  t.after(() => rm(vault, { recursive: true, force: true }));
  await mkdir(path.join(vault, ".genie"));
  await mkdir(path.join(vault, "genie"));
  const config = { spaces: {
    genie: { project: "genie", required: required.map(page => ({ page, roles: ["all"] })) },
    other: { project: "other", required: [{ page: "other/missing.md", roles: ["all"] }] },
  } };
  await writeFile(path.join(vault, ".genie/vault.json"), JSON.stringify(config));
  for (const [name, body] of Object.entries(pages)) {
    const file = path.join(vault, name);
    await mkdir(path.dirname(file), { recursive: true });
    await writeFile(file, body);
  }
  return vault;
}

test("required reading accepts exactly 12000 Unicode characters and ignores other projects", async t => {
  const vault = await fixture(t, { "genie/guide.md": "🧞".repeat(12_000) });
  assert.deepEqual(await checkRequiredReading(vault), []);
});

test("required reading reports oversized pages once, across repeated role mappings", async t => {
  const vault = await fixture(t, { "genie/guide.md": "é".repeat(12_000) + "🧞" }, ["genie/guide.md", "genie/guide.md"]);
  const errors = await checkRequiredReading(vault);
  assert.equal(errors.length, 1);
  assert.match(errors[0], /genie\/guide\.md has 12001 characters \(limit 12000\)/);
});

test("required reading excludes a leading BOM and LF or CRLF frontmatter from the size", async t => {
  for (const newline of ["\n", "\r\n"]) {
    const source = `\uFEFF---${newline}title: ${"a".repeat(12_001)}${newline}---${newline}${newline}${"a".repeat(12_000)}`;
    const vault = await fixture(t, { "genie/guide.md": source });
    assert.deepEqual(await checkRequiredReading(vault), []);
  }
});

test("required reading treats unclosed frontmatter as Markdown and accepts an EOF closing delimiter", async t => {
  const vault = await fixture(t, {
    "genie/open.md": `---\ntitle: ${"a".repeat(12_001)}`,
    "genie/closed.md": `---\ntitle: ${"a".repeat(12_001)}\n---`,
  }, ["genie/open.md", "genie/closed.md"]);
  const errors = await checkRequiredReading(vault);
  assert.equal(errors.length, 1);
  assert.match(errors[0], /genie\/open\.md has 12012 characters/);
});

test("required reading reports a missing file without truncating other diagnostics", async t => {
  const vault = await fixture(t, { "genie/large.md": "a".repeat(12_001) }, ["genie/missing.md", "genie/large.md"]);
  const errors = await checkRequiredReading(vault);
  assert.equal(errors.length, 2);
  assert.match(errors[0], /genie\/missing\.md is missing or unreadable/);
  assert.match(errors[1], /genie\/large\.md has 12001 characters/);
});

test("required reading rejects absolute, foreign, traversal and backslash paths", async t => {
  const vault = await fixture(t, {}, ["/tmp/page.md", "other/page.md", "genie/../other/page.md", "genie\\page.md", ""]);
  const errors = await checkRequiredReading(vault);
  assert.equal(errors.length, 5);
  for (const error of errors) assert.match(error, /must be a page inside genie\//);
});

test("required reading rejects symlink escapes while allowing links within the space", async t => {
  const vault = await fixture(t, { "genie/guide.md": "Guide", "other/guide.md": "Foreign" }, ["genie/outside.md", "genie/inside.md"]);
  await symlink("../other/guide.md", path.join(vault, "genie/outside.md"));
  await symlink("guide.md", path.join(vault, "genie/inside.md"));
  assert.deepEqual(await checkRequiredReading(vault), ["required reading: genie/outside.md resolves outside genie/"]);
});

test("required reading also rejects a symlink replacing the project root", async t => {
  const vault = await fixture(t, { "other/guide.md": "Foreign" });
  await rm(path.join(vault, "genie"), { recursive: true });
  await symlink("other", path.join(vault, "genie"));
  assert.deepEqual(await checkRequiredReading(vault), ["required reading: genie/guide.md resolves outside genie/"]);
});

test("required reading rejects broken configuration but permits an absent required list", async t => {
  const vault = await fixture(t);
  const config = path.join(vault, ".genie/vault.json");
  await writeFile(config, JSON.stringify({ spaces: { genie: { project: "genie" } } }));
  assert.deepEqual(await checkRequiredReading(vault), []);
  await writeFile(config, JSON.stringify({ spaces: { genie: { required: {} } } }));
  assert.match((await checkRequiredReading(vault))[0], /spaces.genie.required must be a list/);
  await writeFile(config, "{");
  assert.match((await checkRequiredReading(vault))[0], /cannot read .genie\/vault.json/);
});
