import assert from "node:assert/strict";
import { test } from "node:test";
import { activeHubItem, canManageProject, currentHubTab, HUB_TABS, hubHome, managementDestination, visibleHub } from "../web/src/shared/lib/management.ts";

test("administration is available to server and project administrators", () => {
  for (const role of [undefined, "viewer", "member"]) assert.equal(canManageProject(false, role), false);
  for (const role of ["admin", "owner"]) assert.equal(canManageProject(false, role), true);
  assert.equal(canManageProject(true, "viewer"), true);
});

test("the hub shows each person only the groups and pages they may open", () => {
  const groups = (viewer: { serverAdmin: boolean; projectAdmin: boolean }) => visibleHub(viewer).map((g) => g.id);
  assert.deepEqual(groups({ serverAdmin: true, projectAdmin: true }), ["settings", "agents", "admin"]);
  assert.deepEqual(groups({ serverAdmin: false, projectAdmin: true }), ["settings", "agents"]);
  assert.deepEqual(groups({ serverAdmin: false, projectAdmin: false }), ["agents"]);
  const memberAgents = visibleHub({ serverAdmin: false, projectAdmin: false })[0].items.map((i) => i.id);
  assert.deepEqual(memberAgents, ["roles", "templates", "skills", "mcp", "automations"]);
  // Project members are managed by server administrators only.
  assert.ok(!visibleHub({ serverAdmin: false, projectAdmin: true }).some((g) => g.items.some((i) => i.id === "members")));
  assert.equal(hubHome({ serverAdmin: false, projectAdmin: true }), "/manage/project/general");
  assert.equal(hubHome({ serverAdmin: false, projectAdmin: false }), "/manage/agents/roles");
});

test("nested hub pages light up their parent page and tab", () => {
  assert.equal(activeHubItem("/manage/agents/automations/observation/7")?.id, "automations");
  assert.equal(activeHubItem("/manage/agents/skills/project")?.id, "skills");
  assert.equal(activeHubItem("/manage/server/stats")?.id, "stats");
  assert.equal(activeHubItem("/board"), undefined);
  assert.equal(currentHubTab(HUB_TABS.automations, "/manage/agents/automations")?.id, "rules");
  assert.equal(currentHubTab(HUB_TABS.automations, "/manage/agents/automations/runs")?.id, "runs");
  assert.equal(currentHubTab(HUB_TABS.automations, "/manage/agents/automations/observation/7")?.id, "observations");
  assert.equal(currentHubTab(HUB_TABS.skills, "/manage/agents/skills/project")?.id, "project");
});

test("old settings links land in the corresponding hub page", () => {
  for (const [scope, old, tab, next] of [
    ["project", "agents", null, "/manage/agents/work"],
    ["project", "general", "agents", "/manage/agents/work"],
    ["project", "decisions", null, "/manage/agents/jev"],
    ["project", "telegram", null, "/manage/project/channels"],
    ["project", "tools", "mcp", "/manage/agents/mcp"],
    ["project", "tools", null, "/manage/agents/skills"],
    ["project", "automations", null, "/manage/agents/automations"],
    ["project", "automation-runs", null, "/manage/agents/automations/runs"],
    ["project", "observations", null, "/manage/agents/automations/observations"],
    ["project", "observation", null, "/manage/agents/automations/sources"],
    ["project", "people", "invites", "/manage/server/members"],
    ["project", "repos", null, "/manage/project/repos"],
    ["server", "stats", null, "/manage/server/stats"],
    ["server", "mcp", null, "/manage/server/tools"],
    ["server", "skills", null, "/manage/agents/skills"],
    ["server", "agents", null, "/manage/agents/roles"],
    ["server", "providers", null, "/manage/server/providers"],
    ["server", "vault", null, "/manage/server/vault"],
  ] as const)
    assert.equal(managementDestination(scope, old, tab), next, `${scope}/${old}?tab=${tab}`);
});

test("settings tabs validate deep links", async () => {
  const { settingsTab, settingsTabHref } = await import("../web/src/shared/lib/management.ts");
  assert.equal(settingsTab("server", "tools", "jev"), "jev");
  assert.equal(settingsTab("server", "tools", "invalid"), "mcp");
  assert.equal(settingsTab("server", "accounts", null), "list");
  assert.equal(settingsTab("server", "members", "invites"), "invites");
  assert.equal(settingsTab("project", "repos", "environment"), "environment");
  assert.equal(settingsTabHref("server", "members", "invites"), "/manage/server/members?tab=invites");
});
