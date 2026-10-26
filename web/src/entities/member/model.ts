// UI-side view of team members. Types come straight from the server code, so the
// API contract is checked by the compiler on both sides.

import type { Member, Role } from "../../shared/api/types.ts";

export type { Member };
export type Activity = Member["activity"];
/** A process class; a member's `role` is a role id of the server and may be a custom role. */
export type MemberRole = Role;

export { displayName, initial, memberLabel, ROLE_TITLE_RU } from "./names.ts";
export { AGENT_TONES, agentTone } from "./tone.ts";

export const ROLE_LETTER: Record<string, string> = { analyst: "A", executor: "E", reviewer: "R", tester: "T", documenter: "D", orchestrator: "O", human: "Я" };
