import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { resolve } from "node:path";
import { createServer } from "node:net";

const root = resolve(import.meta.dirname, "../..");
const version = JSON.parse(readFileSync(resolve(root, "node_modules/@playwright/test/package.json"), "utf8")).version;
const probe = createServer();
await new Promise<void>((resolve) => probe.listen(0, "127.0.0.1", resolve));
const address = probe.address();
if (!address || typeof address === "string") throw new Error("No preview test port");
const port = address.port;
await new Promise<void>((resolve) => probe.close(() => resolve()));
const result = spawnSync("docker", [
  "run", "--rm", "--init", "--network", "host", "--ipc", "host",
  "--user", `${process.getuid?.() ?? 1000}:${process.getgid?.() ?? 1000}`,
  "--volume", `${root}:/work`, "--workdir", "/work", "--env", "HOME=/tmp", "--env", `GENIE_UI_PREVIEW_PORT=${port}`,
  `mcr.microsoft.com/playwright:v${version}-noble`,
  "npx", "playwright", "test", "--config", "test/ui-preview.config.ts",
], { stdio: "inherit" });
if (result.error) throw result.error;
process.exit(result.status ?? 1);
