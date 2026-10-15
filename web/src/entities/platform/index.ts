// Platform features of the Rust server: notifications, automations, proposals, profile.
export * from "./model.ts";
import { useQuery } from "@tanstack/react-query";
import { request } from "@/shared/api";

export interface Notification {
  id: number;
  project?: string;
  task?: string;
  kind: string;
  title: string;
  body: string;
  link?: string;
  created: string;
  readAt?: string;
}

export interface Automation {
  id: number;
  name: string;
  enabled: boolean;
  dryRun: boolean;
  version: number;
  spec: Record<string, unknown>;
  trigger: string;
  createdBy: string;
  updated: string;
  lastRun?: Run | null;
}

export interface Run {
  id: number;
  automation: number;
  status: string;
  started: string;
  finished?: string;
  error?: string;
  depth: number;
  triggerKey: string;
}

export interface RunStep {
  id: number;
  stepId: string;
  kind: string;
  status: string;
  attempt: number;
  input: unknown;
  output?: unknown;
  wait?: unknown;
  error?: string;
}

export interface Playbook {
  id: string;
  title: string;
  spec: Record<string, unknown>;
}

export interface Proposal {
  id: number;
  path: string;
  author: string;
  authorKind: string;
  task?: string;
  note: string;
  status: string;
  created: string;
  content?: string;
}

export const useNotifications = () =>
  useQuery({ queryKey: ["notifications"], queryFn: () => request<{ items: Notification[]; unread: number }>("GET", "/api/notifications"), refetchInterval: 30_000 });

export const useAutomations = () => useQuery({ queryKey: ["automations"], queryFn: () => request<Automation[]>("GET", "/api/automations") });

export const usePlaybooks = () => useQuery({ queryKey: ["playbooks"], queryFn: () => request<Playbook[]>("GET", "/api/automations/playbooks") });

export const useRuns = (automation?: number) =>
  useQuery({ queryKey: ["runs", automation ?? 0], queryFn: () => request<Run[]>("GET", `/api/runs${automation ? `?automation=${automation}` : ""}`), refetchInterval: 5000 });

/** Statuses a run does not leave (`set_run_status` stamps `finished` for them). */
const RUN_ENDED = new Set(["succeeded", "failed", "cancelled", "skipped"]);

export const useRun = (id?: number) =>
  useQuery({
    queryKey: ["run", id ?? 0],
    queryFn: () => request<Run & { steps: RunStep[] }>("GET", `/api/runs/${id}`),
    enabled: !!id,
    // Runs live in server.db, outside the journal the live stream reads: poll while one is going, never after it ended.
    refetchInterval: (q) => (RUN_ENDED.has(q.state.data?.status ?? "") ? false : 4000),
  });

export const useProposals = (status = "open") =>
  useQuery({ queryKey: ["proposals", status], queryFn: () => request<Proposal[]>("GET", `/api/docs/proposals?status=${status}`), refetchInterval: 30_000 });

export const useProposal = (id?: number) =>
  useQuery({
    queryKey: ["proposal", id ?? 0],
    queryFn: () => request<{ proposal: Proposal; current?: string; owners: string[] }>("GET", `/api/docs/proposals/${id}`),
    enabled: !!id,
  });

export const useChannels = () =>
  useQuery({ queryKey: ["channels"], queryFn: () => request<{ links: { channel: string; address: string }[]; telegram: boolean; email: boolean }>("GET", "/api/me/channels") });

/** A provider whose API key a person may keep: pi reads it from `keyVar`. */
export interface LlmProvider {
  id: string;
  title: string;
  keyVar: string;
  /** The key is owed: an agent on this provider does not start without it. */
  strict: boolean;
}

/** A person's key of a provider as the server shows it: never the value itself. */
export interface LlmKeyInfo {
  hint: string;
  updated: string;
  unreadable?: boolean;
}

/** The keys a person's agents use: their own, and the shared ones that stand in where they keep none. */
export const useLlmKeys = () =>
  useQuery({
    queryKey: ["llm-keys"],
    queryFn: () => request<{ providers: LlmProvider[]; keys: Record<string, LlmKeyInfo>; global: Record<string, LlmKeyInfo> }>("GET", "/api/me/llm-keys"),
  });

/** The provider keys the server shares with everyone, for its administrators. */
export const useSharedLlmKeys = () =>
  useQuery({
    queryKey: ["llm-keys", "shared"],
    queryFn: () => request<{ providers: LlmProvider[]; keys: Record<string, LlmKeyInfo> }>("GET", "/api/llm-keys"),
  });

export const RUN_STATUS: Record<string, string> = {
  queued: "в очереди",
  running: "выполняется",
  waiting: "ждёт",
  succeeded: "успешно",
  failed: "ошибка",
  cancelled: "отменён",
  skipped: "пропущен",
  pending: "ожидает",
};

