// The web's agent-configuration helpers (web/src/entities/agent-config/model.ts):
// permissions from checkboxes, role-file edits that keep everything else, the
// template graph and which templates a task can use.

import assert from "node:assert/strict";
import { test } from "node:test";
import {
  allowDeny,
  mcpGrants,
  roleMcpAccess,
  sectionProblems,
  type Problem,
  basePermissions,
  globMatch,
  flowStages,
  handoffRanks,
  layoutTeam,
  liveTeam,
  newRoleFile,
  problemLines,
  reached,
  refusalMessages,
  type Relation,
  type RoleDef,
  setBody,
  setFrontmatterKey,
  splitFrontmatter,
  type TeamDef,
  type TeamSpecView,
  templatesFor,
  toolUsers,
} from "../web/src/entities/agent-config/model.ts";

const ORDER = ["status.refine", "status.start", "status.submit", "status.approve", "task.check", "docs.read"];

test("checkboxes become allow and deny relative to the class, or to the parent role", () => {
  assert.deepEqual(allowDeny(["status.approve", "docs.read"], ["docs.read", "task.check", "status.approve"], ORDER), { allow: ["task.check"], deny: [] });
  assert.deepEqual(allowDeny(["status.approve", "docs.read"], ["docs.read"], ORDER), { allow: [], deny: ["status.approve"] });
  const role = (id: string, extra: Partial<RoleDef>): RoleDef =>
    ({ id, title: id, description: "", class: "reviewer", names: [], allow: [], deny: [], capabilities: [], files: "read", denyCommands: [], mcp: [], stages: [], origin: "custom", ...extra }) as RoleDef;
  const classes = { reviewer: ["status.approve", "docs.read"] };
  const parent = role("reviewer", { capabilities: ["status.approve", "docs.read", "task.check"] });
  assert.deepEqual(basePermissions(role("sec", { extends: "reviewer" }), [parent], classes), parent.capabilities);
  assert.deepEqual(basePermissions(role("sec", {}), [parent], classes), classes.reviewer);
});

const FILE = `---
title: Ревьюер безопасности
allow: [task.check]
deny:
  - status.approve
instructions: |
  Check secrets.

  Then OWASP.
mcp: [semgrep]
---
You review security.
`;

test("a role file field is replaced, added or removed; the rest stays", () => {
  let t = setFrontmatterKey(FILE, "deny", ["status.return"]);
  assert.match(t, /^deny: \[status\.return\]$/m);
  assert.doesNotMatch(t, /- status\.approve/);
  t = setFrontmatterKey(t, "instructions", undefined);
  assert.doesNotMatch(t, /OWASP|instructions/);
  assert.match(t, /^mcp: \[semgrep\]$/m, "the field after the removed block survives");
  t = setFrontmatterKey(t, "mcp", ["semgrep", "github:get_*"]);
  assert.match(t, /^mcp: \[semgrep, "github:get_\*"\]$/m, "items with `:` are quoted");
  t = setFrontmatterKey(t, "denyCommands", ["git push*"]);
  assert.match(t, /^denyCommands: \["git push\*"\]$/m);
  t = setFrontmatterKey(t, "description", "Checks: secrets, OWASP");
  assert.match(t, /^description: Checks: secrets, OWASP$/m);
  t = setFrontmatterKey(t, "model", "[x]");
  assert.match(t, /^model: "\[x\]"$/m, "a scalar that would read as a list is quoted");
  t = setFrontmatterKey(t, "instructions", "Line one\nLine two");
  assert.match(t, /^instructions: \|\n {2}Line one\n {2}Line two$/m);
  const fm = splitFrontmatter(t);
  assert.ok(fm);
  assert.equal(fm.body, "You review security.\n", "the prompt is untouched");
  assert.equal(fm.lines.filter((l) => l.startsWith("title:")).length, 1);
});

