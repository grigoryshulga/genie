// Every screen the capture harness photographs, keyed by a stable file name. The list follows
// the routes in web/src/app/App.tsx. Team and agent chat screens need a running team, which the
// seed cannot create without live agents, so they are the known gap (docs/development.md).

export interface CaptureRoute {
  name: string;
  path: string;
}

export const CAPTURE_ROUTES: CaptureRoute[] = [
  { name: "board", path: "/board" },
  { name: "tasks", path: "/tasks" },
  { name: "task", path: "/task/DM-2" },
  { name: "task-review", path: "/task/DM-9" },
  { name: "task-needs-owner", path: "/task/DM-12" },
  { name: "epics", path: "/epics" },
  { name: "epic", path: "/epic/DM-1" },
  { name: "docs", path: "/docs" },
  { name: "docs-reader", path: "/docs?page=demo/guides/deploy.md" },
  { name: "docs-editor", path: "/docs/edit?page=demo/guides/deploy.md" },
  { name: "docs-search", path: "/docs?q=деплой" },
  { name: "docs-folder", path: "/docs?page=demo/guides" },
  { name: "docs-outdated", path: "/docs?page=demo/decisions/storage.md" },
  { name: "docs-proposals", path: "/docs/proposals" },
  { name: "automations", path: "/automations" },
  { name: "observations", path: "/observations" },
  { name: "agents", path: "/agents" },
  { name: "agents-templates", path: "/agents?tab=templates" },
  { name: "agents-skills", path: "/agents?tab=skills" },
  { name: "agents-mcp", path: "/agents?tab=mcp" },
  { name: "notifications", path: "/notifications" },
  { name: "statistics", path: "/statistics" },
  { name: "settings-me", path: "/settings/me" },
  { name: "manage-project", path: "/manage/project/general" },
  { name: "manage-project-people", path: "/manage/project/people" },
  { name: "manage-server", path: "/manage/server/projects" },
  { name: "manage-server-health", path: "/manage/server/health" },
];
