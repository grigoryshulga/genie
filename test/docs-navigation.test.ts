import { test } from "node:test";
import assert from "node:assert/strict";
import { articlePath, articleTitle, buildDocNavigation, docSections, sectionTrail } from "../web/src/entities/doc/navigation.ts";
import type { DocPage } from "../web/src/entities/doc/model.ts";

// Navigation consumes only title/path; real page metadata stays attached for rendering.
function page(path: string, title: string): DocPage {
  return { path, title, rootId: "vault", type: null, status: null, summary: null, tags: [], aliases: [], paths: null, related: [], verified: null, updated: null, contentHash: "", headings: [], diagnostics: [], stale: false, staleReasons: [], space: path.split("/")[0], project: "shop" };
}

test("sections take article titles from landing pages and hide the storage root", () => {
  const root = buildDocNavigation([
    page("shop/getting-started/index.md", "Начало работы"),
    page("shop/getting-started/install.md", "Установка"),
    page("shop/getting-started/advanced/README.md", "Для опытных пользователей"),
    page("shop/getting-started/advanced/api-guide.md", "api-guide"),
    page("other/private.md", "Secret"),
  ], ["shop"]);
  assert.equal(root.path, "shop/");
  assert.equal(root.count, 4);
  const section = root.sections[0];
  assert.equal(section.title, "Начало работы");
  assert.equal(section.landing?.path, "shop/getting-started/index.md");
  assert.deepEqual(section.pages.map((p) => p.title), ["Установка"]);
  assert.equal(section.sections[0].title, "Для опытных пользователей");
  assert.equal(articleTitle(section.sections[0].pages[0]), "Api guide");
  assert.deepEqual(sectionTrail(root, "shop/getting-started/advanced/api-guide.md").map((s) => s.title), ["Документация", "Начало работы", "Для опытных пользователей"]);
});

test("multiple roots stay distinct, index is preferred, and legacy documents are preserved", () => {
  const root = buildDocNavigation([
    page("shop/index.md", "Main guide"), page("shop/README.md", "Read me"),
    page("handbook/index.md", "Handbook"), page("shop/raw_section/old-file.md", "old-file"),
  ], ["shop", "handbook"]);
  assert.equal(root.count, 4);
  assert.deepEqual(root.sections.map((s) => s.title), ["Handbook", "Main guide"]);
  const shop = root.sections[1];
  assert.equal(shop.landing?.path, "shop/index.md");
  assert.equal(shop.pages[0].title, "Read me");
  assert.equal(shop.sections[0].title, "Raw section");
  assert.equal(docSections(root).length, 4);
  assert.equal(buildDocNavigation([], []).count, 0);
  assert.deepEqual(buildDocNavigation([], ["shop", "handbook"]).sections.map((s) => s.path), ["handbook/", "shop/"]);
});

test("article, note and section creation stay within the selected parent", () => {
  assert.equal(articlePath("shop/guides/", "start", "page", "2026-10-17"), "shop/guides/start.md");
  assert.equal(articlePath("shop/guides/", "idea", "note", "2026-10-17"), "shop/guides/inbox/2026-10-17-idea.md");
  assert.equal(articlePath("shop/", "getting-started", "section", "2026-10-17"), "shop/getting-started/index.md");
});
