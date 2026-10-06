import { lazy, type LazyExoticComponent, type ReactNode } from "react";

// A dynamic import can fail with a 404 after a deploy: the server keeps no old files
// under /assets/, so a tab of the previous build asks for a chunk name the new build
// no longer has, and the server answers 404. That is not a broken page — reloading
// picks up the new index.html and the new chunk names — so the first such failure
// reloads the page once. The sessionStorage flag stops a genuinely missing chunk from
// reloading forever, and a second failure (or any failure after the reload) is
// rethrown to the router's error screen.
const RELOADED = "genie.chunk-reload";

// A fresh boot clears the one-shot too: a deploy-triggered reload can land the user on an eager
// page (the board), and the next deploy's first stale chunk must still get its single reload
// instead of the error screen. This module loads with the entry chunk, so this runs at boot.
try {
  sessionStorage.removeItem(RELOADED);
} catch {
  // A hardened browser that blocks storage: there was no flag to clear.
}

type PageComponent<P> = (props: P) => ReactNode;

/** `React.lazy` for a page module, plus the stale-chunk reload above. */
export function lazyPage<P>(load: () => Promise<PageComponent<P>>): LazyExoticComponent<PageComponent<P>> {
  return lazy(() =>
    load().then(
      (page) => {
        sessionStorage.removeItem(RELOADED);
        return { default: page };
      },
      (error: unknown) => {
        if (sessionStorage.getItem(RELOADED)) throw error;
        sessionStorage.setItem(RELOADED, "1");
        window.location.reload();
        // Keep Suspense waiting: the document is being replaced anyway, and rejecting
        // here would paint the error screen for the instant before the reload.
        return new Promise<{ default: PageComponent<P> }>(() => {});
      },
    ),
  );
}
