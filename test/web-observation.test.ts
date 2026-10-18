// The observation entity's pure helpers: labels, case actions per role, deltas, contract templates, keys.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import {
  type CaseAction,
  type CaseRights,
  INCIDENT_LABEL,
  POLICY_LABEL,
  RESULT_LABEL,
  STATUS_LABEL,
  caseActions,
  caseListQuery,
  contractTemplate,
  evidenceListQuery,
  formatDelta,
  isActive,
  observationKeys,
} from "../web/src/entities/observation/model.ts";
import type { CaseStatus } from "../web/src/shared/api/generated/CaseStatus.ts";

/** The value rows of one UI key in the observation table of GLOSSARY.md: `| \`status\` | \`proposed\` | Черновик |`. */
function glossary(uiKey: string): Record<string, string> {
  const text = readFileSync(new URL("../GLOSSARY.md", import.meta.url), "utf8");
  const rows: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const row = line.match(/^\| `(\w+)` \| `(\w+)` \| (.+?) \|$/);
    if (row && row[1] === uiKey) rows[row[2]] = row[3];
  }
  return rows;
}

/** The members of a generated string union, as ts-rs writes them (`export type CaseStatus = "a" | "b";`). */
function generatedUnion(name: string): string[] {
  const text = readFileSync(new URL(`../web/src/shared/api/generated/${name}.ts`, import.meta.url), "utf8");
  const line = text.split("\n").find((l) => l.startsWith(`export type ${name} = `));
  assert.ok(line, `${name} is a generated string union`);
  return [...line.matchAll(/"([^"]+)"/g)].map((m) => m[1]).sort();
}

const STATUSES: CaseStatus[] = ["proposed", "waiting", "observing", "evaluating", "concluded", "cancelled"];

// --- labels ----------------------------------------------------------------------------------------

test("the labels are the glossary's Russian words", () => {
  assert.deepEqual(STATUS_LABEL, glossary("status"));
  assert.deepEqual(RESULT_LABEL, glossary("result"));
  assert.deepEqual(POLICY_LABEL, glossary("policy"));
  assert.deepEqual(INCIDENT_LABEL, glossary("incident"));
});

test("the labels cover every generated enum value, and no other", () => {
  assert.deepEqual(Object.keys(STATUS_LABEL).sort(), generatedUnion("CaseStatus"));
  assert.deepEqual(Object.keys(RESULT_LABEL).sort(), generatedUnion("CaseResult"));
  assert.deepEqual(Object.keys(POLICY_LABEL).sort(), generatedUnion("ObservationPolicy"));
  assert.deepEqual(Object.keys(INCIDENT_LABEL).sort(), generatedUnion("IncidentStatus"));
  for (const label of [...Object.values(STATUS_LABEL), ...Object.values(RESULT_LABEL), ...Object.values(POLICY_LABEL), ...Object.values(INCIDENT_LABEL)]) {
    assert.notEqual(label.trim(), "", "no empty label");
  }
});

// --- status and case actions -----------------------------------------------------------------------

test("an active case is one that is not concluded or cancelled", () => {
  assert.deepEqual(
    STATUSES.map((s) => [s, isActive(s)]),
    [
      ["proposed", true],
      ["waiting", true],
      ["observing", true],
      ["evaluating", true],
      ["concluded", false],
      ["cancelled", false],
    ],
  );
});

const MANAGER: CaseRights = { writable: true, human: true }; // a person with write access, or a project admin
const AGENT: CaseRights = { writable: true, human: false }; // the project manager agent
const VIEWER: CaseRights = { writable: false, human: true }; // a person who only reads
const TASK_AGENT: CaseRights = { writable: false, human: false }; // a task manager, a member or a job

test("a person who may write can act on a case in every status (§6.2, §14)", () => {
  const expected: Record<CaseStatus, CaseAction[]> = {
    proposed: ["rename", "edit", "contract", "arm", "followup", "cancel"],
    waiting: ["rename", "edit", "contract", "start", "deploy", "followup", "cancel"],
    observing: ["rename", "reviseContract", "deploy", "evaluate", "evaluateFinal", "evidence", "followup", "cancel"],
    evaluating: ["reviseContract", "deploy", "evidence", "followup", "cancel"],
    concluded: ["followup"],
    cancelled: ["followup"],
  };
  for (const status of STATUSES) assert.deepEqual(caseActions({ status }, MANAGER), expected[status], status);
});

test("the project manager agent cannot revise a started contract or request the final evaluation", () => {
  const expected: Record<CaseStatus, CaseAction[]> = {
    proposed: ["rename", "edit", "contract", "arm", "followup", "cancel"],
    waiting: ["rename", "edit", "contract", "start", "deploy", "followup", "cancel"],
    observing: ["rename", "deploy", "evaluate", "evidence", "followup", "cancel"],
    evaluating: ["deploy", "evidence", "followup", "cancel"],
    concluded: ["followup"],
    cancelled: ["followup"],
  };
  for (const status of STATUSES) assert.deepEqual(caseActions({ status }, AGENT), expected[status], status);
});

