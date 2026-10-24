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

  // Two dialogs can be on screen at once here (the form, then the task it just made), so the form is
  // named rather than found by being the only dialog.
  const dialog = page.getByRole("dialog", { name: "Новая задача" });
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
  // The new card opens by itself, so the walkthrough ends where the person lands rather than on the board.
  await expect(page.getByRole("dialog", { name: `Задача ${id}` })).toBeVisible();
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

test("a decision is answered from the keyboard alone", async ({ page }) => {
  // FLOW_TASKS[0] waits for its owner on the flows server only: the ids follow the order of the seed.
  const id = `DM-${SEED_TASKS.length + 1}`;
  await page.goto(`/task/${id}`);

  const box = page.getByRole("region", { name: "Нужно ваше решение" });
  await expect(box).toBeVisible();
  const options = box.getByRole("radiogroup", { name: "Варианты ответа" }).getByRole("radio");
  await expect(options).toHaveCount(3);

  // The group is one tab stop: while nothing is chosen, Tab lands on its first option.
  await tabTo(page, options.first(), "the first option of the decision", 320);
  await expect(options.first()).toHaveAttribute("aria-checked", "false");

  // Arrows move focus and the choice together, and Home/End reach the ends, as the radiogroup pattern has it.
  await page.keyboard.press("ArrowDown");
  await expect(options.nth(1)).toBeFocused();
  await expect(options.nth(1)).toHaveAttribute("aria-checked", "true");
  await expect(options.nth(0)).toHaveAttribute("aria-checked", "false");
  await page.keyboard.press("End");
  await expect(options.nth(2)).toBeFocused();
  await page.keyboard.press("Home");
  await expect(options.nth(0)).toBeFocused();
  await page.keyboard.press("ArrowRight");
  await expect(options.nth(1)).toBeFocused();
  // Only the chosen option is checked, and it is the only stop inside the group.
  await expect(box.getByRole("radio", { checked: true })).toHaveCount(1);
  await expect(options.nth(1)).toHaveAttribute("tabindex", "0");
  await expect(options.nth(0)).toHaveAttribute("tabindex", "-1");

  // Tab leaves the group for the words, then reaches the buttons in reading order.
  await page.keyboard.press("Tab");
  await expect(box.getByLabel("Ваш ответ")).toBeFocused();
  await page.keyboard.type("Так надёжнее.");
  const answer = box.getByRole("button", { name: "Ответить и вернуть в работу" });
  await tabTo(page, answer, "the answer button", 8);
  await page.keyboard.press("Enter");
  await expect(box).toBeHidden();

  // What the server stored: the picked option and the words as the answer, and the task back in work.
  const stored = await storedTask(page.request, id);
  const said = stored.comments.map((c) => c.text);
  expect(said).toContain("Выбран вариант: «Telegram»\n\nТак надёжнее.");
  expect(said.at(-1)).toContain("needs_owner → in_progress");
  expect(stored.status).toBe("in_progress");
  expect(stored.needsOwner).toBeUndefined();
  // The box took focus with it; the page hands it to the headline instead of the document.
  await expect(page.locator(".td-headline .title")).toBeFocused();
});

test("a dialog opened from a dialog owns the keyboard while it is up", async ({ page }) => {
  // The centered task modal is a native <dialog>; ⌘K opens the command palette as a second one above it.
  await page.goto("/board?task=DM-1");
  const modal = page.getByRole("dialog").first();
  await expect(modal).toBeVisible();
  for (let i = 0; i < 3; i += 1) await page.keyboard.press("Tab");
  expect(await focusInside(modal), "focus is in the task modal").toBe(true);

  await page.keyboard.press("Control+k");
  const palette = page.getByRole("dialog", { name: "Команды" });
  await expect(palette).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCount(2);
  const search = palette.getByLabel("Поиск команд, задач и документации");
  await expect(search).toBeFocused();

  // Tab belongs to the palette: the modal underneath must not pull focus back into itself.
  for (let i = 1; i <= 10; i += 1) {
    await page.keyboard.press("Tab");
    expect(await focusInside(palette), `focus stays in the palette after ${i} tabs`).toBe(true);
  }

  // Escape closes the palette only; the task modal is still up and reachable.
  await page.keyboard.press("Escape");
  await expect(palette).toBeHidden();
  await expect(page.getByRole("dialog")).toHaveCount(1);
  expect(await focusInside(modal), "focus is back inside the task modal").toBe(true);

  // A second Escape closes the task modal, and the page behind it is reachable again.
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByRole("main", { name: "Доска" })).toBeVisible();
});
