// Actions with a toast. `useAct` is for pages the journal's live updates already
// refresh; `useRefreshingAct` also refetches every query after a success — for the
// pages the live stream does not cover.

import { useQueryClient } from "@tanstack/react-query";
import { useToast } from "./toast.tsx";

/** Run an action; a failure becomes an error toast, success an optional one. */
export function useAct() {
  const toast = useToast();
  return async (fn: () => Promise<unknown>, ok?: string): Promise<boolean> => {
    try {
      await fn();
      if (ok) toast(ok);
      return true;
    } catch (e) {
      toast(e instanceof Error ? e.message : String(e), "error");
      return false;
    }
  };
}

/** The same, and every query refetches after a success. */
export function useRefreshingAct() {
  const act = useAct();
  const qc = useQueryClient();
  return async (fn: () => Promise<unknown>, ok?: string): Promise<boolean> => {
    const done = await act(fn, ok);
    if (done) await qc.invalidateQueries();
    return done;
  };
}
