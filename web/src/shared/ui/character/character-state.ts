// Which pose the Genie character takes, from what agents are really doing. One state per view; the
// character never invents activity: with no signal it sleeps.
//   thinking: at least one agent is working; surprise: something needs the owner;
//   waking: work has just resumed after being idle; sleeping: nothing is going on.

export type CharacterState = "sleep" | "wake" | "think" | "surprise";

export function characterState({ working, waiting, resumed }: { working: number; waiting: number; resumed?: boolean }): CharacterState {
  if (waiting > 0) return "surprise";
  if (working > 0) return resumed ? "wake" : "think";
  return "sleep";
}

export const CHARACTER_LABEL: Record<CharacterState, string> = {
  sleep: "Агенты отдыхают",
  wake: "Агенты приступают к работе",
  think: "Агенты работают",
  surprise: "Нужно ваше решение",
};
