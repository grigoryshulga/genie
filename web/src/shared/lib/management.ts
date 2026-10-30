// «Управление» is the project hub: three groups of pages behind one top-bar entry. The top bar's menu, the
// hub's rail and the route guards all read this one map, so a page moves between groups in one place.

export function canManageProject(isAdmin: boolean, role: string | undefined): boolean {
  return isAdmin || role === "admin" || role === "owner";
}

/** Who may open a hub page: any project member, the project's administrators, or server administrators only. */
export type HubAccess = "member" | "projectAdmin" | "serverAdmin";
export type HubViewer = { serverAdmin: boolean; projectAdmin: boolean };
export type HubItem = { id: string; label: string; to: string; access: HubAccess };
export type HubGroupId = "settings" | "agents" | "admin";
export type HubGroup = { id: HubGroupId; label: string; hint: string; items: readonly HubItem[] };

export const MANAGEMENT_HUB: readonly HubGroup[] = [
  {
    id: "settings",
    label: "Общие настройки",
    hint: "Проект, знания, уведомления, репозитории",
    items: [
      { id: "general", label: "Проект", to: "/manage/project/general", access: "projectAdmin" },
      { id: "knowledge", label: "База знаний", to: "/manage/project/knowledge", access: "projectAdmin" },
      { id: "channels", label: "Уведомления", to: "/manage/project/channels", access: "projectAdmin" },
      { id: "repos", label: "Репозитории и окружение", to: "/manage/project/repos", access: "projectAdmin" },
    ],
  },
  {
    id: "agents",
    label: "Агенты",
    hint: "Роли, шаблоны, навыки, MCP, автоматизации",
    items: [
      { id: "roles", label: "Роли", to: "/manage/agents/roles", access: "member" },
      { id: "templates", label: "Шаблоны команд", to: "/manage/agents/templates", access: "member" },
      { id: "skills", label: "Навыки", to: "/manage/agents/skills", access: "member" },
      { id: "mcp", label: "MCP-серверы", to: "/manage/agents/mcp", access: "member" },
      { id: "automations", label: "Автоматизации", to: "/manage/agents/automations", access: "member" },
      { id: "jev", label: "Jev", to: "/manage/agents/jev", access: "projectAdmin" },
      { id: "work", label: "Режим работы", to: "/manage/agents/work", access: "projectAdmin" },
    ],
  },
  {
    id: "admin",
    label: "Администрирование",
    hint: "Сервер, учётные записи, участники",
    items: [
      { id: "projects", label: "Проекты", to: "/manage/server/projects", access: "serverAdmin" },
      { id: "accounts", label: "Учётные записи", to: "/manage/server/accounts", access: "serverAdmin" },
      { id: "members", label: "Участники проекта", to: "/manage/server/members", access: "serverAdmin" },
      { id: "providers", label: "Провайдеры", to: "/manage/server/providers", access: "serverAdmin" },
      { id: "tools", label: "Инструменты сервера", to: "/manage/server/tools", access: "serverAdmin" },
      { id: "health", label: "Состояние", to: "/manage/server/health", access: "serverAdmin" },
      { id: "vault", label: "Хранилище знаний", to: "/manage/server/vault", access: "serverAdmin" },
      { id: "stats", label: "Статистика", to: "/manage/server/stats", access: "serverAdmin" },
      { id: "history", label: "Журнал агентов", to: "/manage/server/history", access: "serverAdmin" },
    ],
  },
];

export function canOpen(access: HubAccess, viewer: HubViewer): boolean {
  if (access === "serverAdmin") return viewer.serverAdmin;
  if (access === "projectAdmin") return viewer.serverAdmin || viewer.projectAdmin;
  return true;
}

/** The hub as this person sees it: groups keep only the pages they may open, and empty groups disappear. */
export function visibleHub(viewer: HubViewer): HubGroup[] {
  return MANAGEMENT_HUB.map((group) => ({ ...group, items: group.items.filter((item) => canOpen(item.access, viewer)) })).filter((group) => group.items.length > 0);
}

/** The hub page a path belongs to; nested pages (an automation run, an observation) light up their parent. */
export function activeHubItem(pathname: string): HubItem | undefined {
  for (const group of MANAGEMENT_HUB) for (const item of group.items) if (pathname === item.to || pathname.startsWith(`${item.to}/`)) return item;
  return undefined;
}

