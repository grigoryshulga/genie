import { useQuery } from "@tanstack/react-query";
import { keys, request, upload, useInvalidating } from "@/shared/api";
import type { DoctorCheck, Meta, Membership, Person, ProjectInfo, ProjectRole, ProjectStats, VaultSync } from "./model.ts";

export const useMeta = () => useQuery({ queryKey: keys.meta, queryFn: () => request<Meta>("GET", "/api/meta") });

const at = (slug: string) => `/api/projects/${encodeURIComponent(slug)}`;

/** The projects list and the session's copy of it (name, autonomy, the caller's role):
 * server-database rows, so no journal event refetches them. */
const PROJECTS = [["projects"], ["session"]];
/** A member's role shows in the roster and in the session's own role. */
const MEMBERS = [["members"], ["session"]];
/** People and their photos appear in the users list, in project rosters and in the session. */
const PEOPLE = [["users"], ["members"], ["session"]];

export const useProjects = () => useQuery({ queryKey: ["projects"], queryFn: () => request<ProjectInfo[]>("GET", "/api/projects") });

export const useMembers = (slug?: string) =>
  useQuery({ queryKey: ["members", slug ?? ""], queryFn: () => request<Membership[]>("GET", `${at(slug!)}/members`), enabled: !!slug, staleTime: 60_000 });

export const useUsers = (enabled = true) => useQuery({ queryKey: ["users"], queryFn: () => request<Person[]>("GET", "/api/users"), enabled });

export const usePatchProject = () =>
  useInvalidating(
    ({ slug, patch }: { slug: string; patch: Partial<Pick<ProjectInfo, "name" | "autonomy" | "integration">> }) =>
      request<ProjectInfo>("PATCH", at(slug), patch),
    PROJECTS,
  );

export const useCreateProject = () =>
  useInvalidating((p: { slug: string; name: string; repo?: string; prefix?: string }) => request<ProjectInfo>("POST", "/api/projects", p), PROJECTS);

export const useSetMember = () =>
  useInvalidating(
    ({ slug, user, role }: { slug: string; user: number; role: ProjectRole }) => request("PUT", `${at(slug)}/members/${user}`, { role }),
    MEMBERS,
  );

export const useRemoveMember = () =>
  useInvalidating(({ slug, user }: { slug: string; user: number }) => request("DELETE", `${at(slug)}/members/${user}`), MEMBERS);

// An invite is a row no query reads (the response carries the link), so the broad refetch stays.
export const useInvite = () =>
  useInvalidating(({ slug, role, email }: { slug: string; role: ProjectRole; email?: string }) =>
    request<{ token: string; url: string }>("POST", `${at(slug)}/invites`, { role, email: email || undefined }),
  );

export const useCreateUser = () =>
  useInvalidating(
    (u: { login: string; name: string; email?: string; password?: string; isAdmin: boolean }) => request<Person>("POST", "/api/users", u),
    // Creating the first user switches the server from local mode to accounts: the session changes too.
    [["users"], ["session"]],
  );

export const usePatchUser = () =>
  useInvalidating(
    ({ id, patch }: { id: number; patch: Partial<Pick<Person, "login" | "name" | "isAdmin" | "disabled">> & { email?: string | null } }) =>
      request<Person>("PATCH", `/api/users/${id}`, patch),
    PEOPLE,
  );

/** A person's photo: an image already shrunk to a small square, or none to remove it. */
export const useSetAvatar = () =>
  useInvalidating(
    ({ id, image }: { id: number; image: Blob | null }) =>
      image ? upload<Person>("PUT", `/api/users/${id}/avatar`, image) : request<Person>("DELETE", `/api/users/${id}/avatar`),
    PEOPLE,
  );

/** A personal token as its owner sees it: never the secret. */
export interface UserToken {
  id: number;
  label: string;
  created: string;
  lastUsed?: string;
}

export const useTokens = (enabled: boolean) =>
  useQuery({ queryKey: ["tokens"], queryFn: () => request<UserToken[]>("GET", "/api/auth/tokens"), enabled });

export const useIssueToken = () =>
  useInvalidating((label: string) => request<{ token: string }>("POST", "/api/auth/tokens", { label }), [["tokens"]]);

export const useRevokeToken = () => useInvalidating((id: number) => request("DELETE", `/api/auth/tokens/${id}`), [["tokens"]]);

/** The server's preflight, for its admins (runs pi and git on the server: a second or so). */
export const useDoctor = (enabled: boolean) =>
  useQuery({ queryKey: ["doctor"], queryFn: () => request<{ checks: DoctorCheck[] }>("GET", "/api/doctor"), enabled, staleTime: 60_000 });

export const useVaultSync = (enabled: boolean) =>
  useQuery({ queryKey: ["vault-sync"], queryFn: () => request<VaultSync>("GET", "/api/vault/sync"), enabled, refetchInterval: 30_000 });

export const useSyncVaultNow = () =>
  useInvalidating(() => request<Pick<VaultSync, "last">>("POST", "/api/vault/sync"), [["vault-sync"]]);

export const useStats = (days: number) =>
  useQuery({ queryKey: ["stats", days], queryFn: () => request<{ days: number; since: string; projects: ProjectStats[] }>("GET", `/api/stats?days=${days}`) });

/** The model prices in effect: where each model's price comes from and when LiteLLM's were received. */
export interface ModelPrices {
  models: { model: string; source: string; price: { input: number; output: number; cacheRead: number | null; cacheWrite: number | null } }[];
  fetchedAt: string | null;
  lastError: string | null;
}

export const useModelPrices = () =>
  useQuery({ queryKey: ["model-prices"], queryFn: () => request<ModelPrices>("GET", "/api/model-prices") });

/** «Pull the config»: ask LiteLLM for the tariffs again; the stats reprice with what comes back. */
export const useRefreshModelPrices = () =>
  useInvalidating(() => request<{ received: number; priced: number; fetchedAt: string }>("POST", "/api/model-prices"), [["model-prices"], ["stats"]]);
