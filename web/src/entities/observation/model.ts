// Pure helpers of the observation plane (docs/platform/observation-plane.md §14, §15, §17).
// Labels come from the "Observation plane" section of GLOSSARY.md; the wire types come from the
// generated files. This module loads in Node (the tests import it directly), so it uses only
// relative imports and type-only imports from the generated files.

import type { CaseResult } from "../../shared/api/generated/CaseResult.ts";
import type { CaseStatus } from "../../shared/api/generated/CaseStatus.ts";
import type { Contract } from "../../shared/api/generated/Contract.ts";
import type { Evidence } from "../../shared/api/generated/Evidence.ts";
import type { Incident } from "../../shared/api/generated/Incident.ts";
import type { IncidentStatus } from "../../shared/api/generated/IncidentStatus.ts";
import type { ObservationCase } from "../../shared/api/generated/ObservationCase.ts";
import type { ObservationPolicy } from "../../shared/api/generated/ObservationPolicy.ts";

export type { CaseResult, CaseStatus, Contract, Evidence, IncidentStatus, ObservationCase, ObservationPolicy };

// --- labels (GLOSSARY.md, "Observation plane") -----------------------------------------------------

export const STATUS_LABEL: Record<CaseStatus, string> = {
  proposed: "Черновик",
  waiting: "Ждёт поставки",
  observing: "Наблюдается",
  evaluating: "Оценивается",
  concluded: "Завершено",
  cancelled: "Отменено",
};

export const RESULT_LABEL: Record<CaseResult, string> = {
  confirmed: "Подтверждено",
  negative: "Не подтверждено",
  inconclusive: "Нет вывода",
};

export const POLICY_LABEL: Record<ObservationPolicy, string> = {
  guardrails: "Ограничители",
  verify: "Проверка результата",
};

export const INCIDENT_LABEL: Record<IncidentStatus, string> = {
  open: "Открыт",
  resolved: "Разрешён",
  dismissed: "Отклонён",
};

/** The labels of the task card section and the sidebar item, as GLOSSARY.md names them. */
export const OBSERVATION_TEXT = {
  nav: "Наблюдения",
  section: "Наблюдение",
  empty: "Наблюдение не настроено",
  create: "Создать наблюдение",
} as const;

/** The default title of a case created from a task (`newTitle` in GLOSSARY.md). */
export function newCaseTitle(taskTitle: string): string {
  return `${taskTitle} — наблюдение`;
}

// --- status and permissions (§6.1, §14) --------------------------------------------------------------

/** Not finished: the case may still be armed, observed, evaluated or cancelled. The list filter "active" uses it. */
export function isActive(status: CaseStatus): boolean {
  return status !== "concluded" && status !== "cancelled";
}

/**
 * What the caller may do in observation (§14). `writable`: a person with write access, a
 * project admin, or the project manager agent. `human`: the caller is a person.
 * A viewer has neither; a task manager, a member or a job has no observation rights at all.
 */
export interface CaseRights {
  writable: boolean;
  human: boolean;
}

/**
 * The case actions the page offers (§14, §6.2). The server enforces the same rules, so a
 * hidden action is also refused there.
 * - `rename`: the title (proposed, waiting, observing).
 * - `edit`: the policy and the linked tasks (proposed, waiting).
 * - `contract`: a new contract version before observation starts (proposed, waiting).
 * - `reviseContract`: a new version after it starts, with a reason; a person only.
 * - `arm`, `start`: proposed → waiting, waiting → observing.
 * - `deploy`: a deployment marker (waiting, observing, evaluating).
 * - `evaluate`: a manual evaluation request (observing); `evaluateFinal` is the final one, a person only.
 * - `evidence`: a manual measurement (observing, evaluating).
 * - `followup`: a follow-up task, in any status.
 * - `cancel`: any open case (not concluded or cancelled).
 */
export const CASE_ACTIONS = [
  "rename",
  "edit",
  "contract",
  "reviseContract",
  "arm",
  "start",
  "deploy",
  "evaluate",
  "evaluateFinal",
  "evidence",
  "followup",
  "cancel",
] as const;
export type CaseAction = (typeof CASE_ACTIONS)[number];

