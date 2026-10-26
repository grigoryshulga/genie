// The shell's popups and where the documentation proposals count lives.
//
// The seeded server has no notifications of its own (they come from agents' work) and no section
// that asks for review, so the notification and proposals APIs are answered from the labelled
// fixtures below through `page.route` — fixtures, not live data. Everything else is the real
// server: the shell, its routes, and opening a task from a notification.

import { expect, type Locator, type Page, test } from "@playwright/test";
import { PROJECT, SEED_TASKS } from "./seed.ts";

/** A notification as the fixture states it (the server's own shape). */
interface FixtureNotification {
  id: number;
  project: string;
  task?: string;
  kind: string;
  title: string;
  body: string;
  link?: string;
  created: string;
  readAt?: string;
}

const minutesAgo = (minutes: number) => new Date(Date.now() - minutes * 60_000).toISOString();

/** Two unread notifications and one that was read: the popup's list, its count and its dot. */
const NOTIFICATIONS: FixtureNotification[] = [
  {
    id: 3,
    project: PROJECT.id,
    task: "DM-2",
    kind: "task.comment",
    title: "DM-2 · новый комментарий агента",
    body: "Проверил форму входа: сообщение об ошибке появляется под формой.",
    link: "/task/DM-2",
    created: minutesAgo(4),
  },
  {
    id: 2,
    project: PROJECT.id,
    task: "DM-1",
    kind: "task.status",
    title: "DM-1 · статус: needs_owner",
    body: "Агент ждёт решения владельца.",
    link: "/task/DM-1",
    created: minutesAgo(90),
  },
  {
    id: 1,
    project: PROJECT.id,
    kind: "proposal",
    title: "Предложение к документации",
    body: "Страница про деплой.",
    created: minutesAgo(1_500),
    readAt: minutesAgo(1_400),
  },
];

/** Pending proposals: what the Documentation entry counts. */
const PROPOSALS = [
  { id: 2, path: "demo/guides/deploy.md", author: "agent", authorKind: "agent", note: "Новая страница про деплой", status: "open", created: minutesAgo(60) },
  { id: 1, path: "demo/decisions/storage.md", author: "agent", authorKind: "agent", note: "Правка решения", status: "open", created: minutesAgo(120) },
];

/** Answer the notification APIs from the fixture and record the reads the UI sends. */
async function serveNotifications(page: Page, items: FixtureNotification[]) {
  const state = items.map((n) => ({ ...n }));
  const reads: (number | null)[] = [];
  await page.route(/\/api\/notifications(\?.*)?$/, async (route) => {
    await route.fulfill({ json: { items: state, unread: state.filter((n) => !n.readAt).length } });
  });
  await page.route(/\/api\/notifications\/read$/, async (route) => {
    const body = (route.request().postDataJSON() ?? {}) as { id?: number };
    reads.push(body.id ?? null);
    for (const n of state) if (body.id === undefined || n.id === body.id) n.readAt = minutesAgo(0);
    await route.fulfill({ json: { ok: true } });
  });
  return { state, reads };
}

const popupOf = (page: Page) => page.getByRole("dialog", { name: "Уведомления" });

for (const theme of ["ink", "paper"]) {
  for (const width of [1440, 390]) {
    test(`the bell opens the notification popup in place (${theme}, ${width})`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await serveNotifications(page, NOTIFICATIONS);
      await page.goto("/board");
      await page.evaluate((value) => void (document.documentElement.dataset.theme = value), theme);
      const bell = page.getByRole("button", { name: "Уведомления, непрочитанных: 2" });
      await bell.click();
      const popup = popupOf(page);
      await expect(popup).toBeVisible();
      await expect(bell).toHaveAttribute("aria-expanded", "true");
      // The popup is not a route: the board stays where the person left it.
      expect(new URL(page.url()).pathname).toBe("/board");
      await expect(popup.getByRole("button", { name: /DM-2 · новый комментарий агента/ })).toBeVisible();
      await expect(popup.getByRole("button", { name: /DM-1 · статус: needs_owner/ })).toBeVisible();
      await expect(popup.getByRole("button", { name: "Прочитать все" })).toBeVisible();
      // It opens inside the window it was opened in, on a desktop and under a thumb alike.
      const box = (await popup.boundingBox())!;
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.y).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(width);
      expect(box.y + box.height).toBeLessThanOrEqual(900);
      await page.screenshot({ path: `test-results/shell-popup-${theme}-${width}.png`, animations: "disabled" });
      await page.keyboard.press("Escape");
      await expect(popup).toBeHidden();
      await expect(bell).toBeFocused();
      await expect(bell).toHaveAttribute("aria-expanded", "false");
    });
  }
}


