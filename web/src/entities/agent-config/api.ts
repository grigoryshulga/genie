import { type QueryClient, useQuery, useQueryClient } from "@tanstack/react-query";
import { ApiError, request } from "@/shared/api";
import type { Catalogue, ConfigChange, McpAdmin, McpCall, McpCheck, McpDetail, McpEntry, McpUsageRow, Preview, RoleDetail, SkillDetail, TemplateDetail } from "./model.ts";

export const agentKeys = {
  all: ["agent-config"] as const,
  catalogue: ["agent-config", "catalogue"] as const,
  role: (id: string) => ["agent-config", "role", id] as const,
  template: (id: string) => ["agent-config", "template", id] as const,
  skill: (name: string) => ["agent-config", "skill", name] as const,
  mcp: ["agent-config", "mcp"] as const,
  mcpAdmin: ["agent-config", "mcp-admin"] as const,
  mcpUsage: (days: number) => ["agent-config", "mcp-usage", days] as const,
  history: (item?: string) => ["agent-config", "history", item ?? ""] as const,
};

/** Roles, templates, skills and MCP connections the current project may use. */
export function useAgentConfig() {
  return useQuery({ queryKey: agentKeys.catalogue, queryFn: () => request<Catalogue>("GET", "/api/agent-config") });
}

export function useRole(id: string | undefined) {
  return useQuery({ queryKey: agentKeys.role(id ?? ""), queryFn: () => request<RoleDetail>("GET", `/api/roles/${id}`), enabled: !!id });
}

export function useTemplate(id: string | undefined) {
  return useQuery({ queryKey: agentKeys.template(id ?? ""), queryFn: () => request<TemplateDetail>("GET", `/api/templates/${id}`), enabled: !!id });
}

export function useSkill(name: string | undefined) {
  return useQuery({ queryKey: agentKeys.skill(name ?? ""), queryFn: () => request<SkillDetail>("GET", `/api/skills/${name}`), enabled: !!name });
}

export function useMcpConfig() {
  return useQuery({ queryKey: agentKeys.mcp, queryFn: () => request<McpDetail>("GET", "/api/mcp") });
}

const skillFileUrl = (name: string, path: string) => `/api/skills/${encodeURIComponent(name)}/files/${path.split("/").map(encodeURIComponent).join("/")}`;

/** A supporting file of a skill: its text, or only its size when it is binary or large. */
export function useSkillFile(name: string, path: string | undefined) {
  return useQuery({
    queryKey: ["agent-config", "skill-file", name, path ?? ""],
    queryFn: () => request<{ path: string; size: number; text: string | null }>("GET", skillFileUrl(name, path ?? "")),
    enabled: !!path,
  });
}

/** Put a file into a skill's directory (administrators; up to 5 MB). */
export function useSkillFiles() {
  const qc = useQueryClient();
  const done = () => qc.invalidateQueries({ queryKey: agentKeys.all });
  return {
    upload: async (name: string, path: string, file: Blob) => {
      const res = await fetch(skillFileUrl(name, path), { method: "PUT", headers: { "x-genie": "1", "content-type": "application/octet-stream" }, body: file });
      const data = (await res.json().catch(() => ({}))) as { error?: string };
      if (!res.ok) throw new ApiError(data.error ?? `${res.status} ${res.statusText}`, res.status);
      await done();
    },
    remove: async (name: string, path: string) => {
      await request("DELETE", skillFileUrl(name, path));
      await done();
    },
  };
}

/** The project's latest tool calls through the MCP gateway, newest first. Each is an `mcp.called` journal event, so the live stream refetches this: no polling. */
export function useMcpCalls(project: string) {
  return useQuery({
    queryKey: ["mcp-calls", project],
    queryFn: () => request<McpCall[]>("GET", "/api/mcp/calls?limit=100"),
  });
}

/** Start a connection as agents would get it and list its tools (administrators). */
export function checkMcp(id: string) {
  return request<McpCheck>("POST", `/api/mcp/${encodeURIComponent(id)}/check`);
}

/** The MCP tab of /server in one answer: connections, secrets, the file (server administrators). */
export function useMcpAdmin() {
  return useQuery({ queryKey: agentKeys.mcpAdmin, queryFn: () => request<McpAdmin>("GET", "/api/mcp-admin") });
}

/** Gateway tool calls of the last `days` days, one row per day and tool; the web adds them up. */
export function useMcpUsage(days: number) {
  return useQuery({
    queryKey: agentKeys.mcpUsage(days),
    queryFn: () => request<{ days: number; rows: McpUsageRow[] }>("GET", `/api/mcp-admin/usage?days=${days}`),
  });
}

/** After a change to mcp.json or its secrets: the catalogue, the client MCP views and the admin tab. */
async function refetchMcp(qc: QueryClient) {
  await Promise.all([agentKeys.all, agentKeys.mcp, agentKeys.mcpAdmin].map((key) => qc.invalidateQueries({ queryKey: key })));
}

