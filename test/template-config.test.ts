import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { exportTemplate, parseTemplateTransfer, templateSchema, templateTransferFile, templateExample } from "../web/src/entities/agent-config/template-transfer.ts";
import { memberKey, nextMemberKey, removeTemplateMember, templateDraft, templateFile, templateRelations, updateTemplateMember } from "../web/src/entities/agent-config/template-file.ts";
import { jsonChangeFields } from "../web/src/entities/agent-config/config-history.ts";
import { layoutTeam, type Catalogue, type TemplateDetail } from "../web/src/entities/agent-config/model.ts";
const fixtures = JSON.parse(readFileSync(new URL("../scripts/ui-preview/fixtures.json", import.meta.url), "utf8"));
const cfg: Catalogue = fixtures["/api/agent-config"];
const full: TemplateDetail = fixtures["/api/templates/full"];

test("every recorded template round-trips without runtime metadata or changing relation mode", () => {
  for (const team of cfg.teams) {
    const exported = exportTemplate(team);
    const parsed = parseTemplateTransfer(JSON.stringify({ $schema: "./genie-team.schema.json", ...exported }));
    const file = templateTransferFile(parsed, cfg);
    assert.deepEqual(file, templateFile(team));
    assert.equal("id" in file, false);
    assert.equal("format" in file, false);
    assert.equal("path" in file, false);
    assert.equal("warnings" in file, false);
    assert.equal("$schema" in file, false);
  }
  assert.deepEqual(JSON.parse(readFileSync(new URL("../web/public/genie-team.schema.json", import.meta.url), "utf8")), templateSchema);
});

test("nested portable definitions reject malformed members, relations and unsupported fields", () => {
  for (const [field, value] of Object.entries({ id: "../bad", format: "v2", members: [], stage: "later", workspace: "disk", projects: ["x", "x"], relations: [{ from: "executor", to: [], type: "handoff" }], extra: true })) {
    assert.throws(() => parseTemplateTransfer(JSON.stringify({ ...templateExample, [field]: value })), new RegExp(field));
  }
  for (const member of [{ role: 12 }, { role: "executor", key: "UPPER" }, { role: "executor", thinking: "ultra" }, { role: "executor", name: "Full Name" }, { role: "executor", extra: 1 }]) {
    assert.throws(() => parseTemplateTransfer(JSON.stringify({ ...templateExample, members: [member] })), /members/);
  }
  for (const key of ["constructor", "toString", "__proto__"]) assert.throws(() => parseTemplateTransfer(JSON.stringify({ ...templateExample, [key]: {} })), new RegExp(key));
  assert.throws(() => parseTemplateTransfer("[1]"), /объект/);
  assert.throws(() => parseTemplateTransfer("bad"), /JSON/);
  assert.throws(() => parseTemplateTransfer(" ".repeat(1_000_001)), /1 МБ/);
});

test("import checks role availability, unique keys and valid relation endpoints", () => {
  for (const members of [[{ role: "unknown" }], [{ role: "orchestrator" }], [{ role: "executor" }, { role: "executor" }], [{ role: "executor", key: "orchestrator" }]]) {
    assert.throws(() => templateTransferFile({ ...templateExample, members }, cfg));
  }
  for (const relation of [
    { from: "missing", to: ["reviewer"], type: "handoff" as const },
    { from: "executor", to: ["executor"], type: "handoff" as const },
    { from: "executor", to: ["orchestrator"], type: "handoff" as const },
    { from: "executor", to: ["reviewer"], type: "reports" as const },
    { from: "executor", to: ["reviewer"], type: "consults" as const, on: "review" },
  ]) assert.throws(() => templateTransferFile({ ...templateExample, relations: [relation] }, cfg));
});

