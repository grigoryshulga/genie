// The project chat: the feed, the composer and the @ list. Messages the person sends reach the real server
// (the flows server); the states the page shows from its data are fixtures, so they never change the server.

import { expect, type Page, type Route, test } from "@playwright/test";
import type { ChatMessage } from "../../web/src/shared/api/generated/ChatMessage.ts";
import type { ChatPage } from "../../web/src/shared/api/generated/ChatPage.ts";

const AT = "2026-05-01T12:00:00Z";
const message = (id: number, author: string, authorRole: ChatMessage["authorRole"], text: string, over: Partial<ChatMessage> = {}): ChatMessage => ({
  id,
  at: AT,
  author,
  authorRole,
  text,
  mentions: [],
  replyTo: null,
  ...over,
});

const DEMO: ChatPage = {
  messages: [
    message(1, "owner", "human", "Что сейчас в работе по проекту?", { mentions: ["manager"] }),
    message(2, "orchestrator", "orchestrator", "Сейчас в работе **три задачи**:\n\n- GNE-12 — ждёт ревью", { replyTo: 1 }),
    message(3, "owner", "human", "Создай задачу: проверить поиск по документации", { mentions: ["manager"] }),
  ],
  manager: { state: "working", pending: 0 },
};

const LOG = "Сообщения проекта";
const BOX = "Текст сообщения";

/** The page's chat data comes from `body`; a write (POST) goes on to the server unless `write` answers it. */
async function chatData(page: Page, body: ChatPage, write?: (route: Route) => Promise<void>) {
  await page.route(/\/api\/chat(\?.*)?$/, async (route) => {
    if (route.request().method() === "GET") await route.fulfill({ json: body });
    else if (write) await write(route);
    else await route.continue();
  });
}

test("the feed shows the messages, the manager's by its label and avatar, and the line while it answers", async ({ page }) => {
  await chatData(page, DEMO);
  await page.goto("/chat");
  await expect(page.getByRole("heading", { level: 1, name: "Чат" })).toBeVisible();

  const feed = page.getByRole("log", { name: LOG });
  await expect(feed.locator(".chat-msg")).toHaveCount(3);
  await expect(feed.locator(".chat-msg.manager .chat-from")).toHaveText("Менеджер проекта");
  await expect(feed.locator(".chat-msg.manager .g-avatar")).toHaveAttribute("data-tone", "project-manager");
  // The person's own message sits in the accent, on the right.
  await expect(feed.locator(".chat-msg.mine .chat-bubble.mine")).toHaveCount(2);
  await expect(feed.locator(".chat-msg.manager .chat-quote").first()).toContainText("Что сейчас в работе по проекту?");
  await expect(page.getByRole("status").filter({ hasText: "Менеджер проекта отвечает…" })).toBeVisible();
});

test("an empty chat says how to ask the manager", async ({ page }) => {
  await chatData(page, { messages: [], manager: { state: null, pending: 0 } });
  await page.goto("/chat");
  const empty = page.locator(".g-empty");
  await expect(empty.getByRole("heading", { name: "Сообщений пока нет" })).toBeVisible();
  await expect(empty).toContainText("Тэгните @pm, чтобы спросить менеджера проекта: статус задач, создать или запустить задачу.");
});

test("the typing line shows while chat mail waits for the manager and hides once it is idle", async ({ page }) => {
  let manager: ChatPage["manager"] = { state: null, pending: 1 };
  await page.route(/\/api\/chat(\?.*)?$/, (route) => route.fulfill({ json: { messages: DEMO.messages, manager } }));
  await page.goto("/chat");
  const line = page.getByRole("status").filter({ hasText: "Менеджер проекта отвечает…" });
  await expect(line).toBeVisible();
  manager = { state: "idle", pending: 0 };
  await expect(line).toBeHidden({ timeout: 10_000 });
});

test("Enter sends the message to the server, and Shift+Enter starts a new line", async ({ page, request }) => {
  await page.goto("/chat");
  const box = page.getByRole("textbox", { name: BOX });
  await box.fill("Первая строка");
  await box.press("Shift+Enter");
  await box.pressSequentially("вторая строка");
  await expect(box).toHaveValue("Первая строка\nвторая строка");
  await box.press("Enter");
  await expect(box).toHaveValue("");

  const own = page.getByRole("log", { name: LOG }).locator(".chat-bubble.mine").filter({ hasText: "вторая строка" });
  await expect(own).toContainText("Первая строка");
  const stored = await request.get("/api/chat?limit=50");
  expect(stored.ok()).toBe(true);
  const body: ChatPage = await stored.json();
  expect(body.messages.map((m) => m.text)).toContain("Первая строка\nвторая строка");
});

