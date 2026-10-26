import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { exportRole, parseRoleTransfer, roleSchema, roleTemplate, roleTransferFile } from "../web/src/entities/agent-config/role-transfer.ts";
import { roleChangeFields } from "../web/src/entities/agent-config/role-history.ts";
import type { Catalogue, RoleDetail } from "../web/src/entities/agent-config/model.ts";

const fixtures = JSON.parse(readFileSync(new URL("../scripts/ui-preview/fixtures.json", import.meta.url), "utf8"));
const cfg: Catalogue = fixtures["/api/agent-config"];
const detail: RoleDetail = fixtures["/api/roles/integration-analyst"];

test("portable role round-trip keeps prompt, permissions, unavailable grants and explicit empty skills", () => {
  const role = exportRole({ ...detail.role, skills: [] });
  const encoded = JSON.stringify({ $schema: "./genie-role.schema.json", ...role });
  const loaded = parseRoleTransfer(encoded);
  assert.deepEqual(loaded.mcp, role.mcp);
  assert.deepEqual(loaded.skills, []);
  const file = roleTransferFile(loaded, cfg);
  assert.ok(file.includes(role.prompt));
  assert.match(file, /skills: \[\]/);
  assert.match(file, /archived:read_\*/);
  assert.match(file, /base: analyst/);
  assert.ok(!encoded.includes('"path"'));
  assert.ok(!encoded.includes('"usedBy"'));
  const noSkills = exportRole({ ...detail.role, skills: undefined });
  assert.ok(!roleTransferFile(parseRoleTransfer(JSON.stringify(noSkills)), cfg).includes("skills:"));
});

test("downloadable schema and importer reject invalid external role definitions", () => {
  assert.equal(roleSchema.additionalProperties, false);
  assert.deepEqual(JSON.parse(readFileSync(new URL("../web/public/genie-role.schema.json", import.meta.url), "utf8")), roleSchema);
  assert.deepEqual(parseRoleTransfer(JSON.stringify(roleTemplate)), roleTemplate);
  for (const [field, value] of Object.entries({ id: "../bad", base: "unknown", files: "execute", thinking: "ultra", skills: "all", stages: ["deploy"], prompt: "  ", names: ["Invalid Name"], mcp: ["x", "x"], extra: true })) {
    assert.throws(() => parseRoleTransfer(JSON.stringify({ ...roleTemplate, [field]: value })), new RegExp(field));
  }
  assert.throws(() => parseRoleTransfer("[1,2]"), /объект/);
  assert.throws(() => parseRoleTransfer("{bad"), /JSON/);
  assert.throws(() => parseRoleTransfer(" ".repeat(1_000_001)), /1 МБ/);
  assert.throws(() => roleTransferFile({ ...roleTemplate, permissions: ["not.a.permission"] }, cfg), /Неизвестные разрешения/);
});

test("portable exports flatten custom inheritance and preserve effective permissions", () => {
  const loaded = exportRole({ ...detail.role, extends: "custom-parent", capabilities: ["docs.read"] });
  const file = roleTransferFile(loaded, cfg);
  assert.doesNotMatch(file, /extends:/);
  assert.match(file, /deny:/);
  assert.match(file, /base: analyst/);
});

test("history field comparison separates metadata and prompt without losing multiline values", () => {
  const diff = roleChangeFields({ before: "---\ntitle: Old\ninstructions: |\n  first\n  second\n---\nold prompt\n", after: "---\ntitle: New\ninstructions: |\n  first\n  second\n---\nnew prompt\n" });
  assert.deepEqual(diff.map((f) => f.key), ["title", "prompt"]);
  assert.equal(diff[0].before, "Old");
  assert.equal(diff[1].after, "new prompt\n");
  assert.equal(roleChangeFields({ after: "---\ntitle: New\n---\nPrompt" })[0].before, undefined);
});
