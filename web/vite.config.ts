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
  build: {
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: process.env.GENIE_WEB_SOURCEMAP === "1",
    rolldownOptions: {
      output: {
        // Keep the libraries in their own chunks: a change to the app then leaves the
        // vendor files (and their cache entries) untouched. Groups are matched in order,
        // so react claims its own dependencies (scheduler, react-router's) before the
        // other groups look at them. Vite 8 bundles with Rolldown, where the option is
        // `codeSplitting`; `manualChunks`/`advancedChunks` are deprecated aliases.
        codeSplitting: {
          groups: [
            { name: "react", test: /node_modules[\\/](react|react-dom|react-router)[\\/]/ },
            { name: "react-query", test: /node_modules[\\/]@tanstack[\\/]/ },
            { name: "dnd-kit", test: /node_modules[\\/]@dnd-kit[\\/]/ },
          ],
        },
      },
    },
  },
  server: { port: 5173, proxy: { "/api": { target: "http://127.0.0.1:7420", headers: { host: "127.0.0.1:7420" } } } },
});