/** Where the top bar's «Управление» leads: the first page this person may open. */
export function hubHome(viewer: HubViewer): string {
  return visibleHub(viewer)[0]?.items[0]?.to ?? "/manage/agents/roles";
}

/** Old administration and agent links keep their destination after the hub regrouped the pages. */
export function managementDestination(scope: "project" | "server", section: string, tab?: string | null): string {
  if (scope === "project") {
    if (section === "general" && tab === "agents") return "/manage/agents/work";
    if (section === "tools") return `/manage/agents/${tab === "mcp" || tab === "jev" || tab === "automations" ? tab : "skills"}`;
    const moved: Record<string, string> = {
      agents: "/manage/agents/work",
      decisions: "/manage/agents/jev",
      telegram: "/manage/project/channels",
      automations: "/manage/agents/automations",
      "automation-runs": "/manage/agents/automations/runs",
      observations: "/manage/agents/automations/observations",
      observation: "/manage/agents/automations/sources",
      people: "/manage/server/members",
    };
    return moved[section] ?? `/manage/project/${section}`;
  }
  if (section === "roles" || section === "templates" || section === "skills") return `/manage/agents/${section}`;
  if (section === "agents") return "/manage/agents/roles";
  const moved: Record<string, string> = { mcp: "/manage/server/tools", stats: "/manage/server/stats" };
  return moved[section] ?? `/manage/server/${section}`;
}

export type ManagementScope = "project" | "server";
export const SETTINGS_TABS: Record<ManagementScope, Record<string, readonly (readonly [string, string])[]>> = {
  project: {
    repos: [
      ["repositories", "Репозитории"],
      ["git", "Git-токены"],
      ["environment", "Окружение"],
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
    members: [
      ["members", "Участники"],
      ["invites", "Пригласить"],
      ["activity", "Активность"],
    ],
    tools: [
      ["mcp", "MCP-серверы"],
      ["jev", "Jev"],
    ],
  },
};

/** Views that create something are actions, not tabs: the hub's head offers them as a button, and their
    form offers a way back instead of a tab. */
export const SETTINGS_ACTION_TABS: Partial<Record<ManagementScope, Record<string, readonly string[]>>> = {
  server: { projects: ["create"], accounts: ["create"], members: ["invites"] },
};

export function isActionTab(scope: ManagementScope, section: string, tab: string): boolean {
  return !!SETTINGS_ACTION_TABS[scope]?.[section]?.includes(tab);
}

export function settingsTab(scope: ManagementScope, section: string, requested: string | null): string {
  const tabs = SETTINGS_TABS[scope][section] ?? [];
  return tabs.find(([id]) => id === requested)?.[0] ?? tabs[0]?.[0] ?? "";
}

export function settingsTabHref(scope: ManagementScope, section: string, tab: string): string {
  return `/manage/${scope}/${section}?tab=${encodeURIComponent(tab)}`;
}

export type HubTab = { id: string; label: string; to: string; access: HubAccess };

/** Hub pages with several views keep a URL per view: the shared catalog beside the project's choice,
    automation rules beside their runs and the observations they feed. */
export const HUB_TABS: Record<"skills" | "mcp" | "automations", readonly HubTab[]> = {
  skills: [
    { id: "catalog", label: "Каталог", to: "/manage/agents/skills", access: "member" },
    { id: "project", label: "В проекте", to: "/manage/agents/skills/project", access: "projectAdmin" },
  ],
  mcp: [
    { id: "catalog", label: "Каталог", to: "/manage/agents/mcp", access: "member" },
    { id: "project", label: "В проекте", to: "/manage/agents/mcp/project", access: "projectAdmin" },
  ],
  automations: [
    { id: "rules", label: "Правила", to: "/manage/agents/automations", access: "projectAdmin" },
    { id: "runs", label: "Запуски", to: "/manage/agents/automations/runs", access: "member" },
    { id: "observations", label: "Наблюдения", to: "/manage/agents/automations/observations", access: "member" },
    { id: "sources", label: "Источники", to: "/manage/agents/automations/sources", access: "projectAdmin" },
  ],
};

/** The tab a path opens: the deepest tab URL that contains it (an observation's page belongs to «Наблюдения»). */
export function currentHubTab(tabs: readonly HubTab[], pathname: string): HubTab | undefined {
  const singular = pathname.replace("/automations/observation/", "/automations/observations/");
  return [...tabs].sort((a, b) => b.to.length - a.to.length).find((t) => singular === t.to || singular.startsWith(`${t.to}/`));
}