test("an outside click closes the popup and leaves focus where the click went", async ({ page }) => {
  await serveNotifications(page, NOTIFICATIONS);
  await page.goto("/board");
  const bell = page.getByRole("button", { name: "Уведомления, непрочитанных: 2" });
  await bell.click();
  await expect(popupOf(page)).toBeVisible();
  // The empty stretch of the top bar between the sections and the search: an outside click.
  await page.getByRole("heading", { name: "Доска", exact: true }).click();
  await expect(popupOf(page)).toBeHidden();
  await expect(bell).toHaveAttribute("aria-expanded", "false");
  // The click, not the closing popup, decides where focus lands.
  expect(await page.evaluate(() => document.activeElement?.closest("#notifications-popup") ?? null)).toBeNull();
  // Escape is the way back to the trigger.
  await bell.click();
  await expect(popupOf(page)).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(bell).toBeFocused();
});

test("a notification opens its task and the notification becomes read", async ({ page }) => {
  const { reads } = await serveNotifications(page, NOTIFICATIONS);
  await page.goto("/board");
  await page.getByRole("button", { name: "Уведомления, непрочитанных: 2" }).click();
  await popupOf(page).getByRole("button", { name: /DM-2 · новый комментарий агента/ }).click();
  await expect(page).toHaveURL(/\/task\/DM-2$/);
  await expect(popupOf(page)).toHaveCount(0);
  expect(reads).toEqual([3]);
  await expect(page.getByRole("heading", { level: 1, name: SEED_TASKS[1].title })).toBeVisible();
});

test("a notification without a link is only marked read", async ({ page }) => {
  const { reads } = await serveNotifications(page, [{ ...NOTIFICATIONS[2], readAt: undefined }]);
  await page.goto("/board");
  await page.getByRole("button", { name: "Уведомления, непрочитанных: 1" }).click();
  await popupOf(page).getByRole("button", { name: /Предложение к документации/ }).click();
  expect(reads).toEqual([1]);
  await expect(popupOf(page)).toBeVisible();
  expect(new URL(page.url()).pathname).toBe("/board");
});

test("marking everything read clears the count and the trigger's label", async ({ page }) => {
  const { reads } = await serveNotifications(page, NOTIFICATIONS);
  await page.goto("/board");
  // The bell by its place, not by its label: the label is what this test watches change.
  const bell = page.locator(".g-bell");
  await expect(bell).toHaveAttribute("aria-label", "Уведомления, непрочитанных: 2");
  await bell.click();
  await popupOf(page).getByRole("button", { name: "Прочитать все" }).click();
  expect(reads).toEqual([null]);
  await expect(bell).toHaveAttribute("aria-label", "Уведомления");
  await expect(popupOf(page).getByRole("button", { name: "Прочитать все" })).toHaveCount(0);
  await expect(popupOf(page).getByText(/непрочитанных/)).toHaveCount(0);
});

test("the popup says when there is nothing to show", async ({ page }) => {
  await serveNotifications(page, []);
  await page.goto("/board");
  await page.getByRole("button", { name: "Уведомления", exact: true }).click();
  await expect(popupOf(page).getByRole("heading", { name: "Уведомлений нет" })).toBeVisible();
});

test("the popup shows the server's error and retries", async ({ page }) => {
  let failing = true;
  await page.route(/\/api\/notifications(\?.*)?$/, async (route) => {
    if (failing) return route.fulfill({ status: 500, json: { error: "Сервер недоступен" } });
    return route.fulfill({ json: { items: NOTIFICATIONS, unread: 2 } });
  });
  await page.goto("/board");
  await page.getByRole("button", { name: "Уведомления", exact: true }).click();
  await expect(popupOf(page).getByText("Сервер недоступен")).toBeVisible();
  failing = false;
  await popupOf(page).getByRole("button", { name: "Повторить" }).click();
  await expect(popupOf(page).getByRole("button", { name: /DM-2 · новый комментарий агента/ })).toBeVisible();
});