/**
 * Save one connection of mcp.json (server administrators): `entry` as it will
 * stand in `mcpServers`, `baseHash` the version the edit started from. 409:
 * the file changed since — refetch and redo; 422: the entry does not load.
 */
export function useSaveMcpServer() {
  const qc = useQueryClient();
  return async (id: string, entry: McpEntry, baseHash?: string) => {
    const out = await request<SaveResult>("PUT", `/api/mcp-admin/servers/${encodeURIComponent(id)}`, { entry, baseHash });
    await refetchMcp(qc);
    return out;
  };
}

/** Remove a connection from mcp.json (server administrators); roles keep their grants until edited. */
export function useDeleteMcpServer() {
  const qc = useQueryClient();
  return async (id: string, baseHash?: string) => {
    const q = baseHash ? `?baseHash=${encodeURIComponent(baseHash)}` : "";
    const out = await request<SaveResult>("DELETE", `/api/mcp-admin/servers/${encodeURIComponent(id)}${q}`);
    await refetchMcp(qc);
    return out;
  };
}

/** Set the value of an environment variable MCP entries refer to; a blank value removes it. */
export function setMcpSecret(name: string, value: string) {
  return request<{ ok: boolean }>("PUT", `/api/mcp-admin/secrets/${encodeURIComponent(name)}`, { value });
}

export function deleteMcpSecret(name: string) {
  return request<{ ok: boolean }>("DELETE", `/api/mcp-admin/secrets/${encodeURIComponent(name)}`);
}

/** Changes made through the server (administrators only). */
export function useConfigHistory(item: string | undefined, enabled = true, before?: number, limit?: number) {
  const query = new URLSearchParams();
  if (item) query.set("item", item);
  if (before !== undefined) query.set("before", String(before));
  if (limit !== undefined) query.set("limit", String(limit));
  return useQuery({
    queryKey: [...agentKeys.history(item), before ?? null, limit ?? null],
    queryFn: () => request<ConfigChange[]>("GET", `/api/agent-config/history?${query}`),
    enabled,
  });
}

/** What every member of a template would get as its kickoff, for a task (or an example one). */
export function usePreview(template: string | undefined, task: string | undefined) {
  return useQuery({
    queryKey: ["agent-config", "preview", template ?? "", task ?? ""],
    queryFn: () => request<Preview>("POST", `/api/templates/${template}/preview`, task ? { task } : {}),
    enabled: !!template,
    retry: false,
  });
}

export type ConfigKind = "role" | "template" | "skill" | "mcp";

const itemUrl = (kind: ConfigKind, id: string) =>
  kind === "role" ? `/api/roles/${id}` : kind === "template" ? `/api/templates/${id}` : kind === "skill" ? `/api/skills/${id}` : "/api/mcp";

export interface SaveResult {
  ok: boolean;
  path: string;
  hash: string;
  problems: { level: string; item: string; message: string }[];
}

/**
 * Save a configuration file. `baseHash` is the version the edit started from
 * (`""` for a new file): the server refuses when the file changed since (409)
 * and when the change is invalid or breaks something that works now (422).
 */
export function useSaveConfig() {
  const qc = useQueryClient();
  return async (kind: ConfigKind, id: string, body: { content?: string; template?: unknown }, baseHash: string | undefined) => {
    const out = await request<SaveResult>("PUT", itemUrl(kind, id), { ...body, baseHash });
    await qc.invalidateQueries({ queryKey: agentKeys.all });
    return out;
  };
}

/** Remove a data-directory file: a custom item goes away, a built-in one returns to its defaults. */
export function useDeleteConfig() {
  const qc = useQueryClient();
  return async (kind: Exclude<ConfigKind, "mcp">, id: string, baseHash?: string) => {
    const q = baseHash ? `?baseHash=${encodeURIComponent(baseHash)}` : "";
    const out = await request<{ ok: boolean }>("DELETE", `${itemUrl(kind, id)}${q}`);
    await qc.invalidateQueries({ queryKey: agentKeys.all });
    return out;
  };
}

export function useProjectTools(slug: string) {
  return useQuery({ queryKey: ["project-tools", slug], queryFn: () => request<import("@/shared/api/generated/ProjectToolsView.ts").ProjectToolsView>("GET", `/api/projects/${encodeURIComponent(slug)}/tools`) });
}

export function useSaveProjectTools(slug: string) {
  const qc = useQueryClient();
  return async (body: import("@/shared/api/generated/ProjectToolsWrite.ts").ProjectToolsWrite) => {
    try {
      await request("PUT", `/api/projects/${encodeURIComponent(slug)}/tools`, body);
    } finally {
      await Promise.all([qc.invalidateQueries({ queryKey: ["project-tools", slug] }), qc.invalidateQueries({ queryKey: agentKeys.all })]);
    }
  };
}
