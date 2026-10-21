// The keyboard-only walkthrough the redesign promises: every job can be finished without a pointer.
// These tests never click — they move with Tab and the arrow keys, press Enter or Space, and check the
// server afterwards, so a div that only answers a mouse fails here.

import { expect, test } from "@playwright/test";
import { storedTask } from "./api.ts";
import { focusInside, tabTo } from "./keyboard.ts";
import { SEED_TASKS } from "./seed.ts";

test("a task is created from the keyboard alone", async ({ page }) => {
  await page.goto("/tasks");
  const create = page.getByRole("button", { name: /Новая задача/ });
  await tabTo(page, create, "the new-task button");
  await page.keyboard.press("Enter");

  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();

  await tabTo(page, dialog.getByLabel("Название"), "the title field");
  await page.keyboard.type("Задача с клавиатуры");
  await tabTo(page, dialog.getByLabel("Описание"), "the description field");
  await page.keyboard.type("Создана без мыши.");
  await tabTo(page, dialog.getByLabel(/Критерии приёмки/), "the acceptance field");
  await page.keyboard.type("Форма отправляется с клавиатуры");

  const submit = dialog.getByRole("button", { name: /Создать/ });
  await tabTo(page, submit, "the create button");
  await page.keyboard.press("Enter");
  await expect(dialog).toBeHidden();

  const toast = page.getByRole("status").filter({ hasText: "во входящих" });
  await expect(toast).toBeVisible();
  const id = (await toast.textContent())?.match(/DM-\d+/)?.[0];
  expect(id, "the toast names the new task").toBeTruthy();
  const stored = await storedTask(page.request, id!);
  expect(stored).toMatchObject({ title: "Задача с клавиатуры", acceptance: [{ text: "Форма отправляется с клавиатуры", done: false }] });
});

test("a dialog keeps the keyboard inside and gives it back", async ({ page }) => {
  await page.goto("/tasks");
  const create = page.getByRole("button", { name: /Новая задача/ });
  await tabTo(page, create, "the new-task button");
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();

  // Tab may visit every control in the dialog, but never the page behind it.
  for (let i = 1; i <= 24; i += 1) {
    await page.keyboard.press("Tab");
    expect(await focusInside(dialog), `focus stays inside the dialog after ${i} tabs`).toBe(true);
  }
  // Shift+Tab walks back the same way.
  for (let i = 1; i <= 8; i += 1) {
    await page.keyboard.press("Shift+Tab");
    expect(await focusInside(dialog), `Shift+Tab stays inside after ${i} presses`).toBe(true);
  }

  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  // The opener gets the focus back, so the next Tab continues from where the person was.
  expect(await create.evaluate((el) => el === document.activeElement), "focus returns to the button that opened the dialog").toBe(true);
});

test("a card moves between columns with the keyboard", async ({ page }) => {
  await page.goto("/board");
  await expect(page.getByRole("main", { name: "Доска" })).toBeVisible();

  // SEED_TASKS[1] is DM-2: the ids follow the order of the seed.
  const move = page.getByRole("button", { name: "Переместить DM-2" });
  await tabTo(page, move, "the move button of a card", 240);
  await page.keyboard.press("Enter");
  await expect(move).toHaveAttribute("aria-expanded", "true");

  // The menu opens with its first item focused; arrows walk it, Enter picks.
  const menu = page.getByRole("menu");
  await expect(menu).toBeVisible();
  const wanted = page.getByRole("menuitem", { name: "На ревью" });
  for (let i = 0; i < 12; i += 1) {
    const focused = await wanted.evaluate((el) => el === document.activeElement);
    if (focused) break;
    await page.keyboard.press("ArrowDown");
  }
  await page.keyboard.press("Enter");
  await expect(menu).toBeHidden();
  await expect(move).toHaveAttribute("aria-expanded", "false");

  // The move reached the server, and the panel that opens next is reachable from the card.
  const stored = await storedTask(page.request, "DM-2");
  expect(stored.status).toBe("review");
});

test("a document is opened from the tree with the keyboard", async ({ page }) => {
  await page.goto("/docs");
  const row = page.getByRole("button", { name: /Как запустить сервер/ }).first();
  await tabTo(page, row, "the document in the tree", 240);
  await page.keyboard.press("Enter");
  await expect(page.getByText("Сервер поднимается одной командой", { exact: false })).toBeVisible();
  // Reading is moving through the page: the reader's first control is still reachable by Tab.
  await page.keyboard.press("Tab");
  expect(await page.evaluate(() => document.activeElement !== document.body), "Tab lands on a control of the reader").toBe(true);
});
