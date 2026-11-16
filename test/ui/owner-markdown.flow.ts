import { expect, test } from "@playwright/test";
import { storedTask } from "./api.ts";

for (const theme of ["ink", "paper"]) for (const width of [1440, 390]) {
  test(`owner Markdown preserves draft and reply-only (${theme}, ${width})`, async ({ page, request }) => {
    const headers = { "x-genie": "1" };
    const created = await request.post("/api/tasks?project=demo", { headers, data: { title: "Markdown decision" } });
    expect(created.ok()).toBe(true);
    const { id } = await created.json();
    const moved = await request.post(`/api/tasks/${id}/status?project=demo`, { headers, data: {
      status: "needs_owner", note: "Choose **storage**?", context: "## Evidence\n\n- Keep **history**\n\n<script>alert(1)</script>",
    } });
    expect(moved.ok()).toBe(true);
    await page.setViewportSize({ width, height: 900 });
    await page.goto(`/tasks?task=${id}`);
    await page.evaluate((value) => { document.documentElement.dataset.theme = value; }, theme);
    const box = page.getByRole("region", { name: "Нужно ваше решение" });
    await expect(box.locator(".od-question b")).toHaveText("storage");
    await expect(box.getByRole("heading", { name: "Evidence" })).toBeHidden();
    await box.locator("summary").click();
    await expect(box.getByRole("heading", { name: "Evidence" })).toBeVisible();
    await expect(box.locator("script")).toHaveCount(0);
    const editor = box.getByRole("textbox", { name: "Ваш ответ" });
    await editor.pressSequentially("**Keep history** ");
    await box.getByRole("button", { name: "Развернуть на всю область" }).click();
    await expect(box.locator(".mbx")).toHaveClass(/wide/);
    await page.keyboard.press("Escape");
    await expect(box.locator(".mbx")).not.toHaveClass(/wide/);
    await expect(editor).toHaveText("Keep history");
    await page.screenshot({ path: `test-results/owner-markdown-${theme}-${width}.png`, animations: "disabled" });
    await box.getByRole("button", { name: "Только ответить", exact: true }).click();
    await expect(editor).toHaveText("");
    const stored = await storedTask(request, id);
    expect(stored.status).toBe("needs_owner");
    expect(stored.comments.some((c) => c.text === "**Keep history**")).toBe(true);
  });
}
