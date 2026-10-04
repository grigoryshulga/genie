import { useEffect, useState } from "react";
import { queryClient } from "./client.ts";
import { type JournalEvent, keysForAll, matchesAny } from "./invalidation.ts";

/** A burst of events is refetched together: after this much quiet, or at the latest `MAX_WAIT_MS` after its first event. */
const DEBOUNCE_MS = 150;
const MAX_WAIT_MS = 1000;

/** Server-sent journal events → refetch the queries they touch. The stream is closed while the tab is hidden. */
export function useLiveUpdates(): boolean {
  const [online, setOnline] = useState(true);
  useEffect(() => {
    let es: EventSource | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let batch: JournalEvent[] = [];
    let first = 0;
    const drop = () => {
      clearTimeout(timer);
      batch = [];
      first = 0;
    };
    const flush = () => {
      const events = batch;
      drop();
      // A change without readable events is something we cannot name: refetch everything.
      const keys = events.length ? keysForAll(events) : null;
      void (keys ? queryClient.invalidateQueries({ predicate: (q) => matchesAny(keys, q.queryKey) }) : queryClient.invalidateQueries());
    };
    const schedule = () => {
      first ||= Date.now();
      clearTimeout(timer);
      timer = setTimeout(flush, Math.min(DEBOUNCE_MS, Math.max(0, first + MAX_WAIT_MS - Date.now())));
    };
    // Events missed while the stream was closed are unknown: one broad refetch.
    const refreshAll = () => {
      batch.push({ type: "" });
      schedule();
    };
    const open = () => {
      es = new EventSource("/api/events");
      // `journal` events come first, then one `change` that ends the batch.
      es.addEventListener("journal", (m) => {
        try {
          batch.push(JSON.parse((m as MessageEvent<string>).data) as JournalEvent);
        } catch {
          batch.push({ type: "" }); // unreadable: broad
        }
      });
      es.addEventListener("change", schedule);
      es.onopen = () => setOnline(true);
      es.onerror = () => setOnline(false);
    };
    const onVisibility = () => {
      if (document.hidden) {
        es?.close();
        es = undefined;
        drop();
      } else if (!es) {
        open();
        refreshAll();
      }
    };
    if (!document.hidden) open();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      drop();
      es?.close();
    };
  }, []);
  return online;
}
