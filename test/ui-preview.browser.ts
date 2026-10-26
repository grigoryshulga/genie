import { expect, test } from "@playwright/test";

for (const theme of ["ink", "paper"]) for (const width of [1440, 390]) {
  test(`${theme} ${width}: real board and task detail render from fixtures`, async ({ page }) => {
    const errors: string[] = []; page.on("pageerror", e => errors.push(e.message));
    await page.goto(`/__preview?theme=${theme}&width=${width}&route=/board`);
    const app = page.frameLocator("#app");
    await expect(app.locator("html")).toHaveAttribute("data-theme", theme);
    await expect(app.locator("body")).toContainText("DM-2");
    expect(await app.locator("html").evaluate(el => el.clientWidth)).toBe(width);
    await page.getByLabel("Route").fill("/task/DM-2");
    await page.getByRole("button", { name: "Apply" }).click();
    await expect(app.locator("body")).toContainText("DM-2");
    expect(errors).toEqual([]);
    await page.screenshot({ path: `test-results/ui-preview-${theme}-${width}.png` });
  });
}

test("state controls show empty, pending and failure states and recover", async ({ page }) => {
  await page.goto("/__preview?route=/board");
  const app = page.frameLocator("#app");
  await expect(app.locator("body")).toContainText("DM-2");
  for (const state of ["empty", "loading", "error", "populated"]) {
    await page.getByLabel("State").selectOption(state);
    await page.getByRole("button", { name: "Apply" }).click();
    if (state === "populated") await expect(app.locator("body")).toContainText("DM-2");
    else {
      await expect(app.locator("body")).not.toContainText("DM-2");
      if (state === "empty") await expect(app.getByRole("heading", { name: "Здесь тихо" })).toBeVisible();
      if (state === "loading") await expect(app.locator('.g-board-skeleton[aria-busy="true"]')).not.toHaveCount(0);
      if (state === "error") await expect(app.locator("body")).toContainText("simulated server unavailable");
    }
  }
});

test("recorded docs, catalogue and settings render without frontend exceptions", async ({ page }) => {
  const errors: string[] = []; page.on("pageerror", e => errors.push(e.message));
  for (const route of ["/docs?page=demo/guides/deploy.md", "/agents", "/settings/me"]) {
    await page.goto(`/__preview?route=${encodeURIComponent(route)}`);
    await expect(page.frameLocator("#app").locator("main")).toBeVisible();
    await expect(page.frameLocator("#app").locator("body")).not.toContainText("No UI fixture");
  }
  expect(errors).toEqual([]);
});