test("a viewer and a task manager, member or job get no case actions", () => {
  for (const status of STATUSES) {
    assert.deepEqual(caseActions({ status }, VIEWER), [], `viewer, ${status}`);
    assert.deepEqual(caseActions({ status }, TASK_AGENT), [], `task agent, ${status}`);
  }
});

// --- deltas ----------------------------------------------------------------------------------------

test("a relative change reads as a signed percentage with one decimal", () => {
  assert.equal(formatDelta({ deltaRel: -23.1234, deltaAbs: -4 }), "-23.1%", "the relative change wins over the absolute one");
  assert.equal(formatDelta({ deltaRel: 12.96 }), "+13.0%");
  assert.equal(formatDelta({ deltaRel: 0 }), "0.0%");
  assert.equal(formatDelta({ deltaRel: -0.04 }), "0.0%", "a result that rounds to zero has no sign");
});

test("without a relative change the absolute one reads with two decimals", () => {
  assert.equal(formatDelta({ deltaAbs: 0.05 }), "+0.05");
  assert.equal(formatDelta({ deltaAbs: -0.123 }), "-0.12");
  assert.equal(formatDelta({ deltaRel: undefined, deltaAbs: 1.5 }), "+1.50");
  assert.equal(formatDelta({ deltaAbs: 0.001 }), "0.00");
});

test("a missing change reads as an em dash, including a JSON null", () => {
  assert.equal(formatDelta({}), "—");
  assert.equal(formatDelta({ deltaRel: undefined, deltaAbs: undefined }), "—");
  assert.equal(formatDelta({ deltaRel: Number.NaN }), "—");
  assert.equal(formatDelta({ deltaAbs: null as unknown as number }), "—");
});

// --- contract templates ----------------------------------------------------------------------------

test("a contract template parses and carries the criteria of its policy", () => {
  const verify = JSON.parse(contractTemplate("verify"));
  assert.ok(verify.hypotheses.length >= 1, "verify has a hypothesis");
  assert.deepEqual(verify.guardrails, []);
  const guardrails = JSON.parse(contractTemplate("guardrails"));
  assert.deepEqual(guardrails.hypotheses, []);
  assert.ok(guardrails.guardrails.length >= 1, "guardrails has a guardrail");
});

test("a contract template passes the checks of §5.3 that do not need the server", () => {
  for (const policy of ["verify", "guardrails"] as const) {
    const c = JSON.parse(contractTemplate(policy));
    const criteria = [...c.hypotheses, ...c.guardrails];
    const ids = criteria.map((x: { id: string }) => x.id);
    for (const id of ids) assert.match(id, /^[a-z0-9_]{1,64}$/, `${policy}: id ${id}`);
    assert.equal(new Set(ids).size, ids.length, `${policy}: criterion ids are unique`);
    for (const h of c.hypotheses) {
      assert.equal(Object.keys(h.expect).length, 1, `${policy}: exactly one of change, max, min`);
      assert.match(h.expect.change, /^[+-]\d+(\.\d+)?(%|pp)?$/, `${policy}: change ${h.expect.change}`);
      assert.notEqual(Number.parseFloat(h.expect.change), 0, "a hypothesis does not expect zero");
    }
    for (const g of c.guardrails) {
      assert.ok(g.maxChange || g.minChange || g.max !== undefined || g.min !== undefined, `${policy}: ${g.id} has a limit`);
    }
    assert.match(c.observation.minimumWindow, /^\d+[smhd]$/);
    assert.ok(["tasks_done", "deployment", "manual"].includes(c.observation.start));
    assert.ok(["manual", "auto"].includes(c.followup));
  }
  assert.ok(contractTemplate("verify").includes('\n  "hypotheses": ['), "pretty JSON with two-space indentation");
});

// --- queries and keys ------------------------------------------------------------------------------

test("a list query keeps only the values that were given", () => {
  assert.equal(caseListQuery({}), "");
  assert.equal(caseListQuery({ task: "  " }), "", "a blank task is not a filter");
  assert.equal(caseListQuery({ status: "observing", task: "G-1", limit: 20 }), "?status=observing&task=G-1&limit=20");
  assert.equal(evidenceListQuery({ after: 0, criterion: "error_rate" }), "?after=0&criterion=error_rate");
});

test("every observation key starts with the root the journal refetches; sources keep their own", () => {
  const keys = [
    observationKeys.cases({ status: "observing" }),
    observationKeys.case(7),
    observationKeys.evidence(7, { limit: 5 }),
    observationKeys.incidents("open"),
    observationKeys.incidents(),
    observationKeys.task("G-1"),
  ];
  for (const key of keys) assert.equal(key[0], "observations", JSON.stringify(key));
  assert.deepEqual(observationKeys.sources, ["observation-sources"]);
});

test("query keys differ by their parameters", () => {
  assert.notDeepEqual(observationKeys.incidents("open"), observationKeys.incidents("resolved"));
  assert.notDeepEqual(observationKeys.incidents("open"), observationKeys.incidents());
  assert.notDeepEqual(observationKeys.cases({ status: "observing" }), observationKeys.cases({ status: "concluded" }));
  assert.notDeepEqual(observationKeys.case(7), observationKeys.case(8));
});
