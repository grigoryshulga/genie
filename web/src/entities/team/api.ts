import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { keys, keysFor, request, teamKeys, teamState, useInvalidating } from "@/shared/api";
import type { Mail, MailLevel, Peek, Team, TeamDetail, TeamView } from "./model.ts";

export const useTeams = () => useQuery({ queryKey: keys.teams, queryFn: () => request<TeamView[]>("GET", "/api/teams?all=1") });

export const useTeam = (id: string | undefined) =>
  useQuery({
    queryKey: keys.team(id ?? ""),
    queryFn: () => request<TeamDetail>("GET", `/api/teams/${encodeURIComponent(id!)}`),
    enabled: !!id,
    // A session's current step changes without a database write: poll while someone works.
    refetchInterval: (q) => (Object.values(q.state.data?.sessions ?? {}).some((s) => s.state === "working") ? 3000 : false),
  });

export const useSendMail = () =>
  useInvalidating(
    (v: { team: string; to: string; text: string; level: MailLevel; intent?: Mail["intent"] }) =>
      request<unknown>("POST", `/api/teams/${encodeURIComponent(v.team)}/mail`, { to: v.to, text: v.text, level: v.level, intent: v.intent }),
    keysFor({ type: "mail.sent" }),
  );

export const useStopTeam = () =>
  useInvalidating(
    (v: { team: string; removeWorktree?: boolean }) => request<{ report: string[] }>("POST", `/api/teams/${encodeURIComponent(v.team)}/stop`, { removeWorktree: v.removeWorktree }),
    keysFor({ type: "team.stopped" }),
  );

/** Deleting a team writes no journal event, so it refetches the team's own queries itself. */
export const useDeleteTeam = () =>
  useInvalidating(
    (v: { team: string; removeWorktree?: boolean }) =>
      request<{ report: string[] }>("DELETE", `/api/teams/${encodeURIComponent(v.team)}${v.removeWorktree ? "?removeWorktree=1" : ""}`),
    teamState(),
  );

/** A member change lands in `members` and the team log, which no journal event carries. */
export const useAddMember = () =>
  useInvalidating(
    (v: { team: string; role: string; name?: string; model?: string; instructions?: string }) =>
      request<{ name: string; role: string; model?: string }[]>("POST", `/api/teams/${encodeURIComponent(v.team)}/members`, v),
    teamKeys(),
  );

export const useRemoveMember = () =>
  useInvalidating(
    (v: { team: string; member: string }) =>
      request<unknown>("DELETE", `/api/teams/${encodeURIComponent(v.team)}/members/${encodeURIComponent(v.member)}`),
    teamKeys(),
  );

export function useTeamMap(): Map<string, Team> {
  const teams = useTeams().data;
  return useMemo(() => new Map((teams ?? []).map((t) => [t.id, t])), [teams]);
}

/** An agent's live session and the latest messages of its conversation. */
export const usePeek = (team: string | undefined, member: string | undefined, working: boolean) =>
  useQuery({
    queryKey: ["peek", team ?? "", member ?? ""],
    queryFn: () => request<Peek>("GET", `/api/agents/${encodeURIComponent(team!)}/${encodeURIComponent(member!)}/peek?deep=1&limit=40`),
    enabled: !!team && !!member,
    refetchInterval: working ? 2500 : 10_000,
  });

/** Pause a member (its session stops, mail waits) or let it go on with the mail that waited. */
export const useSetPaused = () =>
  useInvalidating(
    (v: { team: string; member: string; paused: boolean }) =>
      request<unknown>("POST", `/api/agents/${encodeURIComponent(v.team)}/${encodeURIComponent(v.member)}/${v.paused ? "pause" : "resume"}`, {}),
    teamKeys(),
  );

/** Restart a member's session with the role's current settings; the conversation goes on. */
export const useRestartMember = () =>
  useInvalidating(
    (v: { team: string; member: string }) => request<unknown>("POST", `/api/teams/${encodeURIComponent(v.team)}/members/${encodeURIComponent(v.member)}/restart`, {}),
    teamKeys(),
  );

/** A model an agent can be given; `listed: false` when pi's catalogue does not have it. */
export type ModelOption = { id: string; provider: string; name: string; listed: boolean };

/** The models agents can be given and the thinking levels (pi's catalogue is read at most every few minutes). */
export const useModels = (enabled = true) =>
  useQuery({
    queryKey: ["models"],
    queryFn: () => request<{ catalogue: boolean; models: ModelOption[]; thinking: string[] }>("GET", "/api/models"),
    enabled,
    staleTime: 300_000,
  });

/** Give a member its own model and thinking level (`undefined`: as its role); it switches at the end of its current step. */
export const useSetMemberModel = () =>
  useInvalidating(
    (v: { team: string; member: string; model?: string; thinking?: string }) =>
      request<{ model?: string; thinking?: string }>("PATCH", `/api/teams/${encodeURIComponent(v.team)}/members/${encodeURIComponent(v.member)}`, {
        model: v.model ?? null,
        thinking: v.thinking ?? null,
      }),
    teamKeys(),
  );
