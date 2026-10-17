/** Old administration links preserve their destination after the profile menu split. */
export function managementDestination(scope: "project" | "server", section: string): string {
  if (scope === "server" && section === "stats") return "/statistics";
  const mapped =
    scope === "project"
      ? (({ agents: "general", decisions: "tools", telegram: "channels" } as Record<string, string>)[section] ?? section)
      : (({ mcp: "tools" } as Record<string, string>)[section] ?? section);
  return `/manage/${scope}/${mapped}`;
}

export function canManageProject(isAdmin: boolean, role: string | undefined): boolean {
  return isAdmin || role === "admin" || role === "owner";
}

export type ManagementScope = "project" | "server";
export const SETTINGS_TABS: Record<ManagementScope, Record<string, readonly (readonly [string, string])[]>> = {
  project: {
    general: [
      ["project", "Проект"],
      ["agents", "Работа агентов"],
    ],
    repos: [
      ["repositories", "Репозитории"],
      ["git", "Git-токены"],
      ["environment", "Окружение"],
    ],
    tools: [
      ["skills", "Навыки"],
      ["mcp", "MCP-серверы"],
      ["jev", "Jev"],
      ["automations", "Автоматизации"],
    ],
    people: [
      ["members", "Участники"],
      ["invites", "Приглашения"],
      ["activity", "Активность"],
    ],
    knowledge: [
      ["pages", "Документация"],
      ["backups", "Резервные копии"],
      ["storage", "Удалённое хранение"],
    ],
  },
  server: {
    projects: [
      ["list", "Проекты"],
      ["create", "Новый проект"],
    ],
    accounts: [
      ["list", "Учётные записи"],
      ["create", "Новая учётная запись"],
      ["access", "Роли в проектах"],
    ],
    tools: [
      ["mcp", "MCP-серверы"],
      ["jev", "Jev"],
      ["agents", "Агенты"],
      ["health", "Состояние"],
      ["vault", "База знаний"],
    ],
  },
};

export function settingsTab(scope: ManagementScope, section: string, requested: string | null): string {
  const tabs = SETTINGS_TABS[scope][section] ?? [];
  return tabs.find(([id]) => id === requested)?.[0] ?? tabs[0]?.[0] ?? "";
}

export function settingsTabHref(scope: ManagementScope, section: string, tab: string): string {
  if (scope === "project" && section === "tools" && tab === "automations") return "/manage/project/automations";
  if (scope === "server" && section === "tools") {
    if (tab === "agents") return "/manage/server/roles";
    if (tab === "health" || tab === "vault") return `/manage/server/${tab}`;
  }
  return `/manage/${scope}/${section}?tab=${encodeURIComponent(tab)}`;
}
