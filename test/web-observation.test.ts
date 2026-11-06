// The observation entity's pure helpers: labels, case actions per role, deltas, contract templates, keys.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import {
  ACTION_LABEL,
  CASE_ACTIONS,
  CONFIDENCE_LABEL,
  CONTRACT_SHAPE_ERROR,
  CRITERION_STATE_LABEL,
  type CaseAction,
  type CaseRights,
  FIELD_LABEL,
  FILTER_LABEL,
  HEADING_LABEL,
  INCIDENT_ACTION_LABEL,
  INCIDENT_ACTIONS,
  type IncidentAction,
  INCIDENT_LABEL,
  OBSERVATION_TEXT,
  PHASE_LABEL,
  START_LABEL,
  TRIGGER_LABEL,
  TRUST_LABEL,
  VERDICT_LABEL,
  parseContract,
  POLICY_LABEL,
  RESULT_LABEL,
  SOURCE_AUTH_LABEL,
  SOURCE_KIND_LABEL,
  SOURCE_NAME_PATTERN,
  SOURCE_STALE,
  STATUS_DOT,
  STATUS_LABEL,
  type SourceDraft,
  caseActions,
  caseDot,
  caseListQuery,
  contractTemplate,
  criterionChange,
  emptySourceDraft,
  evidenceListQuery,
  formatDelta,
  incidentActions,
  isActive,
  newCaseTitle,
  observationKeys,
  secretProblem,
  sourceBody,
  sourceDraft,
} from "../web/src/entities/observation/model.ts";
import type { CaseStatus } from "../web/src/shared/api/generated/CaseStatus.ts";
import type { Contract } from "../web/src/shared/api/generated/Contract.ts";
import type { IncidentStatus } from "../web/src/shared/api/generated/IncidentStatus.ts";
import type { SourceView } from "../web/src/shared/api/generated/SourceView.ts";
import { matchesAny } from "../web/src/shared/api/invalidation.ts";

/** The rows of the "Observation plane" section of the project glossary: key, the middle cell, and the label. */
function glossaryRows(): { key: string; middle: string; label: string }[] {
  const text = readFileSync(new URL("./fixtures/observation-labels.md", import.meta.url), "utf8");
  const start = text.indexOf("\n## Observation plane");
  assert.ok(start >= 0, "the label fixture has the observation section");
  const next = text.indexOf("\n## ", start + 1);
  const section = text.slice(start, next < 0 ? undefined : next);
  return section
    .split("\n")
    .filter((line) => line.startsWith("| `"))
    .map((line) => {
      const [key, middle, label] = line.split("|").slice(1, -1).map((cell) => cell.trim());
      return { key: key.replace(/^`|`$/g, ""), middle, label };
    });
}

/** The labels of a UI key whose rows name a value in code, such as `status` → `proposed`. */
function glossary(uiKey: string): Record<string, string> {
  const rows: Record<string, string> = {};
  for (const row of glossaryRows()) if (row.key === uiKey && row.middle.startsWith("`")) rows[row.middle.replace(/^`|`$/g, "")] = row.label;
  return rows;
}

