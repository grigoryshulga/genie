/** The sidebar searches the current collection; detail pages fall back to tasks. */
export function searchContext(pathname: string): { label: string; path: string } {
  const sections: [string, string][] = [
    ["/board", "Поиск на доске"],
    ["/tasks", "Поиск задач"],
    ["/epics", "Поиск эпиков"],
    ["/docs/changes", "Поиск изменений документации"],
    ["/docs", "Поиск по документации"],
    // The more specific management page goes first: runs live under «Агенты».
    ["/manage/agents/automations/runs", "Поиск автоматизаций"],
    ["/manage/agents/roles", "Поиск агентов и инструментов"],
    ["/manage/agents/templates", "Поиск агентов и инструментов"],
    ["/manage/agents/skills", "Поиск агентов и инструментов"],
    ["/manage/agents/mcp", "Поиск агентов и инструментов"],
    ["/notifications", "Поиск уведомлений"],
    ["/manage/server/stats", "Поиск проектов"],
  ];
  const match = sections.find(([path]) => pathname === path || pathname.startsWith(`${path}/`));
  return match ? { label: match[1], path: pathname } : { label: "Поиск задач", path: "/tasks" };
}

export function searchParams(current: URLSearchParams, query: string): URLSearchParams {
  const next = new URLSearchParams(current);
  if (query) next.set("q", query);
  else next.delete("q");
  return next;
}

export function matchesSearch(query: string, ...values: (string | undefined | null)[]): boolean {
  const needle = query.trim().toLocaleLowerCase();
  return !needle || values.some((value) => value?.toLocaleLowerCase().includes(needle));
}