for (const theme of ["ink", "paper"]) for (const width of [1440, 768, 390]) {
  test(`role workspace ${theme} ${width}: prompt, settings and compact model controls`, async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.route("**/api/agent-config", async (route) => {
      const response = await route.fetch();
      const cfg = await response.json();
      cfg.mcp.find((s: { id: string }) => s.id === "knowledge").tools = Array.from({ length: 60 }, (_, i) => ({ name: `read_document_${i + 1}` }));
      await route.fulfill({ response, json: cfg });
    });
    await page.goto(`/__preview?theme=${theme}&width=${width}&route=${encodeURIComponent("/agents?id=integration-analyst")}`);
    const app = page.frameLocator("#app");
    await expect(app.getByRole("region", { name: "Промпт роли", exact: true })).toContainText("Investigate the task");
    expect(await app.locator(".role-runtime").evaluate((el) => el.getBoundingClientRect().height)).toBeLessThan(120);
    await app.getByRole("tab", { name: "Промпт", exact: true }).focus();
    await page.keyboard.press("ArrowRight");
    await expect(app.getByRole("tab", { name: "Настройки", exact: true })).toHaveAttribute("aria-selected", "true");
    await expect(app.locator(".ag-mcp-connection")).toHaveCount(3);
    await expect(app.locator(".ag-mcp-connection li").first()).not.toBeVisible();
    const createRole = app.getByRole("region", { name: "Роли", exact: true }).getByRole("button", { name: "Новая роль", exact: true });
    await expect(createRole).toBeVisible();
    await createRole.click();
    await expect(app.getByRole("dialog", { name: "Новая роль", exact: true })).toBeVisible();
    await app.getByRole("dialog").getByRole("button", { name: "Отмена", exact: true }).click();
    await app.getByRole("button", { name: "Редактировать роль", exact: true }).click();
    await expect(app.getByRole("dialog")).toHaveCount(0);
    await expect(app.getByLabel("Название", { exact: true })).toHaveValue("Аналитик интеграций");
    await app.locator(".split-main").evaluate((el) => el.scrollTo(0, 0));
    await app.locator("body").evaluate(() => window.scrollTo(0, 0));
    await page.screenshot({ path: `test-results/role-settings-${theme}-${width}.png`, fullPage: true });
    await app.getByLabel("Размышление", { exact: true }).selectOption("high");
    const widths = await app.locator(".role-field-grid").first().evaluate((el) => ({ fields: el.getBoundingClientRect().width, editor: el.closest("form")!.getBoundingClientRect().width }));
    expect(widths.editor - widths.fields).toBeLessThan(4);
    await expect(app.locator("body")).not.toContainText("Ограничения и технические ключи");
    await expect(app.locator("body")).not.toContainText("Технические сведения и обслуживание");
    await expect(app.locator("body")).not.toContainText("Используется:");
    const tools = app.locator(".ag-mcp-tool-picker");
    await tools.locator("summary").click();
    await app.getByRole("searchbox", { name: "Поиск инструментов knowledge" }).fill("read_document_48");
    await expect(tools.getByRole("checkbox")).toHaveCount(1);
    await tools.getByRole("checkbox").uncheck();
    await app.getByLabel("Описание", { exact: true }).fill("Черновик");
    await app.getByRole("tab", { name: "Промпт", exact: true }).click();
    await app.getByLabel("Текст промпта").fill("Updated role instructions");
    await app.getByRole("tab", { name: "Настройки", exact: true }).click();
    await expect(app.getByLabel("Описание", { exact: true })).toHaveValue("Черновик");
    await expect(app.getByLabel("Размышление", { exact: true })).toHaveValue("high");
    await app.getByRole("button", { name: "Сохранить", exact: true }).click();
    await expect(app.getByRole("alert")).toContainText("read-only");
    expect(await app.locator("html").evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);

    await app.getByRole("button", { name: "Отмена", exact: true }).click();
    await app.getByRole("tab", { name: "Промпт", exact: true }).click();
    await expect(app.getByRole("region", { name: "Промпт роли", exact: true })).toBeVisible();
    await app.locator(".split-main").evaluate((el) => el.scrollTo(0, 0));
    await app.locator("body").evaluate(() => window.scrollTo(0, 0));
    await page.screenshot({ path: `test-results/role-prompt-${theme}-${width}.png`, fullPage: true });
    expect(errors).toEqual([]);
  });
}

test("role history has absolute timestamps, field snapshots and stable pages", async ({ page }) => {
  await page.goto(`/__preview?route=${encodeURIComponent("/agents?id=integration-analyst&view=history")}`);
  const app = page.frameLocator("#app");
  await expect(app.locator(".config-history-table > tbody > tr")).toHaveCount(20);
  await expect(app.locator("time").first()).toHaveAttribute("datetime", /2026-/);
  await app.getByRole("button", { name: "Сравнить ревизию 47", exact: true }).click();
  const diff = app.getByRole("table", { name: "Изменения ревизии 47" });
  await expect(diff).toContainText("thinking");
  await expect(diff).toContainText("low");
  await expect(diff).toContainText("medium");
  await page.screenshot({ path: "test-results/role-history.png", fullPage: true });
  await app.getByRole("button", { name: "Старее", exact: true }).click();
  await expect(app.locator(".config-history-pagination")).toContainText("Страница 2");
  await expect(app.locator(".config-history-table > tbody > tr")).toHaveCount(7);
  await expect(app.getByRole("button", { name: "Старее", exact: true })).toBeDisabled();
  await app.getByRole("button", { name: "Новее", exact: true }).click();
  await expect(app.getByRole("button", { name: "Сравнить ревизию 47", exact: true })).toBeVisible();
});


