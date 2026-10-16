export type DecisionMode = "off" | "observe" | "advise" | "joint" | "select";
export interface DecisionProfile {
  id: string;
  description: string;
  model: string;
  thinking: string | null;
  roles: string[];
}
export interface DecisionPolicy {
  allowExternal: boolean;
  toolEnabled: boolean;
  toolRoles: string[];
  team: DecisionMode;
  model: DecisionMode;
  thinking: DecisionMode;
  templates: string[];
  profiles: DecisionProfile[];
  minConfidence: number;
  minMargin: number;
  maxCallsPerTask: number;
  maxCostPerTaskUsd: number;
}
export interface DecisionSettings {
  policy: DecisionPolicy;
  available: boolean;
  model: string;
  inputPrice: number;
}
export interface DecisionRecord {
  id: number;
  task: string;
  actor: string;
  purpose: string;
  status: string;
  created: string;
  costUsd: number | null;
  reservedUsd: number;
  elapsedMs: number;
  request: { state?: unknown; questions?: Record<string, { instructions: string; criteria: unknown }> };
  result: { model?: string; error?: string | null; answers?: Record<string, { choice?: string; confidence?: number; probability?: number; score?: number; qualified?: boolean; probabilities?: Record<string, number> }> };
  application: { state?: string; team?: string; override?: string | null; by?: string };
}
