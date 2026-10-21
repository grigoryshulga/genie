// What the board's headline and status strip say, from what is really going on in the project.

import type { CharacterState } from "../../shared/ui/character/character-state.ts";
import { plural } from "../../shared/lib/format.ts";

export interface BoardPulse {
  /** Tasks waiting for the owner's decision. */
  waiting: number;
  /** Teams with an agent working right now. */
  working: number;
  /** Tasks in review. */
  review: number;
  /** Tasks that are not finished. */
  open: number;
}

/** The hero: a headline that states the project, the orange stop only while something waits for the owner. */
export function boardHeadline({ waiting, working, review, open }: BoardPulse): { title: string; lead: string; stop: boolean } {
  const parts = [
    working > 0 ? `${working} ${plural(working, "команда работает", "команды работают", "команд работают")}` : "",
    review > 0 ? `${review} на ревью` : "",
  ].filter(Boolean);
  if (waiting > 0) {
    return {
      title: `${waiting} ${plural(waiting, "решение ждёт", "решения ждут", "решений ждут")} вас`,
      lead: parts.length ? `Остальное идёт само: ${parts.join(", ")}.` : "Больше ничего не требует вашего внимания.",
      stop: true,
    };
  }
  if (parts.length) return { title: "Всё идёт само", lead: `Решений от вас не нужно. ${parts.join(", ").replace(/^./, (c) => c.toUpperCase())}.`, stop: false };
  if (open > 0) return { title: "Ничего не ждёт вас", lead: `Решений от вас не нужно. Открытых задач: ${open}, агенты свободны.`, stop: false };
  return { title: "Здесь тихо", lead: "Открытых задач нет. Создайте новую, и Genie возьмётся за дело.", stop: false };
}

/** The line the character says in the status strip, and a second line under it. */
export function boardStrip(state: CharacterState, { working, waiting }: BoardPulse, firstWaiting?: { id: string; title: string }): { say: string; sub: string } {
  if (state === "surprise") {
    return {
      say: firstWaiting ? `Нужно ваше решение: ${firstWaiting.id}` : "Нужно ваше решение",
      sub: firstWaiting && waiting === 1 ? firstWaiting.title : `Ждут ответа: ${waiting}`,
    };
  }
  if (state === "think" || state === "wake") {
    return { say: "Агенты работают", sub: `${working} ${plural(working, "команда в работе", "команды в работе", "команд в работе")}` };
  }
  return { say: "Всё спокойно. Genie ждёт новых задач", sub: "Нет активных агентов" };
}
