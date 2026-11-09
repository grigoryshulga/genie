import { expect, test } from "@playwright/test";
import { STATUS_NAME } from "../../web/src/entities/task/model.ts";
import type { Task } from "../../web/src/shared/api/types.ts";

for (const theme of ["ink", "paper"]) {
  test(`questionnaire helper text contrasts with its sheet (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 1000 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.route("**/api/answer/contrast", (route) => route.fulfill({ json: { questionnaire: {
      id: 1, askedBy: "Аналитик", status: "open", questions: [{ n: 1, text: "Какой вариант выбрать?", why: "От ответа зависит план команды.", options: [] }],
    } } }));
    await page.goto("/answer?token=contrast");
    await expect(page.locator(".answer-why")).toBeVisible();
    const ratios = await page.locator(".answer-form .g-sheet").first().evaluate((sheet) => {
      const channels = (color: string) => (color.match(/[\d.]+/g) ?? []).slice(0, 3).map(Number);
      const luminance = (color: string) => channels(color).map((n) => { const v = n / 255; return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }).reduce((sum, n, i) => sum + n * [0.2126, 0.7152, 0.0722][i], 0);
      const background = luminance(getComputedStyle(sheet).backgroundColor);
      return [".answer-num", ".answer-why"].map((selector) => {
        const foreground = luminance(getComputedStyle(sheet.querySelector(selector)!).color);
        return (Math.max(foreground, background) + 0.05) / (Math.min(foreground, background) + 0.05);
      });
    });
    for (const ratio of ratios) expect(ratio).toBeGreaterThanOrEqual(4.5);
    await page.screenshot({ path: `test-results/answer-contrast-${theme}.png`, animations: "disabled" });
  });

  test(`phone board filter has a full touch target (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto("/board");
    const filters = page.getByRole("button", { name: /^Фильтры/ });
    await expect(filters).toBeVisible();
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
    expect((await filters.boundingBox())!.height).toBeGreaterThanOrEqual(40);
    const epic = page.locator('.bd-meta a.epic-chip').first();
    await expect(epic).toBeVisible();
    expect((await epic.boundingBox())!.height).toBeGreaterThanOrEqual(40);
    await expect(page.locator('.g-hints')).toBeHidden();
    const task: Task = await (await page.request.get("/api/tasks/DM-2")).json();
    await expect(page.locator('[data-task-id="DM-2"] .bd-state')).toContainText(STATUS_NAME[task.status]);
  });

  test(`first project uses accessible roomy fields (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 1000 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.route("**/api/auth/me", async (route) => {
      const response = await route.fetch();
      const session = await response.json();
      await route.fulfill({ json: { ...session, projects: [] } });
    });
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "Первый проект", exact: true })).toBeVisible();
    const fields = page.locator(".pj-form input");
    await expect(fields).toHaveCount(4);
    for (const field of await fields.all()) {
      expect((await field.boundingBox())!.height).toBeGreaterThanOrEqual(40);
      await expect(field).toHaveClass(/g-input/);
    }
    const code = page.getByRole("textbox", { name: "Код (латиница)", exact: true });
    await code.fill("Invalid Code");
    expect(await code.evaluate((node: HTMLInputElement) => node.checkValidity())).toBe(false);
  });

  for (const width of [390, 768]) {
    test(`task actions and navigation stay reachable (${theme}, ${width})`, async ({ page }) => {
      await page.setViewportSize({ width, height: 844 });
      await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
      await page.goto("/tasks");
      const filters = page.getByRole("button", { name: /^Фильтры/ });
      await expect(filters).toBeInViewport({ ratio: 1 });
      await filters.click();
      await expect(page.getByRole("dialog")).toBeVisible();
      await page.keyboard.press("Escape");

      const toggle = page.getByRole("button", { name: "Виды задач", exact: true });
      const bounds = (await toggle.boundingBox())!;
      expect(bounds.width).toBeGreaterThanOrEqual(40);
      expect(bounds.height).toBeGreaterThanOrEqual(40);
      for (const key of ["Tab", "Shift+Tab"]) {
        await toggle.click();
        await expect(page.getByRole("menu", { name: "Виды задач" })).toBeVisible();
        await page.keyboard.press(key);
        await expect(page.getByRole("menu", { name: "Виды задач" })).toBeHidden();
        expect(await page.evaluate(() => document.activeElement?.tagName)).not.toBe("BODY");
        expect(await page.evaluate(() => !!document.activeElement?.closest('[role="menu"]'))).toBe(false);
      }
    });

    test(`knowledge search results open and return (${theme}, ${width})`, async ({ page }) => {
      await page.setViewportSize({ width, height: 844 });
      await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
      await page.goto("/docs?q=запустить");
      const results = page.getByRole("list", { name: "Результаты поиска" });
      await expect(results).toBeVisible();
      await expect(page.getByRole("searchbox", { name: "Поиск по знаниям", exact: true })).toHaveValue("запустить");
      await results.getByRole("button").filter({ hasText: "Как запустить" }).first().click();
      await expect(page.locator(".doc-reading")).toBeVisible();
      await page.getByRole("button", { name: "К результатам поиска", exact: true }).click();
      await expect(results).toBeVisible();
      await expect(page).toHaveURL(/q=/);
    });
  }
}

test("login does not offer a missing social alternative", async ({ page }) => {
  await page.context().clearCookies();
  await page.route("**/api/auth/social", (route) => route.fulfill({ json: [] }));
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Войти", exact: true })).toBeVisible();
  await expect(page.locator(".auth-or")).toHaveCount(0);
});

test("merge publication waits for every page and keeps the phone result first", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 1000 });
  await page.route("**/api/docs/merges/99", (route) => route.fulfill({ json: {
    merge: { id: 99, status: "failed", changeset: 99, tip: "abcdef12", main: "12345678", attempts: 1 },
    pages: ["demo/a.md", "demo/b.md"].map((path) => ({ path, base: "base", ours: "main", theirs: "draft", marked: "<<<<<<< main\nmain\n=======\ndraft\n>>>>>>> draft" })),
  } }));
  await page.goto("/docs/changes/merge/99");
  const save = page.getByRole("button", { name: "Сохранить и опубликовать", exact: true });
  await expect(save).toBeDisabled();
  const result = page.getByRole("textbox", { name: "Результат слияния страницы demo/a.md", exact: true });
  await result.fill("Resolved A");
  await expect(save).toBeDisabled();
  await page.getByRole("button", { name: "demo/b.md", exact: true }).click();
  await page.getByRole("textbox", { name: "Результат слияния страницы demo/b.md", exact: true }).fill("Resolved B");
  await expect(save).toBeEnabled();
  const editor = (await page.locator(".changes-merge-pane.result").boundingBox())!;
  const reference = (await page.locator(".changes-merge-pane.ours").boundingBox())!;
  expect(editor.y + editor.height).toBeLessThanOrEqual(reference.y);
});

test("an empty project offers the existing task creation flow", async ({ page }) => {
  await page.route("**/api/tasks?*", (route) => route.fulfill({ json: [] }));
  await page.goto("/tasks");
  const empty = page.locator(".tl-empty");
  await expect(empty).toContainText("Создайте первую задачу");
  await empty.getByRole("button", { name: "Создать задачу", exact: true }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
});

test("a filtered empty task list can recover without leaving the screen", async ({ page }) => {
  await page.goto("/tasks?who=missing-person");
  const empty = page.locator(".tl-empty");
  await expect(empty).toContainText("Ни одна задача не подходит");
  await empty.getByRole("button", { name: "Сбросить фильтры", exact: true }).click();
  await expect(page.getByRole("list", { name: "Задачи", exact: true })).toBeVisible();
  await expect(page).not.toHaveURL(/who=/);
});

test("an empty project does not offer task creation to a viewer", async ({ page }) => {
  await page.route("**/api/auth/me", async (route) => {
    const response = await route.fetch();
    const session = await response.json();
    await route.fulfill({ json: { ...session, projects: session.projects.map((p: { role: string }) => ({ ...p, role: "viewer" })) } });
  });
  await page.route("**/api/tasks?*", (route) => route.fulfill({ json: [] }));
  await page.goto("/tasks");
  const empty = page.locator(".tl-empty");
  await expect(empty).toContainText("Участник проекта может создать первую задачу");
  await expect(empty.getByRole("button", { name: "Создать задачу", exact: true })).toHaveCount(0);
});

test("phone task content precedes supporting rail", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/task/DM-2");
  await expect(page.locator(".tp-main")).toBeVisible();
  const main = (await page.locator(".tp-main").boundingBox())!;
  const rail = (await page.locator(".tp-rail").boundingBox())!;
  expect(main.y).toBeLessThan(rail.y);
});

test("personal provider setup distinguishes failure and an empty configuration", async ({ page }) => {
  let failed = true;
  await page.route("**/api/me/llm-keys", async (route) => {
    if (failed) await route.fulfill({ status: 503, json: { error: "Unavailable" } });
    else {
      const response = await route.fetch();
      const data = await response.json();
      await route.fulfill({ json: { ...data, keys: {}, global: {}, oauth: {} } });
    }
  });
  await page.goto("/settings/me/keys");
  await expect(page.getByRole("alert")).toContainText("Не удалось загрузить провайдеров");
  failed = false;
  await page.getByRole("button", { name: "Повторить", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Провайдеры ещё не подключены" })).toBeVisible();
});

for (const theme of ["ink", "paper"]) {
  test(`management deep links reveal the active destination (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto("/manage/server/health");
    const current = page.getByRole("combobox", { name: "Раздел управления", exact: true });
    await expect(current).toBeInViewport({ ratio: 1 });
    await expect(current).toHaveValue("/manage/server/health");
    const summary = page.locator(".sv-verdict-text");
    await expect(summary).toBeVisible();
    const button = (await page.getByRole("button", { name: "Проверить снова" }).boundingBox())!;
    const text = (await summary.boundingBox())!;
    expect(button.y).toBeGreaterThanOrEqual(text.y + text.height);
    await current.selectOption("/manage/server/tools");
    await expect(page).toHaveURL(/\/manage\/server\/tools$/);
    await expect(current).toBeInViewport({ ratio: 1 });
    await expect(current).toHaveValue("/manage/server/tools");
    const heads = page.locator(".ag-section-head");
    await expect(heads.first()).toBeVisible();
    for (const head of await heads.all()) {
      const note = head.locator(".ag-section-note");
      const action = head.getByRole("button").first();
      if (await note.count() && await action.count()) {
        const n = (await note.boundingBox())!;
        const a = (await action.boundingBox())!;
        expect(a.y).toBeGreaterThanOrEqual(n.y + n.height);
      }
    }
  });

  test(`phone management titles and history identities remain readable (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 1000 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto("/manage/project/repos");
    const title = page.getByRole("heading", { name: "Репозитории и окружение", exact: true });
    await expect(title).toBeVisible();
    expect(await title.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
    await expect(title).toHaveCSS("white-space", "normal");
    await page.route("**/api/agent-config/history**", (route) => route.fulfill({ json: [
      { id: 1, user: "Анна Семёнова", item: "team:integration-review", path: "teams/integration-review-with-long-distinct-name.json", at: new Date().toISOString(), before: "old", after: "new" },
      { id: 2, user: "Михаил Александров", item: "role:integration-analyst", path: "agents/integration-analyst-with-another-distinct-name.md", at: new Date().toISOString(), before: "old", after: "new" },
    ] }));
    await page.goto("/manage/server/history");
    const rows = page.locator(".ag-history-row");
    await expect(rows).toHaveCount(2);
    for (const row of await rows.all()) {
      for (const selector of [".who", ".what"]) {
        const field = row.locator(selector);
        expect(await field.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
        await expect(field).toHaveCSS("white-space", "normal");
      }
    }
  });

  test(`phone personal navigation and Jev form fit the viewport (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 1000 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto("/settings/me/tokens");
    const section = page.getByRole("combobox", { name: "Личные настройки", exact: true });
    await expect(section).toHaveValue("tokens");
    await expect(section).toBeInViewport({ ratio: 1 });
    await section.selectOption("account");
    await expect(page).toHaveURL(/\/settings\/me\/account$/);
    await expect(page.locator("#pf-name")).toBeVisible();
    expect((await page.locator("#pf-name").boundingBox())!.height).toBeGreaterThanOrEqual(40);
    await page.goto("/manage/agents/jev");
    await expect(page.getByRole("group", { name: "Настройки Jev", exact: true })).toBeVisible();
    for (const card of await page.locator(".st-body .st-card:visible").all()) {
      const rect = (await card.boundingBox())!;
      expect(rect.x).toBeGreaterThanOrEqual(0);
      expect(rect.x + rect.width).toBeLessThanOrEqual(390);
    }
  });
}

