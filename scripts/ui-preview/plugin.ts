import type { IncomingMessage, ServerResponse } from "node:http";
import { readFileSync } from "node:fs";
import type { Plugin } from "vite";
import { fixtureFor, isBootstrap, states, type Fixtures, type PreviewState } from "./model.ts";

/** Development only: handles every API call locally; never proxies an unknown mock request. */
export function uiPreviewMiddleware() {
  const fixtures: Fixtures = JSON.parse(readFileSync(new URL("./fixtures.json", import.meta.url), "utf8"));
  return (req: IncomingMessage, res: ServerResponse, next: () => void) => {
    const url = new URL(req.url ?? "/", "http://preview.local");
    if (url.pathname === "/__preview") {
      res.setHeader("content-type", "text/html; charset=utf-8");
      res.end(readFileSync(new URL("./controls.html", import.meta.url))); return;
    }
    if (!url.pathname.startsWith("/api/")) { next(); return; }
    res.setHeader("cache-control", "no-store");
    if (url.pathname === "/api/events") {
      res.writeHead(200, { "content-type": "text/event-stream" });
      res.write(": mock preview connected\n\n");
      const timer = setInterval(() => res.write(": heartbeat\n\n"), 15_000);
      res.on("close", () => clearInterval(timer)); return;
    }
    res.setHeader("content-type", "application/json; charset=utf-8");
    const candidate = req.headers.cookie?.match(/(?:^|;\s*)genie-preview-state=([^;]+)/)?.[1];
    const state: PreviewState = states.find((s) => s === candidate) ?? "populated";
    const send = (status: number, data: unknown) => { res.statusCode = status; res.end(JSON.stringify(data)); };
    if (req.method !== "GET") {
      // This POST only renders prompts; all writes and task-specific simulations stay on dev.
      if (req.method === "POST" && /^\/api\/templates\/[^/]+\/preview$/.test(url.pathname)) {
        let body = "";
        req.on("data", (chunk) => { body += chunk; });
        req.on("end", () => {
          let value: unknown;
          try { value = body ? JSON.parse(body) : {}; } catch { send(400, { error: "Invalid preview JSON" }); return; }
          if (value && typeof value === "object" && "task" in value && value.task) { send(501, { error: "Task-specific prompts require the dev backend." }); return; }
          if (state === "loading") return;
          if (state === "error") { send(503, { error: "Preview: simulated server unavailable" }); return; }
          const fixture = fixtureFor(fixtures, url, state);
          if (fixture === undefined) send(501, { error: `No UI fixture for ${url.pathname}; use dev.` });
          else send(200, fixture);
        });
        return;
      }
      send(501, { error: "UI preview is read-only. Verify saved changes against the dev backend." }); return;
    }
    if (!isBootstrap(url.pathname)) {
      if (state === "loading") return; // Kept pending until this document/connection closes.
      if (state === "error") { send(503, { error: "Preview: simulated server unavailable" }); return; }
    }
    const value = fixtureFor(fixtures, url, state);
    if (value === undefined) { send(501, { error: `No UI fixture for ${url.pathname}; add a disposable fixture or use dev.` }); return; }
    send(200, value);
  };
}

export function uiPreview(): Plugin {
  return { name: "genie-ui-preview", configureServer(server) { server.middlewares.use(uiPreviewMiddleware()); } };
}
