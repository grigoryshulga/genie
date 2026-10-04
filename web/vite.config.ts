import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// The build (web/dist) goes into the genie binary (crates/genie/build.rs); `genie
// serve --web web/dist` serves a fresh one without rebuilding genie. During
// development run `genie serve` (port 7420) and `npm run dev:web`; API calls are
// proxied to it. Source maps are not built by default because the binary never
// embeds them; `GENIE_WEB_SOURCEMAP=1 npm run build:web` turns them on for
// debugging a build served with `genie serve --web web/dist`.
export default defineConfig({
  root: import.meta.dirname,
  plugins: [react()],
  resolve: { alias: { "@": new URL("./src", import.meta.url).pathname } },
  build: { outDir: "dist", emptyOutDir: true, sourcemap: process.env.GENIE_WEB_SOURCEMAP === "1" },
  server: { port: 5173, proxy: { "/api": { target: "http://127.0.0.1:7420", headers: { host: "127.0.0.1:7420" } } } },
});
