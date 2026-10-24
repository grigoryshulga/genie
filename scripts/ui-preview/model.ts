export const states = ["populated", "empty", "loading", "error"] as const;
export type PreviewState = typeof states[number];
export type Fixtures = Record<string, unknown>;

export function fixtureKey(url: URL): string {
  const copy = new URL(url); copy.searchParams.sort();
  return copy.pathname + copy.search;
}

// Keep session/navigation contracts available while exercising each page's data state.
export function isBootstrap(path: string) {
  return ["/api/auth/me", "/api/projects", "/api/meta", "/api/docs/version"].includes(path);
}

export function fixtureFor(fixtures: Fixtures, url: URL, state: PreviewState): unknown {
  const exact = fixtureKey(url);
  let value = fixtures[exact];
  if (value === undefined && !["/api/docs/page", "/api/docs/search"].includes(url.pathname)) {
    // Pagination/filter variants can share a seeded read-only collection.
    const key = Object.keys(fixtures).find((k) => k.split("?")[0] === url.pathname);
    if (key) value = fixtures[key];
  }
  if (state !== "empty" || isBootstrap(url.pathname)) return value;
  if (Array.isArray(value)) return [];
  if (value && typeof value === "object") {
    const result: Record<string, unknown> = { ...value };
    for (const key of Object.keys(result)) {
      if (Array.isArray(result[key])) result[key] = [];
    }
    if ("unread" in result) result.unread = 0;
    return result;
  }
  return value;
}
