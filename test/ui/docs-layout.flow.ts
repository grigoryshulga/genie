import { expect, test, type APIRequestContext } from "@playwright/test";
import { FLOW_DOC } from "./seed.ts";

async function seedDoc(request: APIRequestContext, suffix: string) {
  const path = `demo/guides/layout-${suffix}.md`;
  const title = `Проверка рабочего пространства (${suffix})`;
  const content = `---\ntitle: ${title}\ntype: guide\nstatus: current\ntags: [deploy, релиз]\npaths: [web/src/**/*.tsx]\n---\n# ${title}\n\nТекст инструкции.\n\n## Шаги\n\n1. Проверить сборку.\n\n## Команды\n\nОткройте [[demo/guides/run.md]].\n`;
  const saved = await request.post("/api/docs/page", { headers: { "x-genie": "1" }, data: { path, content, mode: "upsert" } });
  expect(saved.ok(), await saved.text()).toBe(true);
  const written = (await saved.json()) as { changeset?: { id: number } };
  // A write opens its author's changeset; publish it so the page is on main and other pages see it.
  if (written.changeset) {
    const published = await request.post(`/api/docs/changesets/${written.changeset.id}/publish`, { headers: { "x-genie": "1" } });
    expect(published.ok(), await published.text()).toBe(true);
  }
  return { path, title };
}

