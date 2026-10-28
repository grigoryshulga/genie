import assert from "node:assert/strict";
import { test } from "node:test";
import { matchesSearch, searchContext, searchParams } from "../web/src/shared/lib/contextSearch.ts";

test("context search selects the current collection and distinguishes proposals from documentation", () => {
  for (const path of ["/board", "/tasks", "/epics", "/manage/agents/roles", "/manage/agents/skills", "/manage/agents/automations/runs", "/notifications", "/manage/server/stats", "/docs", "/docs/proposals"])
    assert.equal(searchContext(path).path, path);
  assert.equal(searchContext("/manage/agents/automations/runs").label, "Поиск автоматизаций");
  assert.equal(searchContext("/manage/agents/mcp").label, "Поиск агентов и инструментов");
  assert.equal(searchContext("/docs/proposals").label, "Поиск предложений");
  assert.equal(searchContext("/docs/edit").label, "Поиск по документации");
  assert.deepEqual(searchContext("/task/DEMO-1"), { label: "Поиск задач", path: "/tasks" });
  assert.equal(searchContext("/epics-unknown").path, "/tasks");
});

test("editing and clearing search preserves active filters and selection without mutating input", () => {
  const current = new URLSearchParams("epic=DEMO-1&status=ready&task=DEMO-2");
  const next = searchParams(current, "проверка & обзор");
  assert.equal(current.has("q"), false);
  assert.equal(next.get("q"), "проверка & обзор");
  assert.equal(next.get("epic"), "DEMO-1");
  assert.equal(next.get("task"), "DEMO-2");
  assert.equal(searchParams(next, "").toString(), current.toString());
});

test("collection matching handles Cyrillic, whitespace, and missing descriptions", () => {
  assert.equal(matchesSearch("  ОБЗОР ", "Обзор проекта", undefined), true);
  assert.equal(matchesSearch("mcp", null, "MCP tools"), true);
  assert.equal(matchesSearch("missing", "Обзор проекта", null), false);
  assert.equal(matchesSearch(" ", undefined), true);
});
