import { useQuery } from "@tanstack/react-query";
import { type KeyPrefix, keys, request, useInvalidating } from "@/shared/api";
import type { CaseView } from "@/shared/api/generated/CaseView.ts";
import type { ContractBody } from "@/shared/api/generated/ContractBody.ts";
import type { CreateCaseBody } from "@/shared/api/generated/CreateCaseBody.ts";
import type { DeploymentBody } from "@/shared/api/generated/DeploymentBody.ts";
import type { Evidence } from "@/shared/api/generated/Evidence.ts";
import type { EvidenceBody } from "@/shared/api/generated/EvidenceBody.ts";
import type { EvaluateBody } from "@/shared/api/generated/EvaluateBody.ts";
import type { FollowupBody } from "@/shared/api/generated/FollowupBody.ts";
import type { Incident } from "@/shared/api/generated/Incident.ts";
import type { IncidentStatus } from "@/shared/api/generated/IncidentStatus.ts";
import type { NoteBody } from "@/shared/api/generated/NoteBody.ts";
import type { ObservationCase } from "@/shared/api/generated/ObservationCase.ts";
import type { ReasonBody } from "@/shared/api/generated/ReasonBody.ts";
import type { SecretBody } from "@/shared/api/generated/SecretBody.ts";
import type { SourceBody } from "@/shared/api/generated/SourceBody.ts";
import type { SourceTestBody } from "@/shared/api/generated/SourceTestBody.ts";
import type { SourceTestView } from "@/shared/api/generated/SourceTestView.ts";
import type { SourceView } from "@/shared/api/generated/SourceView.ts";
import type { UpdateCaseBody } from "@/shared/api/generated/UpdateCaseBody.ts";
import {
  type CaseListFilter,
  type EvidenceListFilter,
  SOURCE_STALE,
  caseListQuery,
  evidenceListQuery,
  listQuery,
  observationKeys,
} from "./model.ts";

/** Every observation query: the journal mapping refetches the same root (shared/api/invalidation.ts). */
const OBSERVATIONS: KeyPrefix[] = [observationKeys.root];

const casePath = (id: number, rest = "") => `/api/observation/cases/${id}${rest}`;
const sourcePath = (name: string, rest = "") => `/api/observation/sources/${encodeURIComponent(name)}${rest}`;

// --- queries ----------------------------------------------------------------------------------------

/** Cases of the project, newest first; `status`, `task` and `limit` filter them on the server (§15). */
export const useObservationCases = (filter: CaseListFilter = {}) =>
  useQuery({
    queryKey: observationKeys.cases(filter),
    queryFn: () => request<ObservationCase[]>("GET", `/api/observation/cases${caseListQuery(filter)}`),
  });

/** A case with everything its page shows. `id` is the number of its key (`O-7` is 7). */
export const useObservationCase = (id: number | undefined) =>
  useQuery({
    queryKey: observationKeys.case(id),
    queryFn: () => request<CaseView>("GET", casePath(id!)),
    enabled: id !== undefined,
  });

/** Measurements of a case, oldest first. */
export const useObservationEvidence = (id: number | undefined, filter: EvidenceListFilter = {}) =>
  useQuery({
    queryKey: observationKeys.evidence(id, filter),
    queryFn: () => request<Evidence[]>("GET", `${casePath(id!, "/evidence")}${evidenceListQuery(filter)}`),
    enabled: id !== undefined,
  });

/** Incidents of every case of the project, newest first, optionally of one status. */
export const useObservationIncidents = (status?: IncidentStatus) =>
  useQuery({
    queryKey: observationKeys.incidents(status),
    queryFn: () => request<Incident[]>("GET", `/api/observation/incidents${listQuery({ status })}`),
  });

/** The cases linked to one task (the task card section). */
export const useTaskObservations = (task: string | undefined) =>
  useQuery({
    queryKey: observationKeys.task(task ?? ""),
    queryFn: () => request<ObservationCase[]>("GET", `/api/observation/tasks/${encodeURIComponent(task!)}`),
    enabled: !!task,
  });

/**
 * Observation sources of the project. Admins only: the server answers others with 403, so a page
 * that is not for admins passes `enabled = false` (as `useUsers` does).
 */