test("problems point at the lines of the file they are about", () => {
  const role = "---\ntitle: QA\n  bad: indent\nfiles: sometimes\nallow: [fly]\n---\nBody\n";
  const marks = problemLines(role, ["line 3: unexpected indentation", "`files: sometimes`: expected write, read or none", "allow: unknown permission `fly`", "role ghost not found"]);
  assert.deepEqual([...marks.keys()], [3, 4, 5], "by line number, then by the field a message starts with; the rest has no line");
  assert.deepEqual(marks.get(4), ["`files: sometimes`: expected write, read or none"]);
  const template = '{\n  "title": "Pair",\n  "stage": "later",\n  "members": []\n}\n';
  assert.deepEqual([...problemLines(template, ["`stage: later`: expected refinement or delivery"]).keys()], [3], "a JSON key");
  assert.deepEqual([...problemLines(template, ["expected `,` or `}` at line 4 column 3"]).keys()], [4]);
  assert.equal(problemLines(template, ["line 99: x"]).size, 0, "a line the text does not have");
  const refused = refusalMessages("not saved: role:reviewer: line 3: title: unexpected indented lines after a value; team:pair: `stage: later`: expected refinement or delivery");
  assert.deepEqual(refused, ["line 3: title: unexpected indented lines after a value", "`stage: later`: expected refinement or delivery"]);
  assert.deepEqual([...problemLines(role, refused).keys()], [3], "a refused save's messages name lines too");
});

test("the prompt of a role file is replaced without touching its settings", () => {
  const t = setBody(FILE, "You review *security*.\n\nOnly security.\n\n\n");
  assert.equal(t.split("---\n")[1], FILE.split("---\n")[1], "the frontmatter stays as it was");
  assert.ok(t.endsWith("---\nYou review *security*.\n\nOnly security.\n"), t);
  assert.equal(setBody("---\nbase: tester\n---\nOld.\n", ""), "---\nbase: tester\n---\n", "an empty prompt: the built-in (or the parent's) applies");
  assert.equal(setBody("Just a prompt.\n", "New."), "New.\n", "a file without frontmatter stays without");
  assert.equal(setBody("---\n---\n", "Body"), "---\n---\nBody\n");
});

test("a file without frontmatter gets one; an override may be frontmatter only", () => {
  assert.equal(setFrontmatterKey("Prompt only.\n", "model", "openai/gpt-6"), "---\nmodel: openai/gpt-6\n---\nPrompt only.\n");
  assert.equal(setFrontmatterKey("---\n---\n", "allow", []), "---\nallow: []\n---\n");
  assert.equal(setFrontmatterKey("---\nallow: []\n---\n", "allow", undefined), "---\n---\n");
  const created = newRoleFile({ title: "QA", description: "Tests things.", base: "tester", prompt: "You test.\n" });
  assert.equal(created, "---\ntitle: QA\ndescription: Tests things.\nbase: tester\n---\nYou test.\n");
});

test("the template graph puts the orchestrator on top and flags members named by problems", () => {
  const g = layoutTeam(
    [
      { key: "executor", label: "Исполнитель", sub: "executor" },
      { key: "reviewer", label: "Ревьюер", sub: "reviewer" },
    ],
    [
      { from: "executor", to: ["reviewer"], type: "handoff", on: "review" },
      { from: "reviewer", to: ["executor"], type: "returns", on: "changes_requested" },
      { from: "reviewer", to: ["orchestrator"], type: "reports" },
      { from: "ghost", to: ["executor"], type: "consults" },
    ],
    ["`reviewer` waits for a handoff that never comes"],
  );
  assert.deepEqual(
    g.nodes.map((n) => [n.key, !!n.flagged]),
    [
      ["project-manager", false],
      ["orchestrator", false],
      ["executor", false],
      ["reviewer", true],
    ],
  );
  const [project, orch, exec, rev] = g.nodes;
  assert.ok(project.y < orch.y);
  assert.equal(project.label, "Менеджер проекта");
  assert.equal(orch.label, "Менеджер задачи");
  assert.ok(orch.y < exec.y && exec.y === rev.y && exec.x < rev.x);
  assert.equal(g.edges.length, 4, "a relation from an unknown member is not drawn");
  const handoff = g.edges.find((e) => e.type === "handoff")!;
  const back = g.edges.find((e) => e.type === "returns")!;
  assert.ok(handoff.ly < exec.y && back.ly > exec.y, "forward above the row, returns below");
  assert.ok(g.height > back.ly);
  const five = layoutTeam(
    ["a", "b", "c", "d", "e"].map((key) => ({ key, label: key, sub: "" })),
    [{ from: "a", to: ["e"], type: "handoff" }],
  );
  assert.equal(five.nodeW, 240, "cards retain readable widths");
  assert.equal(five.stages.length, 2, "parallel members share a stage");
  assert.ok(five.edges.every((edge) => !/[QC]/.test(edge.d)), "connections contain only straight segments");
});

