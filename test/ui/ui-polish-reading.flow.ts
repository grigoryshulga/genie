import { expect, test } from "@playwright/test";
import { FLOW_DOC } from "./seed.ts";

for (const theme of ["ink", "paper"]) {
  test(`palette distinguishes a failed document search from an empty search (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 640 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    let failed = true;
    let release!: () => void;
    const held = new Promise<void>((resolve) => { release = resolve; });
    await page.route("**/api/docs/search?*", async (route) => {
      if (!failed) return route.continue();
      await held;
      return route.fulfill({ status: 503, json: { error: "Temporary search failure" } });
    });
    await page.goto("/board");
    await expect(page.getByRole("button", { name: "Новая задача", exact: true })).toBeVisible();
    await page.keyboard.press("Control+k");
    const popup = page.getByRole("dialog", { name: "Команды", exact: true });
    const search = popup.getByRole("combobox");
    const list = popup.getByRole("listbox", { name: "Результаты поиска", exact: true });
    await search.fill("а");
    await expect(list).toHaveAttribute("aria-busy", "true");
    await expect(popup.getByRole("status").filter({ hasText: "Ищем в документации" })).toBeVisible();
    release();
    const error = popup.getByRole("alert");
    await expect(error).toContainText("Не удалось выполнить поиск по документации");
    await expect(search).toHaveValue("а");
    await expect(list).not.toHaveAttribute("aria-busy", "true");
    const bounds = (await popup.boundingBox())!;
    expect(bounds.y).toBeGreaterThanOrEqual(0);
    expect(bounds.y + bounds.height).toBeLessThanOrEqual(640);
    await search.focus();
    await page.keyboard.press("Tab");
    const retry = error.getByRole("button", { name: "Повторить", exact: true });
    await expect(retry).toBeFocused();
    failed = false;
    await page.keyboard.press("Enter");
    await expect(error).toBeHidden();
    await expect(search).toBeFocused();
    await expect(search).toHaveValue("а");
    await search.fill("несуществующаястраница123456");
    await expect(popup.getByRole("status").filter({ hasText: "В документации ничего не найдено" })).toBeVisible();
    await expect(error).toBeHidden();
  });

  test(`palette reserves space for article titles and status instead of paths (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    const response = await page.request.get("/api/docs/search?q=запустить&limit=30");
    const data = await response.json();
    const original = data.results[0];
    expect(original).toBeTruthy();
    const path = "demo/очень-длинный-раздел/вложенный-каталог/страница.md";
    const long = "Очень длинное название статьи с важными деталями для команды";
    await page.route("**/api/docs/search?*", (route) => route.fulfill({ json: { ...data, results: [{ ...original, title: "Как устроен деплой", path, status: "current" }, { ...original, title: long, path: `${path}-2`, status: "current" }] } }));
    await page.goto("/board");
    await expect(page.getByRole("button", { name: "Новая задача", exact: true })).toBeVisible();
    await page.keyboard.press("Control+k");
    const popup = page.getByRole("dialog", { name: "Команды", exact: true });
    await popup.getByRole("checkbox", { name: "Только документация", exact: true }).check();
    await popup.getByRole("combobox").fill("деплой");
    const hits = popup.getByRole("option").filter({ has: page.locator(".pal-path") });
    await expect(hits).toHaveCount(2);
    for (const hit of await hits.all()) {
      await hit.scrollIntoViewIfNeeded();
      const label = (await hit.locator(".pal-label").boundingBox())!;
      const meta = (await hit.locator(".pal-path").boundingBox())!;
      expect(meta.y).toBeGreaterThanOrEqual(label.y + label.height);
      await expect(hit.getByText("Актуальна", { exact: true })).toBeInViewport({ ratio: 1 });
      await expect(hit.locator(".pal-title")).toHaveCSS("text-overflow", "ellipsis");
    }
    const title = hits.first().locator(".pal-title");
    const size = await title.evaluate((node) => ({ client: node.clientWidth, text: node.scrollWidth }));
    expect(size.text).toBeLessThanOrEqual(size.client);
    await expect(hits.last()).toHaveAccessibleName(new RegExp(long));
  });

  for (const width of [390, 1440]) {
    test(`article reader retains a heading hierarchy (${theme}, ${width})`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
      const original = await (await page.request.get(`/api/docs/page?path=${encodeURIComponent(FLOW_DOC.path)}`)).json();
      const content = `# ${original.title}\n\nОбычный текст.\n\n## Раздел\n\nТекст раздела.\n\n### Подраздел\n\nТекст подраздела.\n\n#### Детали\n\nТекст деталей.`;
      await page.route("**/api/docs/page?*", (route) => route.fulfill({ json: { ...original, content } }));
      await page.goto(`/docs?page=${encodeURIComponent(FLOW_DOC.path)}`);
      const article = page.locator(".doc-reading .doc-body .md");
      await expect(article.getByRole("heading", { name: "Раздел", level: 2, exact: true })).toBeVisible();
      await expect(article.getByRole("heading", { name: "Подраздел", level: 3, exact: true })).toBeVisible();
      await expect(article.getByRole("heading", { name: "Детали", level: 4, exact: true })).toBeVisible();
      const sizes = await article.evaluate((node) => ["h2", "h3", "h4", "p"].map((tag) => parseFloat(getComputedStyle(node.querySelector(tag)!).fontSize)));
      expect(sizes[0]).toBeGreaterThan(sizes[1]);
      expect(sizes[1]).toBeGreaterThan(sizes[2]);
      expect(sizes[2]).toBeGreaterThan(sizes[3]);
    });
  }

  test(`token name has a persistent label and roomy phone input (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto("/settings/me/tokens");
    const name = page.getByRole("textbox", { name: "Имя токена", exact: true });
    await expect(page.locator(".tokens-issue-form label")).toHaveText("Имя токена");
    const bounds = (await name.boundingBox())!;
    expect(bounds.height).toBeGreaterThanOrEqual(40);
    expect(bounds.x + bounds.width).toBeLessThanOrEqual(390);
    await name.fill("Локальная проверка");
    await page.getByRole("button", { name: "Как подключить", exact: true }).click();
    await page.getByRole("button", { name: "Токены", exact: true }).click();
    await expect(name).toHaveValue("Локальная проверка");
  });

  test(`desktop article metadata removal controls keep a dense usable target (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto(`/docs/edit?page=${encodeURIComponent(FLOW_DOC.path)}`);
    const buttons = page.locator(".doc-chips button:visible");
    await expect(buttons.first()).toBeVisible();
    for (const button of await buttons.all()) {
      const bounds = (await button.boundingBox())!;
      expect(bounds.width).toBeGreaterThanOrEqual(32);
      expect(bounds.height).toBeGreaterThanOrEqual(32);
    }
  });
}
