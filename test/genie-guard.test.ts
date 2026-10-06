// The genie guard pi extension (crates/genie/pi/genie-guard.ts): which shell
// commands a role's denyCommands stop and which MCP calls stay within its grants.
// The extension itself is exercised with the real pi in crates/genie/tests/sessions.rs.

import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { deniedBy, mcpDenial, mcpOwner, type Policy, readMcpConfig, resourceDenial, simpleCommands, usageOf, withoutHeapCap } from "../crates/genie/pi/genie-guard.ts";

test("a shell line is split into simple commands without assignments and wrappers", () => {
  assert.deepEqual(simpleCommands("cd app && FOO=1 git push origin main; ls | wc -l"), ["cd app", "git push origin main", "ls", "wc -l"]);
  assert.deepEqual(simpleCommands("sudo -u deploy /usr/bin/git  reset   --hard"), ["git reset --hard"]);
  assert.deepEqual(simpleCommands("echo $(git push) `git status`"), ["echo", "git push", "git status"]);
});

test("denyCommands match whole simple commands, also inside quotes", () => {
  const deny = ["git push*", "git reset --hard*", "rm -rf /"];
  assert.equal(deniedBy("git push", deny), "git push*");
  assert.equal(deniedBy("make test && git push --force", deny), "git push*");
  assert.equal(deniedBy("bash -c 'git reset --hard HEAD~1'", deny), "git reset --hard*");
  assert.equal(deniedBy("env GIT_TRACE=1 git push", deny), "git push*");
  assert.equal(deniedBy("git status && git log", deny), undefined);
  assert.equal(deniedBy("echo git push", deny), undefined, "an argument is not a command");
  assert.equal(deniedBy("rm -rf /tmp/x", deny), undefined, "patterns are anchored");
});

const policy: Policy = {
  role: "security-reviewer",
  files: "read",
  denyCommands: [],
  mcp: { semgrep: null, github: ["get_*", "list_commits"] },
};

test("MCP calls reach only granted connections and tools", () => {
  // Whole connections.
  assert.equal(mcpDenial(policy, "semgrep", "scan"), undefined);
  // Tool patterns, matched against the server's own tool names.
  assert.equal(mcpDenial(policy, "github", "get_issue"), undefined);
  assert.equal(mcpDenial(policy, "github", "list_commits"), undefined);
  assert.match(mcpDenial(policy, "github", "delete_repository") ?? "", /only these tools of github: get_\*, list_commits/);
  assert.match(mcpDenial(policy, "github", "get-issue") ?? "", /only these tools/, "a name with other characters is another tool");
  // Other connections.
  assert.match(mcpDenial(policy, "sap-dev", "run") ?? "", /sap-dev is not granted.*semgrep, github/);
  assert.match(mcpDenial(policy, "toString", "run") ?? "", /not granted/, "a name from Object.prototype is no grant");
  // A role without connections.
  assert.match(mcpDenial({ ...policy, mcp: {} }, "github", "get_issue") ?? "", /may use MCP connections: none/);
});

test("a pi MCP tool is traced to its connection by its label", () => {
  assert.deepEqual(mcpOwner("github/get_issue"), { server: "github", tool: "get_issue" });
  assert.deepEqual(mcpOwner("docs-eu/files/read"), { server: "docs-eu", tool: "files/read" }, "the tool's own name may hold a slash");
  assert.equal(mcpOwner("get_issue"), undefined);
  assert.equal(mcpOwner(undefined), undefined);
});

test("MCP resources come with the whole connection only", () => {
  assert.equal(resourceDenial(policy, "read_mcp_resource", { server: "semgrep", uri: "x://y" }), undefined);
  assert.match(resourceDenial(policy, "read_mcp_resource", { server: "github", uri: "x://y" }) ?? "", /not its resources/);
  assert.match(resourceDenial(policy, "list_mcp_resources", { server: "sap-dev" }) ?? "", /not granted/);
  assert.equal(resourceDenial(policy, "list_mcp_resources", {}), undefined, "a listing is limited by the config");
  assert.equal(resourceDenial(policy, "bash", { server: "github" }), undefined);
});

test("the role's MCP config is read as pi's mcp.json; a missing or broken one is no connections", () => {
  const dir = mkdtempSync(join(tmpdir(), "guard-"));
  const file = join(dir, "mcp.json");
  assert.deepEqual(readMcpConfig(""), {});
  assert.deepEqual(readMcpConfig(file), {}, "not there");
  writeFileSync(file, '{"mcpServers": {"docs": {"url": "http://x/mcp", "exposure": "direct"}}, "autoEnableCodemode": false}');
  assert.deepEqual(readMcpConfig(file), { mcpServers: { docs: { url: "http://x/mcp", exposure: "direct" } }, autoEnableCodemode: false });
  writeFileSync(file, "{broken");
  assert.deepEqual(readMcpConfig(file), {});
});

test("a model response is reported with its provider and tokens", () => {
  const usage = { input: 1200, output: 80, cacheRead: 30000, cacheWrite: 0, totalTokens: 31280, cost: { total: 0 } };
  assert.deepEqual(usageOf({ role: "assistant", provider: "litellm", model: "claude-opus-5-5", usage }), {
    model: "litellm/claude-opus-5-5",
    input: 1200,
    output: 80,
    cacheRead: 30000,
    cacheWrite: 0,
  });
  assert.equal(usageOf({ role: "assistant", provider: "litellm", model: "litellm/x", usage })?.model, "litellm/x");
  assert.equal(usageOf({ role: "user", usage }), undefined);
  assert.equal(usageOf({ role: "assistant", model: "m", usage: { input: 0, output: 0 } }), undefined, "a failed request spent nothing");
});

test("the heap cap put on pi is taken off NODE_OPTIONS for its commands, other options stay", () => {
  assert.equal(withoutHeapCap("--max-old-space-size=2048", "2048"), "");
  assert.equal(withoutHeapCap("--no-warnings --max-old-space-size=2048", "2048"), "--no-warnings");
  assert.equal(withoutHeapCap("--max-old-space-size=8192", "2048"), "--max-old-space-size=8192", "not ours");
});