test("handoff stages follow dependencies regardless of member order and tolerate cycles", () => {
  const members = ["review", "build", "plan", "test"].map((key) => ({ key, label: key, sub: "" }));
  const g = layoutTeam(members, [
    { from: "plan", to: ["build"], type: "handoff" },
    { from: "build", to: ["review", "test"], type: "handoff" },
    { from: "review", to: ["build"], type: "returns" },
  ]);
  const at = (key: string) => g.nodes.find((n) => n.key === key)!;
  assert.ok(at("plan").x < at("build").x && at("build").x < at("review").x);
  assert.equal(at("test").x, at("review").x);
  assert.notEqual(at("test").y, at("review").y);
  const cycle = layoutTeam(members, [
    { from: "plan", to: ["build"], type: "handoff" },
    { from: "build", to: ["plan"], type: "handoff" },
  ]);
  assert.equal(cycle.nodes.length, members.length + 2);
  assert.ok(cycle.nodes.every((n) => Number.isFinite(n.x) && n.y < cycle.height));
  assert.equal(layoutTeam([], []).edges.length, 1, "management levels remain connected");
});

test("the team's line groups members by stage in handoff order", () => {
  const relations: Relation[] = [
    { from: "plan", to: ["build"], type: "handoff" },
    { from: "build", to: ["review", "test"], type: "handoff", on: "review" },
    { from: "review", to: ["build"], type: "returns" },
    { from: "build", to: ["plan"], type: "consults" },
  ];
  assert.deepEqual(flowStages(["review", "build", "plan", "test"], relations), [["plan"], ["build"], ["review", "test"]]);
  assert.deepEqual(flowStages(["a", "b"], []), [["a", "b"]], "without handoffs everyone shares one stage");
  assert.deepEqual(flowStages([], relations), []);
  assert.deepEqual(handoffRanks(["plan", "build"], [{ from: "plan", to: ["build"], type: "handoff" }, { from: "build", to: ["plan"], type: "handoff" }]).get("plan"), 0, "a cycle still gets a stage");
});

test("a task before ready gets refinement templates, a ready one delivery templates", () => {
  const t = (id: string, stage: "refinement" | "delivery") => ({ id, stage }) as TeamDef;
  const teams = [t("research", "refinement"), t("standard", "delivery"), t("pair", "delivery")];
  assert.deepEqual(templatesFor(teams, "refining").map((x) => x.id), ["research"]);
  assert.deepEqual(templatesFor(teams, "ready").map((x) => x.id), ["standard", "pair"]);
  assert.deepEqual(templatesFor(teams, "changes_requested").map((x) => x.id), ["standard", "pair"]);
});

test("a connection's tool goes to the roles granted all of it or a matching pattern", () => {
  const role = (id: string, mcp: string[]) => ({ id, mcp }) as RoleDef;
  const roles = [role("lead", ["github"]), role("reader", ["github:get_*", "github:list_?"]), role("admin", ["*"]), role("other", ["jira"])];
  assert.deepEqual(toolUsers(roles, "github", "get_issue"), ["lead", "reader", "admin"]);
  assert.deepEqual(toolUsers(roles, "github", "list_x"), ["lead", "reader", "admin"]);
  assert.deepEqual(toolUsers(roles, "github", "list_xy"), ["lead", "admin"], "`?` is one character");
  assert.deepEqual(toolUsers(roles, "github", "delete_repo"), ["lead", "admin"]);
  assert.deepEqual(toolUsers(roles, "githubx", "get_issue"), ["admin"], "another connection whose name starts the same");
  assert.ok(globMatch("a.b*", "a.bc") && !globMatch("a.b*", "aXbc"), "regex characters are literal");
});

