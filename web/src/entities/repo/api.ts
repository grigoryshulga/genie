import { useMutation, useQuery } from "@tanstack/react-query";
import { keysForAll, request, useInvalidating } from "@/shared/api";
import type { CheckLine, GitHostInfo, ProjectRepo, RepoPolicy, TaskRepo } from "./model.ts";

const enc = encodeURIComponent;

/** Repositories live in the server database: no journal event refetches these. */
const REPOS = [["repos"]];
/** Removing a repository also drops the tasks' deliveries in it. */
const REPOS_AND_TASKS = [["repos"], ["task-repos"]];
/** A merge appends `cr.merged` and a note on the task; the note is mailed to the team. */
const MERGE = keysForAll([{ type: "cr.merged" }, { type: "task.commented" }, { type: "mail.sent" }]);

export const useRepos = () => useQuery({ queryKey: ["repos"], queryFn: () => request<ProjectRepo[]>("GET", "/api/repos") });

/** The hosts of `git.json` (for admins; the answer is a 403 for everybody else). */
export const useGitHosts = (enabled: boolean) =>
  useQuery({
    queryKey: ["git-hosts"],
    queryFn: () => request<{ hosts: GitHostInfo[]; errors: string[] }>("GET", "/api/git/hosts"),
    enabled,
    retry: false,
  });

export interface NewRepoInput {
  name: string;
  host: string;
  remote: string;
  mount?: string;
  access?: "read" | "write";
  policy?: RepoPolicy;
  /** The access token (PAT) on the host; the server keeps it sealed. */
  token?: string;
}

export const useAddRepo = () => useInvalidating((r: NewRepoInput) => request<ProjectRepo & { warning?: string | null }>("POST", "/api/repos", r), REPOS);

export const usePatchRepo = () =>
  useInvalidating(
    ({ name, patch }: { name: string; patch: { mount?: string; defaultBranch?: string; access?: "read" | "write"; policy?: RepoPolicy; token?: string } }) =>
      request<ProjectRepo>("PATCH", `/api/repos/${enc(name)}`, patch),
    REPOS,
  );

export const useRemoveRepo = () => useInvalidating((name: string) => request("DELETE", `/api/repos/${enc(name)}`), REPOS_AND_TASKS);

export const useSyncRepo = () =>
  useInvalidating((name: string) => request<{ branches: number; defaultBranch?: string; warning?: string | null }>("POST", `/api/repos/${enc(name)}/sync`, {}), REPOS);

/** Check a repository against its host; `probe` pushes (and deletes) a throw-away branch. */
export const useCheckRepo = () =>
  useMutation({ mutationFn: ({ name, probe }: { name: string; probe: boolean }) => request<{ ok: boolean; lines: CheckLine[] }>("POST", `/api/repos/${enc(name)}/check${probe ? "?probe=1" : ""}`, {}) });

/** The repositories a task works in and how their delivery stands. */
export const useTaskRepos = (task: string) =>
  useQuery({ queryKey: ["task-repos", task], queryFn: () => request<{ task: string; repos: TaskRepo[] }>("GET", `/api/tasks/${enc(task)}/repos`), refetchInterval: 30_000 });

export const useSetTaskRepos = () =>
  useInvalidating(
    ({ task, repos }: { task: string; repos: { name: string; access: "read" | "write" }[] }) => request("PUT", `/api/tasks/${enc(task)}/repos`, { repos }),
    [["task-repos"]],
  );

/** Merge a task's request; `policy` holds the person to the repository's policy (the merge button of an agent's request). */
export const useMergeRequest = () =>
  useInvalidating(
    ({ task, repo, policy }: { task: string; repo: string; policy?: boolean }) =>
      request("POST", `/api/tasks/${enc(task)}/repos/${enc(repo)}/cr/merge`, { policy: !!policy }),
    MERGE,
  );
