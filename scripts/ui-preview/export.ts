// Called only by the disposable browser-test seed runner, never against a live tracker.
import { globSync, readFileSync, writeFileSync } from "node:fs";
import { synthetic } from "./synthetic.ts";

export async function exportFixtures(base: string, output: string) {
  const login = await fetch(`${base}/api/auth/login`, {
    method: "POST", headers: { "content-type": "application/json", "x-genie": "1" },
    body: JSON.stringify({ login: "owner", password: "ui-tests-password" }),
  });
  if (!login.ok) throw new Error(`fixture login: ${login.status}`);
  const cookie = login.headers.getSetCookie().map((v) => v.split(";")[0]).join("; ");
  const fixtures: Record<string, unknown> = {};
  const paths = [
    "/api/auth/me", "/api/meta", "/api/projects", "/api/users", "/api/teams", "/api/tasks?closed=1",
    "/api/agent-config/history", "/api/notifications", "/api/notifications/unread", "/api/agent-config", "/api/agents", "/api/automations",
    "/api/observations", "/api/docs/tree", "/api/docs/version", "/api/docs/proposals", "/api/doctor",
    "/api/stats?days=30", "/api/stats?days=7", "/api/auth/tokens", "/api/repos", "/api/git/hosts",
    "/api/me/keys", "/api/me/telegram", "/api/providers", "/api/model-prices", "/api/vault/sync",
    "/api/projects/demo/members", "/api/projects/demo", "/api/projects/demo/tools", "/api/projects/demo/observation-sources",
    "/api/docs/search?q=деплой&limit=30", "/api/docs/page?path=demo/guides/deploy.md",
    "/api/docs/page?path=demo/decisions/storage.md",
  ];
  for (const file of globSync("web/src/**/*.ts*")) {
    const text = readFileSync(file, "utf8");
    for (const match of text.matchAll(/"GET",\s*"(\/api[^"$]+)"/g)) paths.push(match[1]);
  }
  paths.push("/api/observation/cases", "/api/observation/sources", "/api/docs/proposals?status=open", "/api/runs");
  // Include all seeded task details and the ancillary queries task pages request.
  for (let i = 1; i <= 16; i++) paths.push(`/api/tasks/DM-${i}`, `/api/tasks/DM-${i}/docs-impact`, `/api/tasks/DM-${i}/repos`, `/api/tasks/DM-${i}/observations`, `/api/tasks/DM-${i}/intelligence`, `/api/management/tasks/DM-${i}`);
  for (const path of new Set(paths)) {
    const res = await fetch(base + path, { headers: { cookie, "x-genie-project": "demo" } });
    if (!res.ok) continue; // Missing contracts stay explicit 501s in mock mode.
    const data: unknown = await res.json();
    const url = new URL(path, base); url.searchParams.sort();
    fixtures[url.pathname + url.search] = data;
  }
  // Role details are requested dynamically after selecting a catalogue item.
  const catalogue = fixtures["/api/agent-config"] as { roles?: { id: string }[]; teams?: { id: string }[] } | undefined;
  for (const role of catalogue?.roles ?? []) {
    const path = `/api/roles/${encodeURIComponent(role.id)}`;
    const res = await fetch(base + path, { headers: { cookie, "x-genie-project": "demo" } });
    if (res.ok) fixtures[path] = await res.json();
  }
  for (const team of catalogue?.teams ?? []) {
    const path = `/api/templates/${encodeURIComponent(team.id)}`;
    const headers = { cookie, "x-genie-project": "demo", "x-genie": "1", "content-type": "application/json" };
    const detail = await fetch(base + path, { headers });
    if (detail.ok) fixtures[path] = await detail.json();
    const preview = await fetch(base + path + "/preview", { method: "POST", headers, body: "{}" });
    if (preview.ok) fixtures[path + "/preview"] = await preview.json();
  }
  Object.assign(fixtures, synthetic);
  const text = JSON.stringify(fixtures, null, 2)
    .replace(/\/tmp\/genie-ui-[^"\s]+/g, "/preview/demo")
    .replace(/\/home\/[^"\s]+/g, "/preview/source")
    .replace(/127\.0\.0\.1:\d+/g, "127.0.0.1:5173")
    .replace(/\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z/g, "2026-05-01T12:00:00Z");
  writeFileSync(output, text + "\n");
  console.log(`Exported ${Object.keys(fixtures).length} disposable API fixtures to ${output}`);
}
