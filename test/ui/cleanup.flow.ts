import { expect, type Page, test } from "@playwright/test";
import type { TaskSummary } from "../../web/src/shared/api/types.ts";
import { focusInside } from "./keyboard.ts";

// Fail an established stream without relying on Chromium to tear down SSE on network emulation.
async function watchLiveStream(page: Page) {
  await page.addInitScript(() => {
    const Original = window.EventSource;
    const streams: EventSource[] = [];
    window.EventSource = class extends Original {
      constructor(url: string | URL, options?: EventSourceInit) {
        super(url, options);
        streams.push(this);
      }
    };
    window.addEventListener("cleanup-reconnect", () => {
      for (const stream of streams) stream.dispatchEvent(new Event("open"));
    });
    window.addEventListener("cleanup-disconnect", () => {
      for (const stream of streams) {
        stream.close();
        stream.dispatchEvent(new Event("error"));
      }
    });
  });
}

async function disconnect(page: Page) {
  await page.evaluate(() => window.dispatchEvent(new Event("cleanup-disconnect")));
  await expect(page.getByRole("main", { name: "Доска" }).getByText("Нет связи с сервером", { exact: true })).toBeVisible();
}

for (const theme of ["ink", "paper"]) {
  for (const width of [1440, 390]) {
    test(`artifact lightbox stays interactive above its artifact dialog (${theme}, ${width})`, async ({ page, request }) => {
      await page.setViewportSize({ width, height: 844 });
      await page.goto("/tasks");
      await expect(page.getByRole("toolbar", { name: "Инструменты страницы" })).toBeVisible();
    const image = await page.screenshot({ clip: { x: 0, y: 0, width: 320, height: 240 } });
      const headers = { "x-genie": "1" };
      const created = await request.post("/api/tasks?project=demo", { headers, data: { title: "Image artifact regression" } });
      expect(created.ok()).toBe(true);
      const task: { id: string } = await created.json();
      const added = await request.post(`/api/tasks/${task.id}/artifacts?project=demo`, {
        headers, data: { name: "audit.png", contentBase64: image.toString("base64") },
      });
      expect(added.ok()).toBe(true);
      await page.goto(`/task/${task.id}`);
      await page.evaluate((value) => { document.documentElement.dataset.theme = value; }, theme);
      const taskPage = page.getByRole("main", { name: `Задача ${task.id}`, exact: true });
      // Attachments live in their own tab; the description tab is open by default.
      await taskPage.getByRole("tab", { name: /Вложения/ }).click();
      await taskPage.locator("button.artifact").filter({ hasText: "audit.png" }).click();
      const artifact = page.getByRole("dialog", { name: "audit.png", exact: true });
      const opener = artifact.getByRole("button", { name: "audit.png", exact: true });
      await opener.click();
      const lightbox = page.locator(".lightbox");
      const zoom = lightbox.getByRole("button", { name: "Увеличить", exact: true });
      await expect(zoom).toBeVisible();
      expect(await zoom.evaluate((node) => {
        const box = node.getBoundingClientRect();
        return node.contains(document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2));
      })).toBe(true);
      await zoom.click();
      await expect(lightbox.getByRole("button", { name: "Сбросить масштаб" })).toHaveText("120%");
      await lightbox.getByRole("button", { name: "Закрыть", exact: true }).focus();
      await page.keyboard.press("Tab");
      expect(await focusInside(lightbox)).toBe(true);
      await page.screenshot({ path: `test-results/cleanup-lightbox-${theme}-${width}.png` });
      await page.keyboard.press("Escape");
      await expect(lightbox).toHaveCount(0);
      await expect(artifact).toBeVisible();
      await expect(opener).toBeFocused();
      await page.keyboard.press("Escape");
      await expect(artifact).toHaveCount(0);
      await expect(taskPage).toBeVisible();
    });

  }

  test(`phone participants sheet traps and restores keyboard focus (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    const response = await page.request.get("/api/tasks?closed=1");
    const tasks: TaskSummary[] = await response.json();
    const task = tasks[0];
    const at = new Date().toISOString();
    const team = {
      id: "cleanup-team", task: task.id, cwd: "/tmp", state: "active", created: at, updated: at,
      members: [{ name: "Бендер", role: "executor", state: "active", activity: "idle", status: "", statusAt: at }],
      taskInfo: { id: task.id, title: task.title, status: "in_progress" }, pending: {}, mail: [], log: [],
    };
    await page.route("**/api/tasks?*", (route) => route.fulfill({ json: tasks.map((t) => t.id === task.id ? { ...t, team: team.id } : t) }));
    await page.route("**/api/teams?*", (route) => route.fulfill({ json: [team] }));
    await page.route(`**/api/teams/${team.id}`, (route) => route.fulfill({ json: team }));
    await page.goto(`/task/${task.id}/team`);
    await page.evaluate((value) => { document.documentElement.dataset.theme = value; }, theme);
    const opener = page.getByRole("button", { name: "Участники · 1", exact: true });
    await opener.click();
    const panel = page.locator(".tv-aside").filter({ visible: true });
    await expect(panel).toBeVisible();
    expect(await focusInside(panel)).toBe(true);
    const dialog = page.getByRole("dialog", { name: "Участники и журнал", exact: true });
    await expect(dialog).toBeVisible();
    const close = panel.getByRole("button", { name: "Закрыть", exact: true });
    await close.focus();
    await page.keyboard.press("Shift+Tab");
    expect(await focusInside(dialog)).toBe(true);
    await page.screenshot({ path: `test-results/cleanup-participants-${theme}.png` });
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    await expect(opener).toBeFocused();
    await opener.click();
    await page.setViewportSize({ width: 1440, height: 900 });
    await expect(dialog).toHaveCount(0);
    await expect(page.locator(".tv-aside")).toBeVisible();
    await page.setViewportSize({ width: 390, height: 844 });
    await opener.click();
    await dialog.getByRole("button", { name: "Закрыть", exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await expect(opener).toBeFocused();
  });
}

test("offline board cannot start a keyboard drag", async ({ page }) => {
  await watchLiveStream(page);
  await page.goto("/board");
  const card = page.locator('[data-lane="prep"] .bd-main').first();
  await expect(card).toBeVisible();
  await disconnect(page);
  await card.focus();
  await page.keyboard.press("Space", { delay: 30 });
  await expect(page.locator(".bd-ghost")).toHaveCount(0);
});

test("disconnecting during a drag prevents a move even after reconnect", async ({ page }) => {
  await watchLiveStream(page);
  await page.goto("/board");
  const card = page.locator('[data-lane="prep"] .bd-main').first();
  await card.focus();
  await page.keyboard.press("Space", { delay: 30 });
  await expect(page.locator(".bd-ghost")).toBeVisible();
  const writes: string[] = [];
  page.on("request", (request) => {
    if (request.method() === "POST" && request.url().includes("/status")) writes.push(request.url());
  });
  await disconnect(page);
  await expect(page.locator(".bd-ghost")).toHaveCount(0);
  await page.evaluate(() => window.dispatchEvent(new Event("cleanup-reconnect")));
  await expect(page.getByRole("main", { name: "Доска" }).getByText("Нет связи с сервером", { exact: true })).toBeHidden();
  // Releasing the cancelled gesture cannot move anything, and the card is no longer dimmed.
  await expect(card.locator("..")).not.toHaveClass(/dragging/);
  expect(writes).toEqual([]);
  // One Space starts a new drag; it must not be consumed by the cancelled sensor.
  await card.focus();
  await page.keyboard.press("Space", { delay: 30 });
  await expect(page.locator(".bd-ghost")).toBeVisible();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Escape");
  await expect(page.locator(".bd-ghost")).toHaveCount(0);
  expect(writes).toEqual([]);
});

test("an owner move opened online cannot be submitted after disconnect", async ({ page }) => {
  await watchLiveStream(page);
  await page.goto("/board");
  const opener = page.locator('[data-lane="prep"]').getByRole("button", { name: /^Переместить/ }).first();
  await opener.click();
  await page.getByRole("menuitem", { name: /Нужно решение/ }).click();
  const dialog = page.getByRole("dialog", { name: "Вопрос к владельцу", exact: true });
  await dialog.getByRole("textbox").fill("Which release should we ship?");
  const submit = dialog.getByRole("button", { name: "Перенести", exact: true });
  await expect(submit).toBeEnabled();
  await disconnect(page);
  await expect(submit).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
});

test("disconnect cancels a pointer gesture before its activation threshold", async ({ page }) => {
  await watchLiveStream(page);
  await page.goto("/board");
  const card = page.locator('[data-lane="prep"] .bd-main').first();
  const box = (await card.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await disconnect(page);
  await page.mouse.move(box.x + box.width / 2 + 30, box.y + box.height / 2, { steps: 3 });
  await expect(page.locator(".bd-card.dragging")).toHaveCount(0);
  await page.mouse.up();
});

test("disconnect cancels a touch gesture before its activation delay", async ({ page }) => {
  await watchLiveStream(page);
  await page.goto("/board");
  const card = page.locator('[data-lane="prep"] .bd-main').first();
  await card.evaluate((node) => {
    const box = node.getBoundingClientRect();
    const touch = new Touch({ identifier: 1, target: node, clientX: box.x + box.width / 2, clientY: box.y + box.height / 2 });
    node.dispatchEvent(new TouchEvent("touchstart", { touches: [touch], changedTouches: [touch], bubbles: true }));
  });
  await disconnect(page);
  // Observe past the TouchSensor's 250ms activation timer: a pending sensor must never wake offline.
  await page.waitForTimeout(300);
  await expect(page.locator(".bd-card.dragging")).toHaveCount(0);
  await card.dispatchEvent("touchend", { touches: [], changedTouches: [] });
});

test("disconnect cancels a drag without closing an unrelated command palette", async ({ page }) => {
  await watchLiveStream(page);
  await page.goto("/board");
  const card = page.locator('[data-lane="prep"] .bd-main').first();
  const box = (await card.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 30, box.y + box.height / 2, { steps: 3 });
  await expect(page.locator(".bd-ghost")).toBeVisible();
  await page.keyboard.press("Control+k");
  const palette = page.getByRole("dialog", { name: "Команды", exact: true });
  await expect(palette).toBeVisible();
  await disconnect(page);
  await expect(palette).toBeVisible();
  await expect(page.locator(".bd-card.dragging")).toHaveCount(0);
  await page.mouse.up();
  await page.keyboard.press("Escape");
  await expect(palette).toHaveCount(0);
});
