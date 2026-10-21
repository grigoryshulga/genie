// What the board headline says, from what is really going on in the project.

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