export const useObservationSources = (enabled = true) =>
  useQuery({ queryKey: observationKeys.sources, queryFn: () => request<SourceView[]>("GET", "/api/observation/sources"), enabled });

// --- case mutations -------------------------------------------------------------------------------

export const useCreateCase = () =>
  useInvalidating((body: CreateCaseBody) => request<CaseView>("POST", "/api/observation/cases", body), OBSERVATIONS);

export const useUpdateCase = () =>
  useInvalidating(({ id, ...body }: { id: number } & UpdateCaseBody) => request<CaseView>("PATCH", casePath(id), body), OBSERVATIONS);

export const useSetContract = () =>
  useInvalidating(({ id, ...body }: { id: number } & ContractBody) => request<CaseView>("PUT", casePath(id, "/contract"), body), OBSERVATIONS);

export const useArmCase = () =>
  useInvalidating((id: number) => request<CaseView>("POST", casePath(id, "/arm")), OBSERVATIONS);

export const useStartCase = () =>
  useInvalidating((id: number) => request<CaseView>("POST", casePath(id, "/start")), OBSERVATIONS);

/** A manual evaluation request, or a final one with `final: true` (a person only, §6.4). */
export const useEvaluateCase = () =>
  useInvalidating(({ id, ...body }: { id: number } & EvaluateBody) => request<CaseView>("POST", casePath(id, "/evaluate"), body), OBSERVATIONS);

export const useCancelCase = () =>
  useInvalidating(({ id, ...body }: { id: number } & ReasonBody) => request<CaseView>("POST", casePath(id, "/cancel"), body), OBSERVATIONS);

export const useRecordDeployment = () =>
  useInvalidating(({ id, ...body }: { id: number } & DeploymentBody) => request<CaseView>("POST", casePath(id, "/deployments"), body), OBSERVATIONS);

export const useReportEvidence = () =>
  useInvalidating(({ id, ...body }: { id: number } & EvidenceBody) => request<CaseView>("POST", casePath(id, "/evidence"), body), OBSERVATIONS);

/** A follow-up task goes through the task services, so the task lists and cards refetch too (§10.2). */
export const useFileFollowup = () =>
  useInvalidating(
    ({ id, ...body }: { id: number } & FollowupBody) => request<CaseView>("POST", casePath(id, "/followup"), body),
    [...OBSERVATIONS, keys.tasks, ["task"]],
  );

// --- incident mutations ---------------------------------------------------------------------------

export const useResolveIncident = () =>
  useInvalidating(
    ({ id, ...body }: { id: number } & NoteBody) => request<Incident>("POST", `/api/observation/incidents/${id}/resolve`, body),
    OBSERVATIONS,
  );

/** Dismissing is a person's call and needs a note (§10.1); the server refuses the rest. */
export const useDismissIncident = () =>
  useInvalidating(
    ({ id, ...body }: { id: number } & NoteBody) => request<Incident>("POST", `/api/observation/incidents/${id}/dismiss`, body),
    OBSERVATIONS,
  );

// --- source mutations (project admins, §14, §15) ----------------------------------------------------

export const useAddSource = () =>
  useInvalidating((body: SourceBody) => request<SourceView>("POST", "/api/observation/sources", body), SOURCE_STALE);

/** The path name is the body's name: the server refuses a body that names another source. */
export const useUpdateSource = () =>
  useInvalidating((body: SourceBody) => request<SourceView>("PUT", sourcePath(body.name), body), SOURCE_STALE);

export const useRemoveSource = () =>
  useInvalidating((name: string) => request<{ ok: boolean }>("DELETE", sourcePath(name)), SOURCE_STALE);

/** Stores the secret; the answer carries only its hint. A blank value removes it. */
export const useSetSourceSecret = () =>
  useInvalidating(({ name, ...body }: { name: string } & SecretBody) => request<SourceView>("PUT", sourcePath(name, "/secret"), body), SOURCE_STALE);

export const useClearSourceSecret = () =>
  useInvalidating((name: string) => request<SourceView>("DELETE", sourcePath(name, "/secret")), SOURCE_STALE);

/** A test query changes no stored state, so it refetches nothing. */
export const useTestSource = () =>
  useInvalidating(
    ({ name, ...body }: { name: string } & SourceTestBody) => request<SourceTestView>("POST", sourcePath(name, "/test"), body),
    [],
  );
