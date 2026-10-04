// Repositories of a project: where they live, what agents may do in them, how a task's delivery stands.

export type PushMode = "none" | "pr_only" | "branches" | "direct";
export type MergeMode = "human" | "agent_after_approval" | "auto";

/** The repository's policy as stored; every field is optional (the defaults fill the rest). */
export interface RepoPolicy {
  read?: boolean;
  push?: PushMode;
  branch?: string;
  branches?: string[];
  protected?: string[];
  force_push?: boolean;
  delete_branches?: boolean;
  change_request?: { open?: boolean; base?: string[]; merge?: MergeMode; method?: string | null; require_ci?: boolean; approvals?: number };
}

export interface RepoHost {
  id: string;
  kind?: string;
  url?: string;
  webUrl?: string;
  problems?: string[];
  error?: string;
}

/** The repository's own access token as the server shows it: never the value. */
export interface RepoToken {
  set: boolean;
  /** The last characters (`…a1b2`). */
  hint?: string;
  updated?: string;
  /** The server cannot read it back (its key changed): enter it again. */
  unreadable?: boolean;
}

export interface ProjectRepo {
  project: string;
  name: string;
  host: RepoHost;
  remote: string;
  mount: string;
  defaultBranch: string;
  access: "read" | "write";
  policy: RepoPolicy;
  policyValid: boolean;
  /** Absent for agents. */
  token?: RepoToken;
  created: string;
}

/** A host of `git.json` as the server describes it (never a secret). */
export interface GitHostInfo {
  id: string;
  kind: string;
  url: string;
  apiUrl?: string;
  transport: string;
  problems: string[];
}

export interface CheckLine {
  level: "ok" | "warn" | "fail";
  text: string;
}

export interface Preset {
  id: string;
  name: string;
  hint: string;
  policy: RepoPolicy;
  /** Shown in red: this loosens what protects the default branch. */
  risky?: boolean;
}

/** The usual policies, so that nobody writes JSON to get the common cases. */
export const PRESETS: Preset[] = [
  { id: "read", name: "Только чтение", hint: "Агенты читают код и ничего не отправляют.", policy: { push: "none" } },
  {
    id: "pr-human",
    name: "Запрос на слияние, сливает человек",
    hint: "Агент отправляет только ветку своей задачи и открывает запрос на слияние. Сливает человек.",
    policy: {},
  },
  {
    id: "pr-agent",
    name: "Запрос на слияние, после ревью сливает агент",
    hint: "Агент сливает сам, когда ревьюер одобрил задачу, проверки зелёные и хостинг разрешает.",
    policy: { change_request: { merge: "agent_after_approval" } },
  },
  {
    id: "pr-auto",
    name: "Запрос на слияние, сервер сливает сам",
    hint: "Сервер сливает, как только задача одобрена и условия хостинга выполнены.",
    policy: { change_request: { merge: "auto" } },
  },
  {
    id: "direct",
    name: "Прямая отправка в ветки",
    hint: "Агент отправляет в ветку задачи без запроса на слияние. Защищённые ветки остаются защищёнными.",
    policy: { push: "branches" },
  },
  {
    id: "direct-main",
    name: "Прямая отправка, в том числе в основную ветку",
    hint: "Осторожно: основную ветку защищают только настройки самого хостинга.",
    policy: { push: "direct", protected: [] },
    risky: true,
  },
];

const same = (a: unknown, b: unknown) => JSON.stringify(a ?? null) === JSON.stringify(b ?? null);

/** Which preset a stored policy is exactly (`custom` when it is none of them). */
export function presetOf(policy: RepoPolicy | undefined): string {
  const p = policy ?? {};
  return PRESETS.find((x) => same(x.policy, p))?.id ?? "custom";
}

export function presetName(id: string): string {
  return PRESETS.find((p) => p.id === id)?.name ?? "Своё правило (JSON)";
}

/** What a pasted repository link names: a host of `git.json` and the path on it. */
export type RepoLink =
  | { kind: "empty" }
  | { kind: "ok"; host: string; remote: string }
  /** A bare `group/repo`: fine when the server has one host, otherwise the host is asked. */
  | { kind: "path"; remote: string }
  | { kind: "unknown"; hostname: string }
  | { kind: "bad" };

const cleanPath = (path: string) =>
  path
    .split(/[?#]/)[0]
    .replace(/\/-(\/.*)?$/, "") // GitLab's pages inside a repository: /-/tree/main, /-/merge_requests…
    .replace(/^\/+|\/+$/g, "")
    .replace(/\.git$/, "");

const validPath = (p: string) => /^[\w.-]+(\/[\w.-]+)+$/.test(p) && !p.split("/").includes("..");

/**
 * Read a link as people copy it: the repository's page in a browser (any page inside it),
 * the https clone address, `git@host:group/repo.git` or `ssh://git@host/group/repo`.
 */
export function parseRepoLink(text: string, hosts: { id: string; kind?: string; url: string }[]): RepoLink {
  const t = text.trim();
  if (!t) return { kind: "empty" };
  let hostname = "";
  let path = "";
  const ssh = t.match(/^ssh:\/\/[\w.-]+@([^:/\s]+)(?::\d+)?\/(.+)$/) ?? t.match(/^[\w.-]+@([^:/\s]+):(.+)$/);
  if (ssh) {
    hostname = ssh[1];
    path = ssh[2];
  } else if (/^https?:\/\//i.test(t)) {
    try {
      const u = new URL(t);
      hostname = u.hostname;
      path = u.pathname;
    } catch {
      return { kind: "bad" };
    }
  } else {
    const p = cleanPath(t);
    return validPath(p) ? { kind: "path", remote: p } : { kind: "bad" };
  }
  hostname = hostname.toLowerCase();
  for (const h of hosts) {
    let base: URL;
    try {
      base = new URL(h.url);
    } catch {
      continue;
    }
    if (base.hostname.toLowerCase() !== hostname) continue;
    // A host served under a path (https://example.com/gitlab) keeps that prefix out of the repository's path.
    const prefix = base.pathname.replace(/\/+$/, "");
    const rest = ssh || !prefix || !path.startsWith(`${prefix}/`) ? path : path.slice(prefix.length);
    let remote = cleanPath(rest);
    // On GitHub a repository is always owner/name: whatever follows is a page inside it.
    if (h.kind === "github") remote = remote.split("/").slice(0, 2).join("/");
    return validPath(remote) ? { kind: "ok", host: h.id, remote } : { kind: "bad" };
  }
  return { kind: "unknown", hostname };
}

/** A name for the project from the repository's path: its last part, as names are allowed. */
export function repoNameFrom(remote: string): string {
  const last = remote.split("/").pop() ?? "";
  return last
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^[-_]+|-+$/g, "");
}

/** A task's delivery in one repository (`task_repos`), as the server's type. */
export type { TaskRepo } from "../../shared/api/types.ts";

export const CI_NAME: Record<string, string> = {
  none: "проверок нет",
  pending: "проверки идут",
  passed: "проверки прошли",
  failed: "проверки упали",
  stalled: "проверки зависли",
};
export const CR_NAME: Record<string, string> = { open: "открыт", merged: "слит", closed: "закрыт без слияния" };