for (const theme of ["ink", "paper"]) for (const width of [1920, 2560]) {
  test(`role content column ${theme} ${width}: centred across view, edit and history`, async ({ page }) => {
    await page.setViewportSize({ width, height: 1080 });
    await page.addInitScript((theme) => localStorage.setItem("genie.appearance.theme", theme), theme);
    await page.goto("/agents?id=integration-analyst");
    await expect(page.getByRole("region", { name: "Промпт роли", exact: true })).toBeVisible();
    const column = await page.locator(".role-body").evaluate((el) => {
      const box = el.getBoundingClientRect(), style = getComputedStyle(el);
      const left = Number.parseFloat(style.paddingLeft), right = Number.parseFloat(style.paddingRight);
      const edges = [".role-header", ".role-tabs"].map((selector) => {
        const other = document.querySelector(selector)!;
        return other.getBoundingClientRect().left + Number.parseFloat(getComputedStyle(other).paddingLeft);
      });
      return { width: box.width - left - right, left, right, start: box.left + left, edges };
    });
    expect(column.width).toBeLessThanOrEqual(841);
    expect(column.width).toBeGreaterThan(800);
    expect(Math.abs(column.left - column.right)).toBeLessThan(1);
    for (const edge of column.edges) expect(Math.abs(edge - column.start)).toBeLessThan(1);
    await page.getByRole("tab", { name: "Настройки", exact: true }).click();
    await page.getByRole("button", { name: "Редактировать роль", exact: true }).click();
    await expect(page.getByLabel("Название", { exact: true })).toHaveValue("Аналитик интеграций");
    const editor = await page.locator(".role-editor").boundingBox();
    expect(editor!.width).toBeCloseTo(column.width, 0);
    await page.locator(".split-main").evaluate((el) => el.scrollTo(0, 0));
    await page.screenshot({ path: `test-results/role-column-${theme}-${width}.png` });
    await page.getByRole("button", { name: "Отмена", exact: true }).click();
    await page.getByRole("tab", { name: "История", exact: true }).click();
    await expect(page.locator(".config-history-table > tbody > tr")).toHaveCount(20);
    const history = await page.locator(".config-history").boundingBox();
    expect(history!.width).toBeCloseTo(column.width, 0);
    expect(await page.locator("html").evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
  });
}

for (const theme of ["ink", "paper"]) for (const width of [1040, 1440]) {
  test(`topbar overflow menu ${theme} ${width}: every item overlays the page`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.addInitScript((theme) => localStorage.setItem("genie.appearance.theme", theme), theme);
    await page.goto("/agents");
    const more = page.getByRole("button", { name: "Ещё", exact: true });
    await more.click();
    const menu = page.getByRole("menu", { name: "Ещё", exact: true });
    await expect(menu).toBeVisible();
    const covered = await menu.getByRole("menuitem").evaluateAll((items) => items.flatMap((item) => {
      const box = item.getBoundingClientRect();
      const hit = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
      return item.contains(hit) ? [] : [item.textContent];
    }));
    expect(covered, "All menu items must receive clicks above the page").toEqual([]);
    await page.screenshot({ path: `test-results/topbar-overflow-${theme}-${width}.png` });
    await menu.getByRole("menuitem", { name: "Наблюдения", exact: true }).click();
    await expect(page).toHaveURL(/\/observations$/);
    await more.click();
    await page.keyboard.press("Escape");
    await expect(menu).toBeHidden();
    await expect(more).toBeFocused();
  });
}