export function caseActions(c: Pick<ObservationCase, "status">, rights: CaseRights): CaseAction[] {
  const { writable, human } = rights;
  const s = c.status;
  const open = s === "proposed" || s === "waiting";
  const started = s === "observing" || s === "evaluating";
  const closed = s === "concluded" || s === "cancelled";
  const allowed: Record<CaseAction, boolean> = {
    rename: writable && (open || s === "observing"),
    edit: writable && open,
    contract: writable && open,
    reviseContract: writable && human && started,
    arm: writable && s === "proposed",
    start: writable && s === "waiting",
    deploy: writable && (s === "waiting" || started),
    evaluate: writable && s === "observing",
    evaluateFinal: writable && human && s === "observing",
    evidence: writable && started,
    followup: writable,
    cancel: writable && !closed,
  };
  return CASE_ACTIONS.filter((action) => allowed[action]);
}

/**
 * The actions on one incident (§10.1, §10.2, §14). `resolve` needs an open incident; `dismiss` is a
 * person's call and refuses a dismissed one; `followup` files a task for the incident in any status.
 */
export const INCIDENT_ACTIONS = ["resolve", "dismiss", "followup"] as const;
export type IncidentAction = (typeof INCIDENT_ACTIONS)[number];

export function incidentActions(i: Pick<Incident, "status">, rights: CaseRights): IncidentAction[] {
  const { writable, human } = rights;
  const allowed: Record<IncidentAction, boolean> = {
    resolve: writable && i.status === "open",
    dismiss: writable && human && i.status !== "dismissed",
    followup: writable,
  };
  return INCIDENT_ACTIONS.filter((action) => allowed[action]);
}

// --- evidence --------------------------------------------------------------------------------------

/**
 * The change threshold a criterion is judged against (§8): a hypothesis's `expect.change`, else a
 * guardrail's `maxChange`, else its `minChange`. Undefined for a bound-only criterion or an unknown id.
 */
export function criterionChange(contract: Contract, id: string): string | undefined {
  const hypothesis = contract.hypotheses.find((h) => h.id === id);
  if (hypothesis) return hypothesis.expect.change;
  const guardrail = contract.guardrails.find((g) => g.id === id);
  return guardrail?.maxChange ?? guardrail?.minChange;
}

/**
 * The unit of a change threshold (§5.2): `%` relative, `pp` absolute, `""` absolute without a unit;
 * undefined when the text is not a change.
 */
function changeUnit(change: string): "%" | "pp" | "" | undefined {
  const match = change.match(/^\s*[+-]\d+(?:\.\d+)?(%|pp)?\s*$/);
  return match ? (match[1] as "%" | "pp" | undefined) ?? "" : undefined;
}

/**
 * A change for display, in the unit of the threshold it is judged against: `%` shows the relative
 * change (`-23.1%`), `pp` the absolute change with its unit (`+0.12pp`), no unit the absolute change
 * (`+0.05`). A missing value is an em dash. Without a threshold, or with one that is not a change,
 * the relative change is shown when the record has one, else the absolute one. `0.0%` has no sign.
 */
export function formatDelta(e: Pick<Evidence, "deltaAbs" | "deltaRel">, change?: string): string {
  const unit = change === undefined ? undefined : changeUnit(change);
  if (unit === "%") return isFiniteNumber(e.deltaRel) ? `${signed(e.deltaRel, 1)}%` : "—";
  if (unit === "pp") return isFiniteNumber(e.deltaAbs) ? `${signed(e.deltaAbs, 2)}pp` : "—";
  if (unit === "") return isFiniteNumber(e.deltaAbs) ? signed(e.deltaAbs, 2) : "—";
  if (isFiniteNumber(e.deltaRel)) return `${signed(e.deltaRel, 1)}%`;
  if (isFiniteNumber(e.deltaAbs)) return signed(e.deltaAbs, 2);
  return "—";
}

