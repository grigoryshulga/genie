import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { fixtureFor, fixtureKey } from "../scripts/ui-preview/model.ts";
import { uiPreviewMiddleware } from "../scripts/ui-preview/plugin.ts";
import { createServer } from "node:http";

test("mock lookup preserves document identity but permits collection filter variants", () => {
  const fixtures = { "/api/tasks?closed=1": [{ id: "DM-1" }], "/api/docs/page?path=a": { content: "A" } };
  assert.deepEqual(fixtureFor(fixtures, new URL("http://test/api/tasks?closed=0"), "populated"), [{ id: "DM-1" }]);
  assert.equal(fixtureFor(fixtures, new URL("http://test/api/docs/page?path=b"), "populated"), undefined);
  assert.equal(fixtureKey(new URL("http://test/api/docs/search?q=x&limit=30")), "/api/docs/search?limit=30&q=x");
});

test("empty state keeps the session usable and leaves the source fixture unchanged", () => {
  const fixtures = { "/api/tasks": [{ id: "DM-1" }], "/api/auth/me": { projects: ["demo"] } };
  assert.deepEqual(fixtureFor(fixtures, new URL("http://test/api/tasks"), "empty"), []);
  assert.deepEqual(fixtureFor(fixtures, new URL("http://test/api/auth/me"), "empty"), { projects: ["demo"] });
  assert.equal(fixtures["/api/tasks"].length, 1);
  assert.deepEqual(fixtureFor({ "/api/catalogue": { roles: ["executor"], version: "1" } }, new URL("http://test/api/catalogue"), "empty"), { roles: [], version: "1" });
});

test("mock server cannot forward unknown reads or writes; scenarios are per browser cookie", async () => {
  const handler = uiPreviewMiddleware();
  const server = createServer((req, res) => handler(req, res, () => res.end()));
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address(); assert.ok(address && typeof address !== "string");
  const base = `http://127.0.0.1:${address.port}`;
  try {
    assert.equal((await fetch(base + "/api/not-recorded")).status, 501);
    assert.equal((await fetch(base + "/api/tasks", { method: "POST" })).status, 501);
    assert.equal((await fetch(base + "/api/templates/full", { method: "PUT", body: "{}" })).status, 501);
    const preview = await fetch(base + "/api/templates/full/preview", { method: "POST", body: "{}" });
    assert.equal(preview.status, 200);
    assert.equal((await fetch(base + "/api/templates/full/preview", { method: "POST", body: "{}", headers: { cookie: "genie-preview-state=error" } })).status, 503);
    assert.equal((await preview.json()).template, "full");
    assert.equal((await fetch(base + "/api/templates/full/preview", { method: "POST", body: '{"task":"DM-2"}' })).status, 501);
    assert.equal((await fetch(base + "/api/templates/full/preview", { method: "POST", body: "broken" })).status, 400);
    assert.equal((await fetch(base + "/api/tasks", { headers: { cookie: "genie-preview-state=error" } })).status, 503);
    assert.deepEqual(await (await fetch(base + "/api/tasks", { headers: { cookie: "genie-preview-state=empty" } })).json(), []);
    assert.ok((await (await fetch(base + "/api/tasks")).json()).length > 0);
    assert.equal((await fetch(base + "/api/auth/me", { headers: { cookie: "genie-preview-state=error" } })).status, 200);
    await assert.rejects(fetch(base + "/api/tasks", { headers: { cookie: "genie-preview-state=loading" }, signal: AbortSignal.timeout(80) }), { name: "TimeoutError" });
  } finally { server.closeAllConnections(); await new Promise<void>((resolve) => server.close(() => resolve())); }
});

test("committed fixtures contain only synthetic portable data", () => {
  const text = readFileSync(new URL("../scripts/ui-preview/fixtures.json", import.meta.url), "utf8");
  assert.doesNotMatch(text, /\/home\/|\.ts\.net|\/tmp\/genie-ui-|ui-tests-password/);
});
