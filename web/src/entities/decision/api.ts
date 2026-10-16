import { useQuery } from "@tanstack/react-query";
import { request, useInvalidating } from "@/shared/api";
import type { DecisionPolicy, DecisionRecord, DecisionSettings } from "./model.ts";

const scoped = (slug: string) => `project=${encodeURIComponent(slug)}`;
export const useDecisionSettings = (slug: string) => useQuery({
  queryKey: ["decision-policy", slug],
  queryFn: () => request<DecisionSettings>("GET", `/api/decisions/policy?${scoped(slug)}`),
});
export const useSaveDecisionPolicy = (slug: string) => useInvalidating(
  (p: DecisionPolicy) => request("PUT", `/api/decisions/policy?${scoped(slug)}`, p),
  [["decision-policy", slug]],
);
export const useDecisions = (slug: string) => useQuery({
  queryKey: ["decisions", slug],
  queryFn: () => request<DecisionRecord[]>("GET", `/api/decisions?${scoped(slug)}`),
  refetchInterval: 10_000,
});