function isFiniteNumber(n: number | undefined | null): n is number {
  return typeof n === "number" && Number.isFinite(n);
}

/** Rounds to `digits` decimals; a sign only for a non-zero result, so `-0.04` reads `0.0`. */
function signed(n: number, digits: number): string {
  const rounded = Number(n.toFixed(digits));
  if (rounded === 0) return (0).toFixed(digits);
  return `${rounded > 0 ? "+" : ""}${rounded.toFixed(digits)}`;
}

// --- contract templates (§5.1) ----------------------------------------------------------------------

// Valid examples per policy (§5.3): `verify` has hypotheses only, `guardrails` has guardrails only.
// The source name `prom` must exist in the project (§6.2) before the case can be armed.
const TEMPLATES: Record<ObservationPolicy, Contract> = {
  verify: {
    hypotheses: [
      {
        id: "checkout_latency_p95",
        source: "prom",
        query: 'histogram_quantile(0.95, sum(rate(http_request_duration_seconds_bucket{route="/checkout"}[$__range])) by (le))',
        expect: { change: "-20%" },
      },
    ],
    guardrails: [],
    observation: { start: "tasks_done", baselineWindow: "7d", minimumWindow: "24h", preferredWindow: "72h" },
    followup: "manual",
  },
  guardrails: {
    hypotheses: [],
    guardrails: [
      {
        id: "error_rate",
        source: "prom",
        query: '100 * sum(rate(http_requests_total{code=~"5.."}[$__range])) / sum(rate(http_requests_total[$__range]))',
        maxChange: "+0.1pp",
      },
    ],
    observation: { start: "tasks_done", baselineWindow: "7d", minimumWindow: "24h", guardrailInterval: "15m", guardrailWindow: "1h" },
    followup: "manual",
  },
};

/** An example contract for a policy, as pretty JSON to start from in the contract editor. */
export function contractTemplate(policy: ObservationPolicy): string {
  return JSON.stringify(TEMPLATES[policy], null, 2);
}

// --- list queries and keys ---------------------------------------------------------------------------

export interface CaseListFilter {
  status?: CaseStatus;
  task?: string;
  limit?: number;
}

export interface EvidenceListFilter {
  after?: number;
  limit?: number;
  criterion?: string;
}

/** A query string with the given values; absent and blank values are left out. */
export function listQuery(params: Record<string, string | number | undefined>): string {
  const search = new URLSearchParams();
  for (const [name, value] of Object.entries(params)) {
    const text = value === undefined ? "" : String(value).trim();
    if (text) search.set(name, text);
  }
  const text = search.toString();
  return text ? `?${text}` : "";
}

export const caseListQuery = (f: CaseListFilter) => listQuery({ status: f.status, task: f.task, limit: f.limit });
export const evidenceListQuery = (f: EvidenceListFilter) => listQuery({ after: f.after, limit: f.limit, criterion: f.criterion });

/**
 * Query keys. Every observation key starts with `observations`, which the journal mapping
 * refetches (shared/api/invalidation.ts). Sources have their own root: they change through
 * their settings forms, not through journal events.
 */
export const observationKeys = {
  root: ["observations"] as const,
  cases: (filter: CaseListFilter = {}) => ["observations", "cases", filter] as const,
  /** Every case view: a case's key prefix, which covers all of its queries. */
  caseRoot: ["observations", "case"] as const,
  case: (id: number | undefined) => ["observations", "case", id] as const,
  evidence: (id: number | undefined, filter: EvidenceListFilter = {}) => ["observations", "evidence", id, filter] as const,
  incidents: (status?: IncidentStatus) => ["observations", "incidents", status ?? "all"] as const,
  task: (task: string) => ["observations", "task", task] as const,
  sources: ["observation-sources"] as const,
};

/**
 * What a change to a source makes stale: the source list and every case view, because a case's
 * arm problems depend on the project's sources (§6.2).
 */
export const SOURCE_STALE: readonly (readonly string[])[] = [observationKeys.sources, observationKeys.caseRoot];
