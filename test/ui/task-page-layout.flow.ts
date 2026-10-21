import { expect, test } from "@playwright/test";
import type { Task, TaskSummary } from "../../web/src/shared/api/types.ts";

for (const theme of ["ink", "paper"]) {
  for (const width of [1920, 1440, 1024, 390]) {
    test(`task reading column and detailed roster (${theme}, ${width})`, async ({ page }) => {
      const at = new Date(Date.now() - 4 * 60_000).toISOString();
      const response = await page.request.get("/api/tasks?closed=1");
      const tasks: TaskSummary[] = await response.json();
      const detail: Task = await (await page.request.get("/api/tasks/DM-2")).json();
      const stub = {
        id: "roster-team", task: "DM-2", state: "active", cwd: "/tmp", created: at, updated: at,
        members: [
          { name: "scully", role: "analyst", state: "active", activity: "idle", status: "", statusAt: at },
          { name: "robocop", role: "executor", state: "active", activity: "idle", status: "", statusAt: at },
          { name: "picard", role: "reviewer", state: "active", activity: "idle", status: "", statusAt: at },
        ],
        pending: { scully: 1, robocop: 1, picard: 1 },
        spec: { stage: "delivery", workspace: "scratch", mail: "flow", members: [
          { key: "analyst", name: "scully", role: "analyst" },
          { key: "executor", name: "robocop", role: "executor" },
          { key: "reviewer", name: "picard", role: "reviewer" },
        ], relations: [{ from: "executor", to: ["reviewer"], type: "handoff", on: "review" }] },
        mail: [], log: [],
      };
      // Earlier move flows deliberately remove DM-2 from its epic. Own the complete fixture here,
      // including detail and summary, so the breadcrumb and dependency checks ignore that history.
      await page.route("**/api/tasks**", (route) => {
        const path = new URL(route.request().url()).pathname;
        if (path === "/api/tasks") return route.fulfill({ json: tasks.map((task) => task.id === "DM-2" ? { ...task, parent: "DM-1", status: "in_progress", team: stub.id } : task) });
        if (path === "/api/tasks/DM-2") return route.fulfill({ json: { ...detail, parent: "DM-1", status: "in_progress" } });
        return route.fallback();
      });
      await page.route("**/api/teams**", (route) => route.fulfill({ json: new URL(route.request().url()).pathname === "/api/teams" ? [stub] : stub }));
      await page.setViewportSize({ width, height: 1000 });
      await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
      await page.goto("/task/DM-2");
      const header = page.locator(".task-page > header");
      await expect(header.getByRole("heading", { name: "Форма входа показывает ошибку при неверном пароле" })).toBeVisible();
      await expect(header.locator(".crumbs")).toHaveText(/Задачи.*DM-1.*DM-2/);
      await expect(page.locator(".task-page > .tp-head")).toHaveCount(0);
      const roster = page.getByLabel("Участники команды", { exact: true });
      await expect(roster.getByRole("link")).toHaveCount(3);
      await expect(roster.getByRole("link", { name: /Scully/ })).toContainText("аналитик");
      await expect(roster.getByRole("link", { name: /Scully/ })).toContainText("свободен");
      await expect(roster.getByRole("link", { name: /^RoboCop/ })).toContainText("исполнитель");
      const reviewer = roster.getByRole("link", { name: /Picard/ });
      await expect(reviewer).toContainText("ревьюер");
      await expect(reviewer).toContainText("ждёт RoboCop");
      await expect(reviewer).toContainText("✉ 1");
      await expect(reviewer.locator(".tp-member-when")).toHaveText(/4 мин/);
      const description = page.locator(".tp-main > .sec").first();
      const box = (await description.boundingBox())!;
      const rail = (await page.locator(".tp-rail").boundingBox())!;
      if (width >= 1440) expect(Math.abs(box.x + box.width / 2 - width / 2)).toBeLessThan(2);
      if (width > 900) expect(box.x + box.width).toBeLessThanOrEqual(rail.x);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await page.screenshot({ path: `test-results/task-page-roster-${theme}-${width}.png`, animations: "disabled" });
      await reviewer.click();
      await expect(page).toHaveURL(/\/team\/roster-team\/picard$/);
    });
  }
}
