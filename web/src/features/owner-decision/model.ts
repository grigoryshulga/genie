// What the decision box says and sends, apart from how it looks.

import { nextIndex } from "../../shared/ui/kit/menu-nav.ts";
import type { OwnerAction } from "../../shared/api/generated/OwnerAction.ts";

export type Kind = OwnerAction["kind"];

/** The kinds this web has a card for; any other shows as a plain question. */
export const CARD_KINDS: readonly Kind[] = ["ask-owner-question", "ask-for-merge-pr"];

/** The action's card, or none: an action from a newer server shows as a plain question. */
export function cardKind(action: { kind: string } | undefined): Kind | undefined {
  return action && (CARD_KINDS as readonly string[]).includes(action.kind) ? (action.kind as Kind) : undefined;
}

/** The answer as it goes into the task: the picked option first, then the words. */
export function answerText(picked: string | undefined, words: string): string {
  const w = words.trim();
  if (!picked) return w;
  return w ? `Выбран вариант: «${picked}»\n\n${w}` : `Выбран вариант: «${picked}»`;
}

/**
 * Why the merge button waits, as far as the web knows (the server checks approvals and
 * conflicts too): the request is no longer open, or the policy wants checks that are not green.
 */
export function mergeHeld(repo: string, crState: string | undefined, ci: string | undefined, requireCi: boolean): string | undefined {
  if (crState === "merged") return "Запрос уже слит.";
  if (crState === "closed") return "Запрос закрыт на хостинге.";
  if (requireCi && ci === "failed") return `Слить пока нельзя: политика ${repo} ждёт зелёных проверок, а они упали.`;
  if (requireCi && ci === "pending") return `Слить пока нельзя: политика ${repo} ждёт зелёных проверок, а они ещё идут.`;
  return undefined;
}

/**
 * Where an arrow key moves the roving focus of the answer radiogroup. A radiogroup is one tab stop and
 * the arrows walk it: left and right do what up and down do, and Home and End reach the ends.
 */
export function radioIndex(at: number, key: string, count: number): number {
  const mapped = key === "ArrowRight" ? "ArrowDown" : key === "ArrowLeft" ? "ArrowUp" : key;
  return nextIndex(at, mapped, count);
}