test("@ offers the manager and the members; arrows, Enter and Tab pick, Escape closes the list", async ({ page }) => {
  await page.goto("/chat");
  const box = page.getByRole("textbox", { name: BOX });
  const list = page.getByRole("listbox", { name: "Кого упомянуть" });

  await box.pressSequentially("Спроси @");
  await expect(list.getByRole("option")).toHaveCount(2);
  await expect(list.getByRole("option", { selected: true })).toHaveText("@pm — менеджер проекта");
  await expect(box).toHaveAttribute("aria-activedescendant", await list.getByRole("option").first().getAttribute("id") ?? "");

  await box.press("ArrowDown");
  await expect(list.getByRole("option", { selected: true })).toHaveText("@owner — Анна Смирнова");
  await box.press("ArrowDown");
  await expect(list.getByRole("option", { selected: true })).toHaveText("@pm — менеджер проекта");

  await box.press("Escape");
  await expect(list).toBeHidden();
  await expect(box).toHaveValue("Спроси @");

  // Typing reopens the list; Tab picks the highlighted entry and keeps the focus in the field.
  await box.pressSequentially("p");
  await expect(list.getByRole("option")).toHaveCount(1);
  await box.press("Tab");
  await expect(box).toHaveValue("Спроси @pm ");
  await expect(list).toBeHidden();
  await expect(box).toBeFocused();

  // Enter picks as well, while the list is open; it does not send.
  await box.fill("");
  await box.pressSequentially("@o");
  await box.press("Enter");
  await expect(box).toHaveValue("@owner ");
});

test("replying quotes the message above the composer, and the cancel button drops the quote", async ({ page }) => {
  await chatData(page, DEMO);
  await page.goto("/chat");
  const quote = page.locator(".chat-reply-to");
  await expect(quote).toBeHidden();

  await page.getByRole("button", { name: /Ответить на сообщение: Что сейчас в работе/ }).click();
  await expect(quote).toContainText("В ответ");
  await expect(quote).toContainText("Что сейчас в работе по проекту?");
  await expect(page.getByRole("textbox", { name: BOX })).toBeFocused();

  await page.getByRole("button", { name: "Отменить ответ" }).click();
  await expect(quote).toBeHidden();
});

test("a reply is sent with the id of the message it answers", async ({ page }) => {
  await chatData(page, DEMO, (route) =>
    route.fulfill({
      status: 201,
      json: message(4, "owner", "human", "Уточню в ответ", { replyTo: 1 }),
    }),
  );
  await page.goto("/chat");
  await page.getByRole("button", { name: /Ответить на сообщение: Что сейчас в работе/ }).click();
  const box = page.getByRole("textbox", { name: BOX });
  await box.fill("Уточню в ответ");

  const posted = page.waitForRequest((req) => req.method() === "POST" && new URL(req.url()).pathname === "/api/chat");
  await box.press("Enter");
  expect((await posted).postDataJSON()).toEqual({ text: "Уточню в ответ", replyTo: 1 });
  await expect(box).toHaveValue("");
  await expect(page.locator(".chat-reply-to")).toBeHidden();
});

test("Чат is a top-bar tab of its own and opens the chat", async ({ page }) => {
  await page.goto("/tasks");
  await page.getByRole("button", { name: "Виды задач" }).click();
  await expect(page.getByRole("menu", { name: "Виды задач" }).getByRole("menuitem", { name: "Чат" })).toHaveCount(0);
  await page.keyboard.press("Escape");
  const tab = page.getByRole("navigation", { name: "Навигация", exact: true }).getByRole("link", { name: "Чат", exact: true });
  await expect(tab).toHaveAttribute("href", "/chat");
  await tab.click();
  await expect(page).toHaveURL(/\/chat$/);
  await expect(tab).toHaveAttribute("aria-current", "page");
  await expect(page.getByRole("heading", { level: 1, name: "Чат" })).toBeVisible();
});