for (const theme of ["ink", "paper"]) {
  for (const width of [1920, 1440, 1024, 390]) {
    test(`centered reader and collapsible cards (${theme}, ${width})`, async ({ page, request }) => {
      const doc = await seedDoc(request, "reader");
      await page.setViewportSize({ width, height: 1000 });
      await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
      await page.goto(`/docs?page=${encodeURIComponent(doc.path)}`);
      await expect(page.getByRole("heading", { name: doc.title, exact: true })).toBeVisible();
      const article = page.locator(".doc-reading");
      const box = (await article.boundingBox())!;
      const view = (await page.locator(".doc-reader").boundingBox())!;
      expect(Math.abs(box.x + box.width / 2 - view.x - view.width / 2)).toBeLessThan(2);
      const tags = page.getByLabel("Теги статьи", { exact: true });
      await expect(tags).toHaveCount(1);
      const tagBox = (await tags.boundingBox())!;
      const title = (await page.getByRole("heading", { name: doc.title, exact: true }).boundingBox())!;
      expect(tagBox.y + tagBox.height).toBeLessThanOrEqual(title.y);
      expect(Math.abs(tagBox.x + tagBox.width - box.x - box.width)).toBeLessThan(2);
      const aside = page.locator(".docs-aside");
      await expect(aside.locator(".g-disclosure-card")).toHaveCount(4);
      const paths = aside.getByRole("button", { name: "Описывает код", exact: true });
      await paths.focus();
      await page.keyboard.press("Space");
      await expect(paths).toHaveAttribute("aria-expanded", "false");
      await page.reload();
      await expect(paths).toHaveAttribute("aria-expanded", "false");
      await paths.click();
      await expect(aside.getByText("web/src/**/*.tsx", { exact: true })).toBeVisible();
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await page.screenshot({ path: `test-results/docs-reader-${theme}-${width}.png`, animations: "disabled" });
      const contents = aside.locator(".g-disclosure-card").filter({ has: page.getByRole("button", { name: "На странице", exact: true }) });
      await contents.getByRole("button", { name: "Команды", exact: true }).click();
      await expect(page.locator(".doc-body").getByRole("heading", { name: "Команды", exact: true })).toBeInViewport();
      const outgoing = aside.locator(".g-disclosure-card").filter({ has: page.getByRole("button", { name: "Ссылки со страницы", exact: true }) });
      await outgoing.getByRole("button", { name: /demo\/guides\/run\.md/ }).click();
      await expect(page).toHaveURL(new RegExp(`page=${encodeURIComponent(FLOW_DOC.path)}`));
      const backlinks = aside.locator(".g-disclosure-card").filter({ has: page.getByRole("button", { name: /^Ссылаются сюда/ }) });
      await backlinks.getByRole("button", { name: doc.title, exact: true }).click();
      await expect(page).toHaveURL(new RegExp(`page=${encodeURIComponent(doc.path)}`));
    });

    test(`fullscreen editor saves body and rail metadata (${theme}, ${width})`, async ({ page, request }) => {
      const doc = await seedDoc(request, "editor");
      await page.setViewportSize({ width, height: 1000 });
      await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
      await page.goto(`/docs/edit?page=${encodeURIComponent(doc.path)}`);
      const editor = page.locator(".doc-editor-fullscreen");
      await expect(editor).toBeVisible();
      await expect(page.locator(".g-topbar")).toBeHidden();
      const box = (await editor.boundingBox())!;
      expect(box.x).toBe(0); expect(box.y).toBe(0);
      expect(box.width).toBe(width); expect(box.height).toBe(1000);
      const narrow = width <= 900;
      if (narrow) await editor.getByRole("button", { name: "Свойства статьи", exact: true }).click();
      const properties = narrow ? page.getByRole("dialog", { name: "Свойства статьи", exact: true }) : page.getByRole("complementary", { name: "Свойства статьи", exact: true });
      await expect(properties.locator(".g-disclosure-card")).toHaveCount(3);
      await properties.getByLabel("Название статьи", { exact: true }).fill("Обновлённая инструкция");
      await properties.getByLabel("Тип", { exact: true }).selectOption("reference");
      await properties.getByLabel("Статус", { exact: true }).selectOption("draft");
      await properties.getByLabel("Проверена", { exact: true }).fill("2026-10-21");
      await properties.getByRole("textbox", { name: "Теги", exact: true }).fill("проверка");
      await properties.getByRole("textbox", { name: "Теги", exact: true }).press("Enter");
      if (narrow) {
        await page.keyboard.press("Escape");
        await expect(properties).toBeHidden();
        await expect(editor.getByRole("button", { name: "Свойства статьи", exact: true })).toBeFocused();
      }
      await editor.getByRole("button", { name: "Markdown", exact: true }).click();
      const source = editor.getByRole("textbox", { name: "Markdown статьи", exact: true });
      await source.fill("# Обновлённая инструкция\n\nПроверенный текст.\n");
      await editor.getByRole("button", { name: "Вместе", exact: true }).click();
      await expect(editor.locator(".doc-preview")).toContainText("Проверенный текст.");
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await page.screenshot({ path: `test-results/docs-editor-${theme}-${width}.png`, animations: "disabled" });
      await source.focus();
      await page.keyboard.press("Control+s");
      await expect(page).toHaveURL(new RegExp(`/docs\\?page=${encodeURIComponent(doc.path)}`));
      const saved = await (await request.get(`/api/docs/page?path=${encodeURIComponent(doc.path)}`)).json();
      expect(saved).toMatchObject({ title: "Обновлённая инструкция", type: "reference", status: "draft", verified: "2026-10-21" });
      expect(saved.tags).toContain("проверка");
      expect(saved.content).toContain("Проверенный текст.");
      await expect(page.locator(".g-topbar")).toBeVisible();
    });
  }
}

test("diagnostics remain readable in a keyboard collapsible card", async ({ page, request }) => {
  const doc = await (await request.get(`/api/docs/page?path=${encodeURIComponent(FLOW_DOC.path)}`)).json();
  await page.route("**/api/docs/page?*", (route) => route.fulfill({ json: { ...doc, diagnostics: ['Unknown frontmatter field "extra"'] } }));
  await page.goto(`/docs?page=${encodeURIComponent(FLOW_DOC.path)}`);
  const card = page.locator(".g-disclosure-card").filter({ has: page.getByRole("button", { name: "Диагностика", exact: true }) });
  const toggle = card.getByRole("button", { name: "Диагностика", exact: true });
  await expect(card.locator(".doc-diags")).toBeVisible();
  await toggle.focus();
  await page.keyboard.press("Space");
  await expect(card.locator(".doc-diags")).toBeHidden();
  await page.keyboard.press("Enter");
  await expect(card.locator(".doc-diags")).toBeVisible();
});
