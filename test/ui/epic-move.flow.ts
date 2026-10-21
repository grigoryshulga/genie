// Moving a task into an epic from either screen: the confirmation names the task and the epic the
// server actually saved, even though both selects are controlled and render the saved state again
// while the mutation is in flight (regression GNE-72).

import { expect, test, type Page } from "@playwright/test";
import { createEpic, storedTask } from "./api.ts";

// The toast is one polite live region for the whole app, so its text is the whole confirmation.
async function confirmation(page: Page): Promise<string> {
  const toast = page.getByRole("status");
  await expect(toast).not.toHaveText("");
  return ((await toast.textContent()) ?? "").trim();
}

test("the epic page confirmation names the moved task and the epic", async ({ page, request }) => {
  await page.goto("/epic/DM-1");
  const move = page.getByLabel("Перенести задачу в эпик");
  await expect(move).toBeVisible();
  // DM-6 stands on its own in the seed, so the epic page offers it; that select drops back to ""
  // while the patch is in flight, which used to swallow the task ID in the confirmation.
  await move.selectOption("DM-6");

  expect(await confirmation(page)).toBe("DM-6 теперь в эпике DM-1");
  expect((await storedTask(request, "DM-6")).parent).toBe("DM-1");
});

test("the task page confirmation names the chosen epic after the response", async ({ page, request }) => {
  const target = await createEpic(request, "Второй эпик: смена эпика из свойств");
  // DM-2 already belongs to DM-1. Holding the PATCH open makes the asynchronous wording reproducible:
  // the controlled select shows DM-1 again while the request is in flight, so a callback that reads
  // the control instead of the chosen IDs reports the previous epic.
  await page.route("**/api/tasks/DM-2", async (route) => {
    if (route.request().method() !== "PATCH") return route.continue();
    await new Promise((done) => setTimeout(done, 750));
    return route.continue();
  });
  await page.goto("/task/DM-2");
  const epic = page.getByLabel("Эпик", { exact: true });
  await expect(epic).toHaveValue("DM-1");

  await epic.selectOption(target);

  expect(await confirmation(page)).toBe(`DM-2 теперь в эпике ${target}`);
  // The server saved the chosen epic, not the one the control showed in the meantime.
  expect((await storedTask(request, "DM-2")).parent).toBe(target);
  // The page follows the server: the saved epic is the one the properties show afterwards.
  await expect(epic).toHaveValue(target);
});

test("leaving an epic from the task page is confirmed without a target", async ({ page, request }) => {
  await page.goto("/task/DM-2");
  const epic = page.getByLabel("Эпик", { exact: true });
  await expect(epic).not.toHaveValue("");

  await epic.selectOption("");

  expect(await confirmation(page)).toBe("DM-2 больше не в эпике");
  expect((await storedTask(request, "DM-2")).parent).toBeUndefined();
});