test("the notifications page stays its own route", async ({ page }) => {
  await page.route(/\/api\/notifications(\?.*)?$/, async (route) => await route.fulfill({ json: { items: NOTIFICATIONS, unread: 2 } }));
  await page.goto("/notifications");
  await expect(page.getByRole("heading", { level: 1, name: "Уведомления" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Уведомления, непрочитанных: 2" })).toHaveAttribute("aria-expanded", "false");
});

for (const width of [1440, 390]) {
  test(`pending proposals ride on the Knowledge entry (${width})`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.route(/\/api\/docs\/proposals\?status=open$/, async (route) => await route.fulfill({ json: PROPOSALS }));
    await page.goto("/board");

    const docs = page.getByRole("navigation", { name: "Навигация" }).getByRole("link", { name: /Знания/ });
    await expect(docs).toBeVisible();
    await expect(docs).toHaveAttribute("href", "/docs");
    await expect(docs).toHaveAccessibleName(/Знания\s+предложений к документации: 2/);
    // A count inside a count's entry, not a second link inside a link.
    await expect(docs.locator("a")).toHaveCount(0);

    await expect(page.getByRole("button", { name: "Ещё", exact: true })).toHaveCount(0);
    await page.getByRole("button", { name: /^Профиль:/ }).click();
    const profile = page.getByRole("menu", { name: "Профиль", exact: true });
    await expect(profile.getByRole("menuitem", { name: "Быстрые команды" })).toBeVisible();
    await expect(profile.getByRole("menuitem", { name: /Автоматизации|Наблюдения/ })).toHaveCount(0);

  });
}

test("the proposals page keeps its own address and a way in from the documentation", async ({ page }) => {
  await page.goto("/docs/proposals");
  await expect(page.getByRole("heading", { level: 1, name: "Предложения" })).toBeVisible();
  await page.getByRole("link", { name: "Страницы" }).click();
  await expect(page).toHaveURL(/\/docs$/);
});

test("the current tab's count reads on the accent pill", async ({ page }) => {
  await page.goto("/tasks");
  const tab = page.getByRole("navigation", { name: "Навигация" }).locator('a[aria-current="page"]');
  await expect(tab).toContainText("Задачи");
  const count = tab.locator(".g-topbar-open");
  await expect(count).toBeVisible();
  // The number takes the pill's own foreground: the muted caption it wears elsewhere would not read.
  const colors = await tab.evaluate((node) => {
    const number = node.querySelector(".g-topbar-open")!;
    return { number: getComputedStyle(number).color, pill: getComputedStyle(node).color };
  });
  expect(colors.number).toBe(colors.pill);
  await page.screenshot({ path: "test-results/shell-topbar-tab-count.png", clip: { x: 0, y: 0, width: 1000, height: 72 }, animations: "disabled" });
});

test("the project switcher lists names on one line without role hints", async ({ page }) => {
  await page.goto("/board");
  await page.getByRole("button", { name: "Проект Демо" }).click();
  const menu = page.getByRole("menu", { name: "Проекты" });
  await expect(menu.getByRole("menuitemcheckbox", { name: "Демо", exact: true })).toBeVisible();
  await expect(menu.locator(".hint")).toHaveCount(0);
  const name = await menu.locator(".g-topbar-project-name").first().evaluate((node) => {
    const style = getComputedStyle(node);
    return { whiteSpace: style.whiteSpace, overflow: style.overflow, textOverflow: style.textOverflow };
  });
  expect(name).toEqual({ whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" });
  await page.screenshot({ path: "test-results/shell-project-menu.png", clip: { x: 0, y: 0, width: 460, height: 220 }, animations: "disabled" });
});

for (const theme of ["ink", "paper"]) {
  for (const width of [1440, 390]) {
    test(`shell menus stay above sticky task-list groups (${theme}, ${width})`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
      await serveNotifications(page, NOTIFICATIONS);
      await page.goto("/tasks");
      await expect(page.locator(".tl-group-head").first()).toBeVisible();
      await page.getByRole("button", { name: /^Профиль:/ }).click();
      const profile = page.getByRole("menu", { name: "Профиль", exact: true });
      await expect(profile).toBeVisible();
      const exposed = (popup: Locator) => popup.evaluate((menu) => [...menu.querySelectorAll<HTMLElement>("button:not(:disabled)")].every((row) => {
        const rect = row.getBoundingClientRect();
        const hit = document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2);
        return !!hit && row.contains(hit);
      }));
      await expect.poll(() => exposed(profile)).toBe(true);
      await page.screenshot({ path: `test-results/shell-menu-layer-${theme}-${width}.png`, animations: "disabled" });
      await profile.getByRole("menuitem", { name: "Быстрые команды" }).click();
      await expect(page.getByRole("dialog", { name: "Команды", exact: true })).toBeVisible();
      await page.keyboard.press("Escape");
      await page.getByRole("button", { name: /^Проект / }).click();
      const projects = page.getByRole("menu", { name: "Проекты", exact: true });
      await expect(projects).toBeVisible();
      await expect(projects.getByRole("menuitemcheckbox").first()).toBeInViewport();
      await expect.poll(() => exposed(projects)).toBe(true);
      await page.keyboard.press("Escape");
      await page.getByRole("button", { name: /^Уведомления/ }).click();
      const notifications = popupOf(page);
      await expect(notifications).toBeVisible();
      await expect(notifications.getByRole("button", { name: "Прочитать все" })).toBeVisible();
      await expect.poll(() => exposed(notifications)).toBe(true);
    });
  }
}

for (const theme of ["ink", "paper"]) {
  test(`phone dock stays above task groups while scrolling (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto("/tasks");
    await expect(page.locator(".tl-group-head").first()).toBeVisible();
    // Move each real sticky group through the dock's center, rather than assuming a fixed list length.
    for (const group of await page.locator(".tl-group-head").all()) {
      await group.evaluate((node) => {
        const dock = document.querySelector(".g-topbar-nav")!.getBoundingClientRect();
        const head = node.getBoundingClientRect();
        window.scrollBy(0, head.top - (dock.top + (dock.height - head.height) / 2));
      });
      const exposed = await page.locator(".g-topbar-nav").evaluate((dock) => [...dock.querySelectorAll("a"), document.querySelector(".g-topbar-create")!].every((link) => {
        const box = link.getBoundingClientRect();
        return link.contains(document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2));
      }));
      expect(exposed).toBe(true);
    }
    await page.screenshot({ path: `test-results/dock-over-list-${theme}.png`, animations: "disabled" });
  });
}