test("the shared header search filters roles without a second search field", async ({ page }) => {
  await page.goto("/agents?id=analyst&view=settings");
  const roles = page.getByRole("region", { name: "Роли", exact: true });
  const items = roles.locator(".pick");
  await expect(items).not.toHaveCount(0);
  const count = await items.count();
  expect(count).toBeGreaterThan(1);
  await expect(roles.getByRole("searchbox")).toHaveCount(0);
  const search = page.getByRole("searchbox", { name: "Поиск агентов и инструментов", exact: true }).filter({ visible: true });
  await search.fill("ИНТЕГРАЦИЙ");
  await expect(items).toHaveCount(1);
  await expect(items).toContainText("Аналитик интеграций");
  await search.fill("reviewer");
  await expect(items).toHaveCount(1);
  await expect(items).toContainText("Ревьюер");
  await search.fill("Изучает документацию и задачи");
  await expect(items).toHaveCount(1);
  await expect(items).toContainText("Аналитик интеграций");
  await search.fill("no-such-role");
  await expect(items).toHaveCount(0);
  await expect(roles).toContainText("Ничего не найдено.");
  await search.press("Escape");
  await expect(items).toHaveCount(count);
  const params = new URL(page.url()).searchParams;
  expect(params.has("q")).toBe(false);
  expect(params.get("id")).toBe("analyst");
  expect(params.get("view")).toBe("settings");
});

for (const theme of ["ink", "paper"]) {
  test(`role diagnostics ${theme}: per-role counts replace the list summary`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.addInitScript((theme) => localStorage.setItem("genie.appearance.theme", theme), theme);
    await page.route("**/api/agent-config", async (route) => {
      const response = await route.fetch();
      const cfg = await response.json();
      cfg.problems.push(
        { item: "role:analyst", level: "error", message: "A second invalid role setting" },
        { item: "role:analyst", level: "warning", message: "An unavailable tool" },
      );
      cfg.problems = cfg.problems.filter((p: { item: string }) => p.item !== "role:tester");
      await route.fulfill({ response, json: cfg });
    });
    await page.goto("/agents?id=analyst");
    const roles = page.getByRole("region", { name: "Роли", exact: true });
    await expect(roles.locator(".ag-problem-summary")).toHaveCount(0);
    const analyst = roles.locator(".pick").filter({ has: page.locator(".role-name", { hasText: /^Аналитик$/ }) });
    const errors = analyst.getByRole("img", { name: "Ошибок: 2", exact: true });
    await expect(errors).toHaveText("2");
    await expect(errors).toHaveAttribute("title", /A second invalid role setting/);
    await expect(analyst.getByRole("img", { name: "Предупреждений: 1", exact: true })).toHaveText("1");
    const integration = roles.locator(".pick").filter({ hasText: "Аналитик интеграций" });
    await expect(integration.getByRole("img", { name: "Предупреждений: 1", exact: true })).toBeVisible();
    await expect(integration.getByRole("img", { name: /Ошибок:/ })).toHaveCount(0);
    await expect(roles.locator(".pick").filter({ hasText: "Тестировщик" }).getByRole("img")).toHaveCount(0);
    const detail = page.getByRole("region", { name: "Роль", exact: true });
    await detail.locator(".ag-problem-summary > summary").click();
    await expect(detail.locator(".ag-problems")).toContainText("the role has no model");
    await detail.locator(".ag-problem-summary > summary").click();
    await page.screenshot({ path: `test-results/role-diagnostics-${theme}.png` });
  });
}

