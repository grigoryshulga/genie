// The genie guard pi extension (crates/genie/pi/genie-guard.ts): which shell
// commands a role's denyCommands stop and which MCP calls stay within its grants.
// The extension itself is exercised with the real pi in crates/genie/tests/sessions.rs.

import assert from "node:assert/strict";
import { test } from "node:test";
import { deniedBy, mcpDenial, type Policy, simpleCommands, usageOf, withoutHeapCap } from "../crates/genie/pi/genie-guard.ts";

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
  assert.equal(mcpDenial(policy, "mcp", { tool: "semgrep_scan", args: {} }), undefined);
  assert.equal(mcpDenial(policy, "mcp", { server: "semgrep", tool: "scan" }), undefined);
  assert.equal(mcpDenial(policy, "mcp__semgrep", { tool: "scan" }), undefined);
  // Tool patterns.
  assert.equal(mcpDenial(policy, "mcp", { tool: "github_get_issue" }), undefined);
  assert.equal(mcpDenial(policy, "mcp", { server: "github", tool: "list_commits" }), undefined);
  assert.match(mcpDenial(policy, "mcp", { tool: "github_delete_repository" }) ?? "", /only these tools of github: get_\*, list_commits/);
  assert.match(mcpDenial(policy, "mcp__github", { tool: "merge_pull_request" }) ?? "", /only these tools of github/);
  // Other connections.
  assert.match(mcpDenial(policy, "mcp", { server: "sap-dev", tool: "run" }) ?? "", /sap-dev is not granted.*semgrep, github/);
  assert.match(mcpDenial(policy, "mcp", { connect: "sap-dev" }) ?? "", /not granted/);
  assert.match(mcpDenial(policy, "mcp__sap-dev", { tool: "run" }) ?? "", /not granted/);
  assert.match(mcpDenial(policy, "mcp", { tool: "sap-dev_run" }) ?? "", /not from a granted connection/);
  // Installing servers, listing and searching.
  assert.match(mcpDenial(policy, "mcp", { action: "install", url: "https://example.com/mcp" }) ?? "", /do not install MCP servers/);
  assert.equal(mcpDenial(policy, "mcp", {}), undefined);
  assert.equal(mcpDenial(policy, "mcp", { search: "issue" }), undefined);
  // Other tools are not MCP's business.
  assert.equal(mcpDenial(policy, "bash", { command: "ls" }), undefined);
  // A role without connections.
  assert.match(mcpDenial({ ...policy, mcp: {} }, "mcp", { tool: "github_get_issue" }) ?? "", /may use MCP connections: none/);
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
