import { CHARACTER_LABEL, type CharacterState } from "./character-state.ts";
import sleep from "./genie-sleep.svg";
import surprise from "./genie-surprise.svg";
import think from "./genie-think.svg";
import wake from "./genie-wake.svg";

const ART: Record<CharacterState, string> = { sleep, wake, think, surprise };

/**
 * The character: the logo mark that reports what the agents are doing. Use one per view, in status
 * panels and empty states, never in dense lists. The animation is an SVG with its own CSS, so it
 * runs inside <img>; the SVG carries its own prefers-reduced-motion rule, so it is static for those users.
 */
export function Character({ state, size = 32, label }: { state: CharacterState; size?: number; label?: string }) {
  return <img className="g-character" src={ART[state]} width={size} height={Math.round(size * (780 / 880))} alt={label ?? CHARACTER_LABEL[state]} data-state={state} />;
}
