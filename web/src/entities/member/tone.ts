/** Standard roles with a colour of their own (`--g-agent-<role>` in g-tokens.css). */
export const AGENT_TONES = ["executor", "analyst", "researcher", "reviewer", "tester", "documenter", "planner", "orchestrator", "project-manager"] as const;

const KNOWN = new Set<string>(AGENT_TONES);

/**
 * The colour of an agent's role: a standard role's own; a custom role takes the colour of the role it
 * extends, then of its process class, when `roles` knows it. `undefined` for a person or a role without one.
 */
export function agentTone(role: string, roles: { id: string; extends?: string; class?: string }[] = []): string | undefined {
  const seen = new Set<string>();
  for (let id: string | undefined = role; id && !seen.has(id); ) {
    if (KNOWN.has(id)) return id;
    seen.add(id);
    const def = roles.find((r) => r.id === id);
    id = def?.extends ?? (def?.class && def.class !== id ? def.class : undefined);
  }
  return undefined;
}