test("public answer preserves input across a failed send and reports actual completion", async ({ page }) => {
  let sends = 0;
  let finish!: () => void;
  const pending = new Promise<void>((resolve) => { finish = resolve; });
  const questionnaire = { id: 1, task: "DM-2", askedBy: "Аналитик", status: "open", questions: [{ n: 1, text: "Что учесть?", why: "Это поможет команде.", options: [] }] };
  await page.route("**/api/answer/audit", async (route) => {
    if (route.request().method() === "GET") return route.fulfill({ json: { questionnaire } });
    sends++;
    if (sends === 1) {
      await pending;
      return route.fulfill({ status: 503, json: { error: "Temporary failure" } });
    }
    return route.fulfill({ json: { questionnaire: { ...questionnaire, status: "answered" }, complete: true } });
  });
  await page.goto("/answer?token=audit");
  const answer = page.getByRole("textbox", { name: "Ответ на вопрос 1", exact: true });
  await answer.fill("Сохранить сроки");
  const send = page.getByRole("button", { name: "Отправить ответы", exact: true });
  await send.click();
  await expect(send).toBeDisabled();
  await expect(answer).toBeDisabled();
  finish();
  await expect(page.getByRole("alert")).toContainText("Не удалось подтвердить отправку");
  await expect(answer).toHaveValue("Сохранить сроки");
  await expect(page.getByRole("heading", { name: /Нужны ваши ответы/ })).toBeVisible();
  await send.click();
  await expect(page.getByRole("heading", { name: "Ответы получены", exact: true })).toBeVisible();
  expect(sends).toBe(2);
});