for (const theme of ["ink", "paper"]) for (const width of [1440, 768, 390]) {
  test(`template workspace ${theme} ${width}: inline composition and one draft across views`, async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto(`/__preview?theme=${theme}&width=${width}&route=${encodeURIComponent("/agents?tab=templates&id=full")}`);
    const app = page.frameLocator("#app");
    await expect(app.getByRole("heading", { name: "Полная", exact: true })).toBeVisible();
    await expect(app.locator(".template-member")).toHaveCount(5);
    await expect(app.locator(".template-relations")).toBeHidden();
    await app.locator(".template-flow-details > summary").click();
    await expect(app.locator(".template-relations li")).toHaveCount(9);
    await app.locator(".template-flow-details > summary").click();
    await expect(app.locator(".template-body")).not.toContainText("Используется:");
    await expect(app.locator(".template-body .config-history")).toHaveCount(0);
    const create = app.getByRole("button", { name: "Новый шаблон", exact: true });
    await expect(create).toHaveCount(1);
    await create.click();
    await expect(app.getByRole("dialog", { name: "Новый шаблон", exact: true })).toBeVisible();
    await app.getByRole("dialog").getByRole("button", { name: "Отмена", exact: true }).click();
    await app.getByRole("button", { name: "Редактировать шаблон", exact: true }).click();
    await expect(app.getByRole("dialog")).toHaveCount(0);
    const member = app.locator(".template-member").first();
    await member.getByLabel("Ключ", { exact: true }).fill("lead");
    await member.getByLabel("Имя", { exact: true }).fill("sherlock");
    await member.getByLabel("Инструкции участнику", { exact: true }).fill("Preserve the evidence");
    await member.getByLabel("Размышление", { exact: true }).selectOption("high");
    await app.getByRole("tab", { name: "Настройки", exact: true }).click();
    await app.getByLabel("Описание", { exact: true }).fill("Единый черновик");
    await app.locator(".split-main").evaluate((el) => el.scrollTo(0, 0));
    await page.screenshot({ path: `test-results/template-settings-${theme}-${width}.png`, fullPage: true });
    await app.getByRole("tab", { name: "Команда", exact: true }).click();
    await expect(member.getByLabel("Ключ", { exact: true })).toHaveValue("lead");
    await expect(member.getByLabel("Инструкции участнику", { exact: true })).toHaveValue("Preserve the evidence");
    await expect(member.getByLabel("Размышление", { exact: true })).toHaveValue("high");
    await expect(app.locator("select[multiple]")).toHaveCount(0);
    await app.getByRole("button", { name: "Добавить участника", exact: true }).click();
    await expect(app.locator(".template-member")).toHaveCount(6);
    await app.locator(".template-member").last().getByRole("button", { name: "Удалить участника" }).click();
    await expect(app.locator(".template-member")).toHaveCount(5);
    await app.locator(".split-main").evaluate((el) => el.scrollTo(0, 0));
    await page.screenshot({ path: `test-results/template-edit-${theme}-${width}.png`, fullPage: true });
    await app.getByRole("button", { name: "Сохранить", exact: true }).click();
    await expect(app.getByRole("alert")).toContainText("read-only");
    await app.getByRole("button", { name: "Отмена", exact: true }).click();
    await expect(app.locator(".template-editor")).toHaveCount(0);
    await app.getByRole("tab", { name: "Промпты", exact: true }).click();
    await expect(app.locator(".template-kickoff")).toHaveCount(5);
    await app.locator(".template-kickoff > summary").first().click();
    await expect(app.locator(".template-kickoff pre").first()).toContainText("TASK-1");
    await app.getByRole("tab", { name: "Команда", exact: true }).click();
    await app.locator(".split-main").evaluate((el) => el.scrollTo(0, 0));
    await page.screenshot({ path: `test-results/template-team-${theme}-${width}.png`, fullPage: true });
    expect(await app.locator("html").evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
    expect(await app.locator(".config-tabs [role=tab]").evaluateAll((tabs) => tabs.every((tab) => tab.getBoundingClientRect().right <= window.innerWidth))).toBe(true);
    expect(errors).toEqual([]);
  });
}