test("renaming and removing a member preserve graph consistency and member settings", () => {
  const draft = templateDraft(full);
  const index = draft.members.findIndex((m) => memberKey(m) === "executor");
  const renamed = updateTemplateMember(draft, index, { key: "builder", name: "bender", thinking: "high", instructions: "Keep evidence" });
  assert.ok(renamed.relations?.some((r) => r.from === "builder"));
  assert.ok(renamed.relations?.some((r) => r.to.includes("builder")));
  assert.ok(renamed.relations?.every((r) => r.from !== "executor" && !r.to.includes("executor")));
  assert.equal(renamed.members[index].thinking, "high");
  assert.equal(draft.members[index].key, "executor");
  const removed = removeTemplateMember(renamed, index);
  assert.ok(removed.relations?.every((r) => r.from !== "builder" && !r.to.includes("builder") && r.to.length));
  assert.equal(nextMemberKey("reviewer", [{ role: "reviewer" }, { role: "reviewer", key: "reviewer-2" }]), "reviewer-3");
});

test("automatic relation previews match the server's process and preserve explicit empty relations", () => {
  const draft = templateFile(full.template);
  delete draft.relations;
  const derived = templateRelations(draft, cfg.roles);
  assert.ok(derived.some((r) => r.from === "executor" && r.on === "review" && r.to.includes("reviewer")));
  assert.ok(derived.some((r) => r.type === "reports" && r.from === "reviewer"));
  assert.deepEqual(templateRelations({ ...draft, relations: [] }, cfg.roles), []);
  const refinement = templateRelations({ ...draft, stage: "refinement" }, cfg.roles);
  assert.ok(refinement.some((r) => r.type === "reports" && r.from === "analyst"));
  assert.ok(!refinement.some((r) => r.from === "executor"));
});

test("editor retains unknown file fields and resolves legacy workspace and automatic relations", () => {
  const automatic = { ...full, template: { ...full.template, relationsDerived: true, workspace: "repo" as const }, file: { ...full.file, path: "teams/full.json", content: '{"worktree":false,"futureSetting":{"keep":true}}' } };
  const draft = templateDraft(automatic);
  assert.deepEqual(draft.futureSetting, { keep: true });
  assert.equal(draft.workspace, "repo");
  assert.equal(draft.relations, undefined);
  assert.equal("worktree" in draft, false);
});

test("template audit compares fields with full JSON values and preserves malformed historical files", () => {
  const diff = jsonChangeFields({ before: '{"title":"Old","members":[{"role":"executor"}]}', after: '{"title":"New","members":[{"role":"executor","thinking":"high"}]}' });
  assert.deepEqual(diff.map((field) => field.key), ["title", "members"]);
  assert.match(diff[1].after!, /high/);
  assert.deepEqual(jsonChangeFields({ before: '{ "title":"A", "members": [] }', after: '{"members":[],"title":"A"}' }), []);
  assert.deepEqual(jsonChangeFields({ before: '{"members":[{"key":"x","role":"executor"}]}', after: '{"members":[{"role":"executor","key":"x"}]}' }), []);
  assert.equal(jsonChangeFields({ before: "broken", after: "{}" })[0].key, "file");
});

test("compact template graphs wrap stages at a readable node size without repeating the global hierarchy", () => {
  const members = full.template.members.map((m) => ({ key: m.key, label: m.role, sub: "" }));
  for (const columns of [1, 2, 3]) {
    const graph = layoutTeam(members, full.template.relations, [], columns);
    assert.ok(graph.width <= 840);
    assert.equal(graph.nodeW, 200);
    assert.ok(!graph.nodes.some((node) => node.key === "project-manager"));
    assert.ok(graph.nodes.some((node) => node.key === "orchestrator"));
    for (const node of graph.nodes) assert.ok(node.y > 0 && node.y < graph.height);
    for (let i = 0; i < graph.nodes.length; i++) for (let j = i + 1; j < graph.nodes.length; j++) {
      const a = graph.nodes[i], b = graph.nodes[j];
      assert.ok(Math.abs(a.x - b.x) >= graph.nodeW || Math.abs(a.y - b.y) >= 88);
    }
  }
});