for (const theme of ["ink", "paper"]) {
  test(`phone Changes filters fit without widening the document (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto("/docs/changes");
    const all = page.locator(".changes-filters").getByRole("button", { name: /^Все(?:\s|$)/ });
    await expect(all).toBeInViewport({ ratio: 1 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(390);
    await all.click();
    await expect(all).toHaveAttribute("aria-pressed", "true");
  });

  test(`phone create options have full targets and contain a long epic (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 640 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto("/board");
    await page.getByRole("button", { name: "Новая задача", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Новая задача", exact: true });
    await expect(dialog).toBeVisible();
    const options = dialog.locator(".opts");
    await options.scrollIntoViewIfNeeded();
    for (const field of await options.locator("input, select").all()) {
      expect((await field.boundingBox())!.height).toBeGreaterThanOrEqual(40);
    }
    const epic = dialog.getByRole("combobox", { name: "Эпик", exact: true });
    await epic.selectOption({ index: 1 });
    const bounds = (await options.boundingBox())!;
    for (const field of await options.locator("input, select").all()) {
      const box = (await field.boundingBox())!;
      expect(box.x).toBeGreaterThanOrEqual(bounds.x);
      expect(box.x + box.width).toBeLessThanOrEqual(bounds.x + bounds.width + 1);
    }
    await dialog.getByRole("button", { name: /^Создать/ }).scrollIntoViewIfNeeded();
    await expect(dialog.getByRole("button", { name: /^Создать/ })).toBeInViewport({ ratio: 1 });
  });

  test(`permission help fits the phone and describes its focused button (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto("/manage/agents/roles");
    await page.getByRole("tab", { name: "Настройки", exact: true }).click();
    const buttons = page.getByRole("button", { name: /^О разрешении/ });
    for (const button of [buttons.first(), buttons.last()]) {
      await button.focus();
      const description = await button.getAttribute("aria-describedby");
      expect(description).toBeTruthy();
      const tooltip = page.getByRole("tooltip");
      await expect(tooltip).toBeVisible();
      await expect(tooltip).toHaveAttribute("id", description!);
      await expect(tooltip).toBeInViewport({ ratio: 1 });
      await page.keyboard.press("Escape");
      await expect(tooltip).toBeHidden();
      await expect(button).toBeFocused();
    }
  });

  test(`active automation tab is visible on a phone deep link and after resizing (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto("/manage/agents/automations/sources");
    const active = page.locator('.st-tabs-band [aria-current="page"]');
    await expect(active).toHaveText("Источники");
    await expect(active).toBeInViewport({ ratio: 1 });
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
    await page.setViewportSize({ width: 320, height: 844 });
    await expect(active).toBeInViewport({ ratio: 1 });
    expect(await page.evaluate(() => window.scrollY)).toBe(0);
  });
}

test("health capacity guidance is localized and preserves the affected path", async ({ page }) => {
  await page.route("**/api/doctor", (route) => route.fulfill({ json: { checks: [{ area: "data", level: "warn", text: "3.0 GB free on the data disk (/tmp/genie-ui-audit)", hint: "worktrees and agent logs grow; keep a few GB free" }] } }));
  await page.goto("/manage/server/health");
  const checks = page.locator(".sv-checks");
  await expect(checks).toContainText("3,0 ГБ");
  await expect(checks).toContainText("/tmp/genie-ui-audit");
  await expect(checks).toContainText("Рабочие копии и журналы агентов растут");
  await expect(checks).not.toContainText("GB free");
});

test("an observation explains its missing contract and offers the contract editor", async ({ page }) => {
  await page.route("**/api/observation/cases/1", (route) => route.fulfill({ json: {
    case: { id: 1, key: "O-1", title: "Проверка после запуска", status: "proposed", policy: "guardrails", tasks: ["DM-2"], author: "owner", contractVersion: 0, failures: 0, created: "2026-11-01T00:00:00Z", updated: "2026-11-01T00:00:00Z", conclusion: "" },
    armProblems: ["no contract yet"], contracts: [], deployments: [], evaluations: [], evidence: [], followups: [], incidents: [],
  } }));
  await page.goto("/manage/agents/automations/observation/O-1");
  await expect(page.getByLabel("Почему наблюдение нельзя ждать поставки")).toContainText("Не задан контракт наблюдения");
  await page.getByRole("button", { name: "Задать контракт", exact: true }).click();
  await expect(page.locator(".obs-form textarea")).toBeVisible();
});

for (const theme of ["ink", "paper"]) {
  for (const width of [390, 768]) {
    test(`knowledge section keeps readable article rows and reachable sorting (${theme}, ${width})`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
      await page.goto("/docs?section=demo/guides/");
      const articles = page.locator(".doc-article-open");
      await expect(articles.first()).toBeVisible();
      for (const article of await articles.all()) {
        const title = (await article.locator(":scope > span").first().boundingBox())!;
        const meta = (await article.locator(".doc-article-meta").boundingBox())!;
        const date = (await article.locator(":scope > span").last().boundingBox())!;
        expect(title.width).toBeGreaterThan(width * 0.6);
        expect(meta.y).toBeGreaterThanOrEqual(title.y + title.height - 1);
        expect(date.y).toBeGreaterThanOrEqual(meta.y + meta.height - 1);
      }
      const sort = page.getByRole("combobox", { name: "Сортировка статей", exact: true });
      const bounds = (await sort.boundingBox())!;
      expect(bounds.x).toBeGreaterThanOrEqual(0);
      expect(bounds.x + bounds.width).toBeLessThanOrEqual(width);
      await sort.selectOption("newest");
      const title = await articles.first().locator("strong").innerText();
      await page.getByRole("searchbox", { name: "Поиск статей раздела", exact: true }).fill(title);
      await expect(articles).toHaveCount(1);
      await articles.first().click();
      await expect(page).toHaveURL(/[?&]page=/);
      await expect(page.locator(".doc-head").getByRole("heading", { name: title, exact: true })).toBeVisible();
    });
  }

  test(`role toolbar uses full phone targets (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto("/manage/agents/roles");
    const buttons = page.locator(".config-icon-group button");
    await expect(buttons.first()).toBeVisible();
    for (const button of await buttons.all()) {
      const bounds = (await button.boundingBox())!;
      expect(bounds.width).toBeGreaterThanOrEqual(40);
      expect(bounds.height).toBeGreaterThanOrEqual(40);
      expect(bounds.x + bounds.width).toBeLessThanOrEqual(390);
    }
  });

  test(`new-task mode supports radio keyboard behavior without stealing focus (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto("/board");
    await page.getByRole("button", { name: "Новая задача", exact: true }).click();
    const manual = page.getByRole("radio", { name: "Заполнить самому", exact: true });
    const talk = page.getByRole("radio", { name: "Обсудить с менеджером", exact: true });
    await manual.focus();
    await page.keyboard.press("ArrowRight");
    await expect(talk).toBeChecked();
    await expect(talk).toBeFocused();
    await expect(page.getByRole("textbox", { name: "Идея как есть", exact: true })).toBeVisible();
    await expect(page.locator('.nt-mode [tabindex="0"]')).toHaveCount(1);
    await page.keyboard.press("ArrowLeft");
    await expect(manual).toBeChecked();
    await expect(manual).toBeFocused();
    await page.keyboard.press("End");
    await expect(talk).toBeChecked();
    await expect(talk).toBeFocused();
    await page.keyboard.press("Home");
    await expect(manual).toBeChecked();
    await page.keyboard.press("Tab");
    await expect(page.getByRole("button", { name: "Закрыть", exact: true })).toBeFocused();
    await talk.click();
    await page.getByRole("textbox", { name: "Идея как есть", exact: true }).fill("Проверить идею");
    await page.getByRole("button", { name: /^Начать разговор/ }).focus();
    await page.keyboard.press("Tab");
    await expect(talk).toBeFocused();
  });

  test(`team creation stays secondary while the task needs an owner answer (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    const task: Task = await (await page.request.get("/api/tasks/DM-2")).json();
    await page.route(/\/api\/tasks\/DM-2(?:\?.*)?$/, (route) => route.fulfill({ json: { ...task, team: undefined, status: "needs_owner", needsOwner: { question: "Какой канал выбрать?", by: "agent", at: new Date().toISOString(), previous: "in_progress" } } }));
    await page.route(/\/api\/teams(?:\?.*)?$/, (route) => route.fulfill({ json: [] }));
    await page.goto("/task/DM-2");
    await expect(page.getByRole("region", { name: "Нужно ваше решение", exact: true })).toBeVisible();
    const create = page.locator(".tp-team-card").getByRole("button", { name: "Собрать команду", exact: true });
    await expect(create).toBeVisible();
    await expect(create).toHaveClass(/secondary/);
    await expect(create).not.toHaveClass(/primary/);
  });
}
