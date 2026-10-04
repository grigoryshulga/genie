import { useEffect, useState } from "react";
import { queryClient } from "./client.ts";

/** Server-sent change events → refetch everything that is on screen. The stream is closed while the tab is hidden. */
export function useLiveUpdates(): boolean {
  const [online, setOnline] = useState(true);
  useEffect(() => {
    let es: EventSource | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const refresh = () => {
      clearTimeout(timer);
      timer = setTimeout(() => void queryClient.invalidateQueries(), 150);
    };
    const open = () => {
      es = new EventSource("/api/events");
      es.addEventListener("change", refresh);
      es.onopen = () => setOnline(true);
      es.onerror = () => setOnline(false);
    };
    const onVisibility = () => {
      if (document.hidden) {
        es?.close();
        es = undefined;
      } else if (!es) {
        open();
        refresh();
      }
    };
    if (!document.hidden) open();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      clearTimeout(timer);
      es?.close();
    };
  }, []);
  return online;
}