test("template history uses JSON field comparisons and stable 20-row pages", async ({ page }) => {
  await page.goto("/agents?tab=templates&id=full&view=history");
  await expect(page.locator(".config-history-table > tbody > tr")).toHaveCount(20);
  await page.getByRole("button", { name: "Сравнить ревизию 127", exact: true }).click();
  const diff = page.getByRole("table", { name: "Изменения ревизии 127" });
  await expect(diff).toContainText("charter");
  await expect(diff).toContainText("Revision 26");
  await expect(diff).toContainText("Revision 27");
  await expect(page.locator("time").first()).toHaveAttribute("datetime", "2026-05-03T18:00:00Z");
  await page.screenshot({ path: "test-results/template-history.png" });
  await page.getByRole("button", { name: "Старее", exact: true }).click();
  await expect(page.locator(".config-history-table > tbody > tr")).toHaveCount(7);
  await expect(page.getByRole("button", { name: "Старее", exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "Новее", exact: true }).click();
  await expect(page.getByRole("button", { name: "Сравнить ревизию 127", exact: true })).toBeVisible();
});

test("template list uses the shared search and guards an unsaved composition", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/agents?tab=templates&id=full");
  const list = page.getByRole("region", { name: "Шаблоны команд", exact: true });
  await expect(list.getByRole("searchbox")).toHaveCount(0);
  await expect(list.locator(".ag-problem-summary")).toHaveCount(0);
  await expect(list.getByRole("img", { name: /Предупреждений:/ }).first()).toBeVisible();
  const search = page.getByRole("searchbox", { name: "Поиск агентов и инструментов", exact: true }).filter({ visible: true });
  await search.fill("abap");
  await expect(list.locator(".pick")).toHaveCount(1);
  await search.press("Escape");
  await page.getByRole("button", { name: "Редактировать шаблон", exact: true }).click();
  await list.locator(".pick").filter({ hasText: "ABAP" }).click();
  const confirm = page.getByRole("dialog", { name: "Отменить изменения шаблона?", exact: true });
  await expect(confirm).toBeVisible();
  await confirm.getByRole("button", { name: "Отмена", exact: true }).click();
  await expect(page.locator(".template-editor")).toBeVisible();
  await list.locator(".pick").filter({ hasText: "ABAP" }).click();
  await page.getByRole("button", { name: "Отменить и перейти", exact: true }).click();
  await expect(page.getByRole("heading", { name: "ABAP", exact: true })).toBeVisible();
  await expect(page.locator(".template-editor")).toHaveCount(0);
});

for (const theme of ["ink", "paper"]) for (const width of [1920, 2560]) {
  test(`template content column ${theme} ${width}: readable graph and aligned editor`, async ({ page }) => {
    await page.setViewportSize({ width, height: 1080 });
    await page.addInitScript((theme) => localStorage.setItem("genie.appearance.theme", theme), theme);
    await page.goto("/agents?tab=templates&id=full");
    await expect(page.getByRole("heading", { name: "Полная", exact: true })).toBeVisible();
    const column = await page.locator(".template-body").evaluate((el) => {
      const box = el.getBoundingClientRect(), style = getComputedStyle(el);
      return { width: box.width - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight), left: parseFloat(style.paddingLeft), right: parseFloat(style.paddingRight) };
    });
    expect(column.width).toBeLessThanOrEqual(841);
    expect(column.width).toBeGreaterThan(800);
    expect(Math.abs(column.left - column.right)).toBeLessThan(1);
    await expect(page.getByRole("status", { name: "Масштаб схемы" })).toHaveText("100%");
    await page.getByRole("button", { name: "Редактировать шаблон", exact: true }).click();
    const editor = await page.locator(".template-editor").boundingBox();
    expect(editor!.width).toBeCloseTo(column.width, 0);
    await page.screenshot({ path: `test-results/template-column-${theme}-${width}.png` });
  });
}

test("template readers can inspect and export without administrator actions", async ({ page }) => {
  await page.route("**/api/auth/me", async (route) => { const response = await route.fetch(); const data = await response.json(); data.user.isAdmin = false; await route.fulfill({ response, json: data }); });
  await page.goto("/agents?tab=templates&id=full&view=history");
  await expect(page.getByRole("heading", { name: "Полная", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Экспорт шаблона", exact: true })).toBeVisible();
  for (const name of ["Новый шаблон", "Импорт шаблона", "Редактировать шаблон", "Дополнительные действия шаблона"]) await expect(page.getByRole("button", { name, exact: true })).toHaveCount(0);
  await expect(page.getByRole("tab", { name: "История", exact: true })).toHaveCount(0);
});