test("a running team shows who works and who waits for whose handoff", () => {
  const spec: TeamSpecView = {
    stage: "delivery",
    workspace: "worktree",
    mail: "open",
    members: [
      { key: "executor", name: "bender", role: "executor" },
      { key: "reviewer", name: "yoda", role: "reviewer" },
      { key: "tester", name: "chaos", role: "tester" },
    ],
    relations: [
      { from: "executor", to: ["reviewer", "tester"], type: "handoff", on: "review" },
      { from: "reviewer", to: ["executor"], type: "returns", on: "changes_requested" },
    ],
  };
  const members = [
    { name: "bender", activity: "working", state: "active" },
    { name: "yoda", activity: "idle", state: "active" },
    { name: "chaos", activity: "error", state: "active" },
  ];
  const now = liveTeam(spec, members, "in_progress", true);
  assert.equal(now.live.executor.state, "working");
  assert.deepEqual(now.live.reviewer, { state: "waiting", note: "ждёт Bender" });
  assert.equal(now.live.tester.state, "error", "an error shows whatever the flow");
  assert.deepEqual(now.pending, ["executor->reviewer"]);
  assert.deepEqual(now.done, [], "nothing has been handed over yet");
  assert.deepEqual(liveTeam(spec, members, "in_progress", true, (n) => n.toUpperCase()).live.reviewer.note, "ждёт BENDER");
  const inReview = liveTeam(spec, members, "review", true);
  assert.equal(inReview.live.reviewer.state, "idle", "the handoff has happened");
  assert.deepEqual(inReview.pending, []);
  assert.deepEqual(inReview.done, ["executor->reviewer", "executor->tester"], "the handoff on review has happened");
  assert.equal(liveTeam(spec, members, "review", false).live.executor.state, "stopped");
  assert.ok(reached("approved", "review") && !reached("changes_requested", "review") && reached("changes_requested", "in_progress"));
});

test("configuration failures stay in their own section even when an item could not load", () => {
  const problems: Problem[] = ["role:broken", "team:missing", "skill", "skill:bad", "mcp.json", "mcp:offline"].map((item) => ({ item, level: "error", message: "Invalid configuration" }));
  assert.deepEqual(sectionProblems(problems, "roles").map((p) => p.item), ["role:broken"]);
  assert.deepEqual(sectionProblems(problems, "templates").map((p) => p.item), ["team:missing"]);
  assert.deepEqual(sectionProblems(problems, "skills").map((p) => p.item), ["skill", "skill:bad"]);
  assert.deepEqual(sectionProblems(problems, "mcp").map((p) => p.item), ["mcp.json", "mcp:offline"]);
});

test("MCP edits preserve unavailable grants and explicitly revoke inherited connections", () => {
  const servers = [{ id: "github" }, { id: "search" }];
  const access = ["github:get_*", "offline:read"];
  const grants = mcpGrants(access, servers);
  assert.equal(grants.github.mode, "tools");
  assert.deepEqual(grants.github.tools, ["get_*"]);
  assert.equal(grants.search.mode, "none");
  grants.github = { mode: "none", tools: [] };
  grants.search = { mode: "all", tools: [] };
  assert.deepEqual(roleMcpAccess(access, servers, grants), ["offline:read", "search"]);
  const wildcard = mcpGrants(["*"], servers);
  assert.equal(wildcard.github.mode, "all");
  assert.equal(wildcard.search.mode, "all");
  wildcard.github.mode = "none";
  wildcard.search.mode = "none";
  assert.deepEqual(roleMcpAccess(["*"], servers, wildcard), []);
  assert.match(setFrontmatterKey("---\nmcp: [github]\n---\n", "mcp", []), /mcp: \[\]/);
});