/** The text of a UI key whose row has no code value, such as `nav` or `newTitle`. */
function glossaryText(uiKey: string): string {
  const row = glossaryRows().find((r) => r.key === uiKey && !r.middle.startsWith("`"));
  assert.ok(row, `the label fixture has a ${uiKey} row`);
  return row.label;
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

test("the task card and navigation texts are the glossary's words", () => {
  assert.equal(OBSERVATION_TEXT.nav, glossaryText("nav"));
  assert.equal(OBSERVATION_TEXT.section, glossaryText("section"));
  assert.equal(OBSERVATION_TEXT.empty, glossaryText("empty"));
  assert.equal(OBSERVATION_TEXT.create, glossaryText("create"));
});

test("a case created from a task takes the glossary's default title", () => {
  const template = glossaryText("newTitle");
  assert.equal(template, "`<task title>` — наблюдение");
  assert.equal(newCaseTitle("Оплата заказа"), template.replace("`<task title>`", "Оплата заказа"));
  assert.equal(newCaseTitle("Оплата заказа"), "Оплата заказа — наблюдение");
});

test("the page labels are the glossary's words, and cover every generated value", () => {
  assert.deepEqual(FILTER_LABEL, glossary("filter"));
  assert.deepEqual(Object.keys(FILTER_LABEL).sort(), ["active", "all", "concluded"]);
  assert.deepEqual(VERDICT_LABEL, glossary("verdict"));
  assert.deepEqual(Object.keys(VERDICT_LABEL).sort(), generatedUnion("Verdict"));
  assert.deepEqual(TRIGGER_LABEL, glossary("trigger"));
  assert.deepEqual(Object.keys(TRIGGER_LABEL).sort(), generatedUnion("EvalTrigger"));
  assert.deepEqual(CONFIDENCE_LABEL, glossary("confidence"));
  assert.deepEqual(Object.keys(CONFIDENCE_LABEL).sort(), generatedUnion("Confidence"));
  assert.deepEqual(CRITERION_STATE_LABEL, glossary("criterion"));
  assert.deepEqual(Object.keys(CRITERION_STATE_LABEL).sort(), generatedUnion("CriterionState"));
  assert.deepEqual(PHASE_LABEL, glossary("phase"));
  assert.deepEqual(Object.keys(PHASE_LABEL).sort(), generatedUnion("EvidencePhase"));
  assert.deepEqual(TRUST_LABEL, glossary("trust"));
  assert.deepEqual(Object.keys(TRUST_LABEL).sort(), generatedUnion("TrustLevel"));
  assert.deepEqual(START_LABEL, glossary("start"));
  assert.deepEqual(Object.keys(START_LABEL).sort(), generatedUnion("StartOn"));
});

test("the action, heading and field labels are the glossary's words, one per action", () => {
  assert.deepEqual(ACTION_LABEL, glossary("action"));
  assert.deepEqual(Object.keys(ACTION_LABEL).sort(), [...CASE_ACTIONS].sort());
  assert.deepEqual(INCIDENT_ACTION_LABEL, glossary("incidentAction"));
  assert.deepEqual(Object.keys(INCIDENT_ACTION_LABEL).sort(), [...INCIDENT_ACTIONS].sort());
  assert.deepEqual(HEADING_LABEL, glossary("heading"));
  assert.deepEqual(FIELD_LABEL, glossary("field"));
});

test("the evidence record is a measurement in Russian, as the glossary and the pages name it", () => {
  const row = readFileSync(new URL("./fixtures/observation-labels.md", import.meta.url), "utf8").split("\n").find((l) => l.startsWith("| **Evidence record**"));
  assert.ok(row, "the label fixture names the evidence record");
  assert.match(row, /Russian: измерение\.\s*\|$/);
  assert.equal(HEADING_LABEL.evidence, "Измерения");
  assert.equal(ACTION_LABEL.evidence, "Записать измерение");
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

test("incident actions follow the incident's status and the rights (§10.1, §10.2, §14)", () => {
  const table: [IncidentStatus, CaseRights, IncidentAction[]][] = [
    ["open", MANAGER, ["resolve", "dismiss", "followup"]],
    ["open", AGENT, ["resolve", "followup"]], // the project manager cannot dismiss
    ["open", VIEWER, []],
    ["resolved", MANAGER, ["dismiss", "followup"]], // a resolved violation is still dismissable
    ["resolved", AGENT, ["followup"]],
    ["resolved", VIEWER, []],
    ["dismissed", MANAGER, ["followup"]],
    ["dismissed", AGENT, ["followup"]],
    ["dismissed", TASK_AGENT, []],
  ];
  for (const [status, rights, expected] of table) {
    assert.deepEqual(incidentActions({ status }, rights), expected, `${status}, writable ${rights.writable}, human ${rights.human}`);
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

test("the change threshold picks the delta in its own unit", () => {
  const both = { deltaRel: -23.1234, deltaAbs: 0.1234 };
  assert.equal(formatDelta(both, "-20%"), "-23.1%", "% shows the relative change");
  assert.equal(formatDelta(both, "+0.1pp"), "+0.12pp", "pp shows the absolute change with its unit");
  assert.equal(formatDelta(both, "+5"), "+0.12", "no unit shows the absolute change, without a unit");
  assert.equal(formatDelta({ deltaRel: -10, deltaAbs: -0.04 }, "+0.1pp"), "-0.04pp");
  assert.equal(formatDelta({ deltaAbs: 0.5 }, "-20%"), "—", "a % threshold never shows the absolute change");
  assert.equal(formatDelta({ deltaRel: -10 }, "+0.1pp"), "—", "a pp threshold never shows the relative change");
  assert.equal(formatDelta({ deltaRel: -10 }, "+0"), "—", "no absolute change for a no-unit threshold");
});

test("without a threshold, or with text that is not one, the delta reads as before", () => {
  const both = { deltaRel: -23.1234, deltaAbs: 0.1234 };
  assert.equal(formatDelta(both), "-23.1%");
  assert.equal(formatDelta(both, undefined), "-23.1%");
  assert.equal(formatDelta(both, "twenty"), "-23.1%");
});

const CONTRACT: Contract = {
  hypotheses: [
    { id: "latency", source: "prom", query: "q", expect: { change: "-20%" } },
    { id: "floor", source: "prom", query: "q", expect: { min: 5 } },
  ],
  guardrails: [
    { id: "errors", source: "prom", query: "q", maxChange: "+0.1pp", minChange: "-0.2pp" },
    { id: "drop", source: "prom", query: "q", minChange: "-0.4%" },
    { id: "cap", source: "prom", query: "q", max: 50 },
  ],
  observation: { start: "tasks_done", minimumWindow: "24h" },
  followup: "manual",
};

test("a criterion's threshold is its expected change, else maxChange, else minChange (§8)", () => {
  assert.equal(criterionChange(CONTRACT, "latency"), "-20%");
  assert.equal(criterionChange(CONTRACT, "errors"), "+0.1pp", "maxChange before minChange");
  assert.equal(criterionChange(CONTRACT, "drop"), "-0.4%", "minChange when there is no maxChange");
  assert.equal(criterionChange(CONTRACT, "floor"), undefined, "a bound has no change");
  assert.equal(criterionChange(CONTRACT, "cap"), undefined);
  assert.equal(criterionChange(CONTRACT, "missing"), undefined);
});

test("a source change refetches the source list and every case view, not the other observation queries", () => {
  assert.ok(matchesAny(SOURCE_STALE, observationKeys.sources));
  assert.ok(matchesAny(SOURCE_STALE, observationKeys.case(7)), "arm problems depend on sources");
  assert.ok(!matchesAny(SOURCE_STALE, observationKeys.incidents()));
  assert.ok(!matchesAny(SOURCE_STALE, observationKeys.cases({})));
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

// --- contract text ----------------------------------------------------------------------------------

test("the editor sends a contract whose shape is an object with the three parts (§5.1)", () => {
  const text = contractTemplate("guardrails");
  const parsed = parseContract(text);
  assert.ok("contract" in parsed, "a template parses");
  assert.deepEqual(parsed.contract, JSON.parse(text));
  const minimal = '{"hypotheses":[],"guardrails":[],"observation":{"start":"manual","minimumWindow":"1h"}}';
  assert.deepEqual(parseContract(minimal), { contract: JSON.parse(minimal) });
});

test("text that is not JSON is refused with the reason the parser gives", () => {
  const parsed = parseContract('{"hypotheses": [');
  assert.ok("error" in parsed);
  assert.match(parsed.error, /^Контракт должен быть корректным JSON: /);
  assert.ok(parsed.error.length > "Контракт должен быть корректным JSON: ".length, "the parser's reason follows");
  const empty = parseContract("");
  assert.ok("error" in empty, "empty text is not JSON");
  assert.match(empty.error, /^Контракт должен быть корректным JSON: /);
});

test("JSON that is not an object with the three parts is refused with the shape message", () => {
  const shapes = [
    "null",
    "42",
    '"text"',
    "[]",
    "{}",
    '{"hypotheses":[],"guardrails":[]}',
    '{"hypotheses":{},"guardrails":[],"observation":{}}',
    '{"hypotheses":[],"guardrails":null,"observation":{}}',
    '{"hypotheses":[],"guardrails":[],"observation":[]}',
    '{"hypotheses":[],"guardrails":[],"observation":null}',
  ];
  assert.equal(CONTRACT_SHAPE_ERROR, "Контракт должен быть JSON-объектом с полями hypotheses, guardrails и observation.");
  for (const text of shapes) assert.deepEqual(parseContract(text), { error: CONTRACT_SHAPE_ERROR }, text);
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

// --- observation sources (§7.3, §13.2, §15) -------------------------------------------------------

/** The field names of a generated type, read from its one-line declaration (`export type X = { a: T, b?: U, };`). */
function generatedFields(name: string): string[] {
  const text = readFileSync(new URL(`../web/src/shared/api/generated/${name}.ts`, import.meta.url), "utf8");
  const line = text.split("\n").find((l) => l.startsWith(`export type ${name} = `));
  assert.ok(line, `${name} is a generated object type`);
  return [...line.matchAll(/(\w+)\??:/g)].map((m) => m[1]);
}

const SENTRY_VIEW: SourceView = {
  name: "sentry-main",
  kind: "sentry",
  url: "https://sentry.io",
  auth: "bearer",
  organization: "acme",
  sentryProject: "backend",
  environment: "production",
  field: "count()",
  dataset: "errors",
  timeoutSecs: 45,
  created: "2026-10-17T12:00:00.000Z",
  updated: "2026-10-17T12:00:00.000Z",
  secret: { hint: "…abcd", updated: "2026-10-17T12:00:00.000Z" },
};

test("the source kind and auth labels are the glossary's words", () => {
  assert.deepEqual(SOURCE_KIND_LABEL, glossary("kind"));
  assert.deepEqual(SOURCE_AUTH_LABEL, glossary("auth"));
});

test("the source labels cover every generated kind and auth, and no other", () => {
  assert.deepEqual(Object.keys(SOURCE_KIND_LABEL).sort(), generatedUnion("SourceKind"));
  assert.deepEqual(Object.keys(SOURCE_AUTH_LABEL).sort(), generatedUnion("SourceAuth"));
});

test("a Prometheus source sends no Sentry fields, even ones filled before the kind changed", () => {
  const body = sourceBody({
    ...emptySourceDraft(),
    name: "prom",
    url: "https://prom.example.com",
    organization: "acme",
    sentryProject: "backend",
    environment: "prod",
    field: "count()",
    dataset: "errors",
  });
  assert.deepEqual(body, { name: "prom", kind: "prometheus", url: "https://prom.example.com", auth: "none" });
});

test("blank and whitespace text fields are left out of the body, not sent as empty strings", () => {
  const body = sourceBody({
    ...emptySourceDraft(),
    name: " sentry-main ",
    kind: "sentry",
    url: " https://sentry.io ",
    organization: "acme",
    sentryProject: "   ",
    environment: "",
    field: "  ",
    dataset: "",
  });
  assert.deepEqual(body, { name: "sentry-main", kind: "sentry", url: "https://sentry.io", auth: "none", organization: "acme" });
});

test("the timeout is sent only when given: blank keeps the server default, \"45\" is the number 45", () => {
  const draft: SourceDraft = { ...emptySourceDraft(), name: "prom", url: "https://prom.example.com" };
  assert.equal("timeoutSecs" in sourceBody({ ...draft, timeoutSecs: "" }), false);
  assert.equal("timeoutSecs" in sourceBody({ ...draft, timeoutSecs: "   " }), false);
  assert.equal(sourceBody({ ...draft, timeoutSecs: "45" }).timeoutSecs, 45);
});

test("a stored Sentry source comes back from its draft unchanged, without its secret", () => {
  assert.deepEqual(sourceBody(sourceDraft(SENTRY_VIEW)), {
    name: "sentry-main",
    kind: "sentry",
    url: "https://sentry.io",
    auth: "bearer",
    organization: "acme",
    sentryProject: "backend",
    environment: "production",
    field: "count()",
    dataset: "errors",
    timeoutSecs: 45,
  });
});

test("a body names only fields of the generated SourceBody", () => {
  const fields = generatedFields("SourceBody");
  const full = sourceBody(sourceDraft(SENTRY_VIEW));
  const plain = sourceBody({ ...emptySourceDraft(), name: "prom", url: "https://prom.example.com", auth: "basic" });
  for (const body of [full, plain]) for (const key of Object.keys(body)) assert.ok(fields.includes(key), `${key} is a SourceBody field`);
  assert.equal(Object.keys(full).length, 10, "a full Sentry body has every field the form sends");
});

test("the body never carries the secret, in any shape", () => {
  const body = sourceBody({ ...emptySourceDraft(), name: "prom", url: "https://prom.example.com", auth: "basic", secret: "monitor:s3cret-value" });
  assert.equal("secret" in body, false);
  assert.equal(JSON.stringify(body).includes("s3cret-value"), false);
});

test("a source name follows the server's rule: 1 to 32 characters, lowercase letters, digits, dashes and underscores, not starting with a dash or underscore", () => {
  // Mirrors source_names_follow_the_pattern in crates/genie-core/src/observation/sources.rs.
  const pattern = new RegExp(`^(?:${SOURCE_NAME_PATTERN})$`);
  for (const name of ["prom", "2fast_1-x", "a".repeat(32)]) assert.ok(pattern.test(name), `${name} is accepted`);
  for (const name of ["a".repeat(33), "", "-prom", "_prom", "Prom", "pro m"]) assert.equal(pattern.test(name), false, `${name} is refused`);
});

test("a secret must fit its authorization: a token for bearer, login:password for basic, nothing for none", () => {
  assert.equal(secretProblem("none", ""), undefined);
  assert.equal(secretProblem("bearer", "glpat-x"), undefined);
  assert.match(secretProblem("bearer", "") ?? "", /токен/);
  assert.match(secretProblem("bearer", "   ") ?? "", /токен/);
  assert.equal(secretProblem("basic", "monitor:pass"), undefined);
  assert.match(secretProblem("basic", "monitor") ?? "", /двоеточие/);
  assert.match(secretProblem("basic", "") ?? "", /двоеточие/);
});

test("a concluded case's dot shows its result, and an open case's dot shows its status", () => {
  assert.equal(caseDot({ status: "concluded", result: "confirmed" }), "ok");
  assert.equal(caseDot({ status: "concluded", result: "negative" }), "failed");
  assert.equal(caseDot({ status: "concluded", result: "inconclusive" }), "off");
  assert.equal(caseDot({ status: "observing" }), STATUS_DOT.observing);
  assert.equal(caseDot({ status: "waiting" }), "waiting");
  assert.equal(caseDot({ status: "cancelled" }), "off");
  assert.equal(STATUS_DOT.concluded, "", "a concluded case takes its dot from its result");
});
