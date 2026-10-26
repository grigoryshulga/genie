import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ApiError, request } from "@/shared/api";

export interface SessionUser {
  id: number;
  login: string;
  name: string;
  email?: string;
  isAdmin: boolean;
  created?: string;
  /** Where the person's photo is served; none without a photo. */
  avatar?: string;
}

export interface SessionProject {
  icon?: string;
  slug: string;
  name: string;
  role: "viewer" | "member" | "admin" | "owner";
  autonomy: string;
  hasRepo: boolean;
}

export interface Session {
  user: SessionUser;
  /** "local": the server has no users yet and trusts this machine. */
  mode: "local" | "users";
  projects: SessionProject[];
  project?: string;
}

export class Unauthorized extends Error {}

/**
 * Whether the signed-in person may write in the current project: a member, an admin or an owner,
 * or anyone on a local server, which trusts this machine. Viewers read only.
 */
export function canWriteProject(session: Session | null | undefined): boolean {
  if (!session) return false;
  if (session.mode === "local") return true;
  const role = session.projects.find((p) => p.slug === session.project)?.role;
  return role !== undefined && role !== "viewer";
}

/** Whether the signed-in person may remove documentation sections: a project admin or owner, or anyone on a local server. */
export function canAdminProject(session: Session | null | undefined): boolean {
  if (!session) return false;
  if (session.mode === "local") return true;
  const role = session.projects.find((p) => p.slug === session.project)?.role;
  return role === "admin" || role === "owner";
}

export const sessionKey = ["session"] as const;

export function useSession() {
  return useQuery({
    queryKey: sessionKey,
    queryFn: async () => {
      try {
        return await request<Session>("GET", "/api/auth/me");
      } catch (e) {
        if (e instanceof ApiError && e.status === 401) return null;
        throw e;
      }
    },
    staleTime: 60_000,
    retry: false,
  });
}

/** Switch the current project: the server sets a cookie, every query refetches. */
export function useSwitchProject() {
  const qc = useQueryClient();
  return async (slug: string) => {
    await request("POST", "/api/session/project", { project: slug });
    await qc.resetQueries();
  };
}

export function useLogout() {
  const qc = useQueryClient();
  return async () => {
    await request("POST", "/api/auth/logout");
    qc.clear();
    window.location.assign("/");
  };
}
