import assert from "node:assert/strict";
import { test } from "node:test";
import { canManageProject, managementDestination } from "../web/src/shared/lib/management.ts";

test("administration is available to server and project administrators", () => {
  for (const role of [undefined, "viewer", "member"]) assert.equal(canManageProject(false, role), false);
  for (const role of ["admin", "owner"]) assert.equal(canManageProject(false, role), true);
  assert.equal(canManageProject(true, "viewer"), true);
});

test("old settings links land in the corresponding management section", () => {
  for (const [scope, old, next] of [
    ["project", "agents", "/manage/project/general"],
    ["project", "decisions", "/manage/project/tools"],
    ["project", "telegram", "/manage/project/channels"],
    ["project", "automations", "/manage/project/automations"],
    ["server", "stats", "/statistics"],
    ["server", "mcp", "/manage/server/tools"],
    ["server", "skills", "/manage/server/skills"],
    ["server", "providers", "/manage/server/providers"],
    ["server", "vault", "/manage/server/vault"],
  ] as const)
    assert.equal(managementDestination(scope, old), next);
});

test("settings tabs validate deep links and retain standalone editor destinations", async () => {
  const { settingsTab, settingsTabHref } = await import("../web/src/shared/lib/management.ts");
  assert.equal(settingsTab("project", "tools", "jev"), "jev");
  assert.equal(settingsTab("project", "tools", "invalid"), "skills");
  assert.equal(settingsTab("server", "accounts", null), "list");
  assert.equal(settingsTab("project", "repos", "environment"), "environment");
  assert.equal(settingsTabHref("project", "tools", "automations"), "/manage/project/automations");
  assert.equal(settingsTabHref("server", "tools", "agents"), "/manage/server/roles");
  assert.equal(settingsTabHref("server", "tools", "vault"), "/manage/server/vault");
  assert.equal(settingsTabHref("project", "tools", "mcp"), "/manage/project/tools?tab=mcp");
});
