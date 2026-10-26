/** The sidebar searches the current collection; detail pages fall back to tasks. */
export function searchContext(pathname: string): { label: string; path: string } {
  const sections: [string, string][] = [
    ["/board", "Поиск на доске"],
    ["/tasks", "Поиск задач"],
    ["/epics", "Поиск эпиков"],
    ["/docs/proposals", "Поиск предложений"],
    ["/docs", "Поиск по документации"],
    ["/agents", "Поиск агентов и инструментов"],
    ["/automations", "Поиск автоматизаций"],
    ["/manage/project/automation-runs", "Поиск автоматизаций"],
    ["/notifications", "Поиск уведомлений"],
    ["/statistics", "Поиск проектов"],
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
