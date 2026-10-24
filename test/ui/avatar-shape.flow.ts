// Shape says who is who: an agent is a rounded square, a person is a circle — on every surface that
// shows the member avatars. The team is stubbed because the test server runs no agents.
import { expect, test, type Page } from "@playwright/test";
import type { TaskSummary } from "../../web/src/shared/api/types.ts";

const at = new Date().toISOString();
const STUB = "shape-test";

/** Enough of a team for the rail, the epic rows and the task list: they all read `members`. */
const teamFor = (taskId: string) => ({
  id: STUB,
  task: taskId,
  cwd: "/tmp",
  state: "active",
  created: at,
  updated: at,
  members: [
    { name: "Бендер", role: "executor", state: "active", activity: "working", status: "Проверяет тесты", statusAt: at },
    { name: "Анна Смирнова", role: "human", state: "active", activity: "idle", status: "", statusAt: at },
  ],
  spec: {
    stage: "delivery",
    workspace: "scratch",
    mail: "flow",
    members: [
      { key: "executor", name: "Бендер", role: "executor" },
      { key: "human", name: "Анна Смирнова", role: "human" },
    ],
    relations: [{ from: "human", to: ["executor"], type: "handoff", on: "start" }],
  },
  taskInfo: { id: taskId, title: "Форма входа", status: "in_progress" },
  pending: {},
  mail: [],
  log: [],
});

/**
 * Puts one team on the seeded tasks and marks DM-1 as their epic. Each handler answers by URL, so the
 * list, a single task, an epic, its usage and the team all stay consistent.
 */
async function stubTeam(page: Page) {
  const response = await page.request.get("/api/tasks?closed=1");
  expect(response.ok()).toBe(true);
  const tasks: TaskSummary[] = await response.json();
  const epic = tasks.find((task) => task.id === "DM-1")!;
  const work = tasks.find((task) => task.id === "DM-2")!;
  const stub = teamFor(work.id);

  // Only the list is stubbed: pages read their own detail from the server, which answers in full.
  await page.route("**/api/tasks**", async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname !== "/api/tasks") return route.fallback();
    return route.fulfill({
      json: tasks.map((task) => task.id === epic.id ? task : { ...task, team: stub.id, parent: epic.id }),
    });
  });
  await page.route("**/api/teams**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    return route.fulfill({ json: path === "/api/teams" ? [stub] : stub });
  });
}

/** The corner radius as a share of the box: computed styles keep percentages written in the sheet. */
const shape = (node: Element) => {
  const style = getComputedStyle(node);
  const box = node.getBoundingClientRect();
  const corner = parseFloat(style.borderTopLeftRadius);
  const percent = style.borderTopLeftRadius.endsWith("%") ? corner : (corner / Math.min(box.width, box.height)) * 100;
  return { percent, width: box.width, raw: style.borderTopLeftRadius };
};

const appearance = (node: Element) => {
  const style = getComputedStyle(node);
  const luminance = (color: string) => {
    const rgb = (color.match(/[\d.]+/g) ?? []).slice(0, 3).map((part) => Number(part) / 255);
    return rgb.map((c) => c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)
      .reduce((sum, c, i) => sum + c * [0.2126, 0.7152, 0.0722][i], 0);
  };
  const a = luminance(style.color);
  const b = luminance(style.backgroundColor);
  return { background: style.backgroundColor, contrast: (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05) };
};

for (const theme of ["ink", "paper"]) {
for (const [route, what] of [
  ["/tasks", "the task list"],
  ["/task/DM-2", "the task page"],
  ["/epic/DM-1", "the epic page"],
] as const) {
  test(`agents are rounded and people are round on ${what} (${theme})`, async ({ page }) => {
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await stubTeam(page);
    await page.goto(route);
    // The task rail now uses the same kit avatars as the detailed team roster.
    const roster = route === "/task/DM-2";
    const agent = page.locator(roster ? ".tp-team-roster .g-avatar.agent" : ".avatars .av.agent").first();
    const human = page.locator(roster ? ".tp-team-roster .g-avatar.human" : ".avatars .av.human").first();
    await expect(agent).toBeVisible();
    await expect(human).toBeVisible();
    await expect(agent).toHaveAttribute("aria-label", /Бендер/);

    const a = await agent.evaluate(shape);
    const h = await human.evaluate(shape);
    // A quarter of the box is a rounded square at any size; a circle is half of it.
    if (roster) {
      expect(a.percent).toBeGreaterThan(20);
      expect(a.percent).toBeLessThan(40);
    } else expect(a.percent).toBeCloseTo(25, 0);
    expect(h.percent).toBeCloseTo(50, 0);
    expect(Math.abs(a.percent - h.percent)).toBeGreaterThan(15);
    for (const avatar of [agent, human]) {
      const colors = await avatar.evaluate(appearance);
      expect(colors.background).not.toBe("rgba(0, 0, 0, 0)");
      expect(colors.contrast).toBeGreaterThanOrEqual(4.5);
    }
    await page.screenshot({ path: `test-results/avatar-shape-${theme}${route.replace(/\W+/g, "-")}.png`, animations: "disabled" });
  });
}

}

test("a person's own photo stays a circle", async ({ page }) => {
  await page.goto("/settings/me");
  const person = page.locator(".st-photo .av.person").first();
  await expect(person).toBeVisible();
  const box = await person.evaluate(shape);
  expect(box.percent).toBeCloseTo(50, 0);
});
