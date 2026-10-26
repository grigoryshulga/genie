// Portable, resolved roles: no machine paths, history or dependency on a custom parent.
import { allowDeny, setBody, setFrontmatterKey, type Catalogue, type RoleDef } from "./model.ts";

export const ROLE_FORMAT = "genie.role/v1";
export const THINKING_LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh", "max"] as const;
const bases = ["analyst", "executor", "reviewer", "tester", "documenter", "orchestrator"];
const strings = { type: "array", items: { type: "string", minLength: 1 }, uniqueItems: true };

export const roleSchema = {
  $schema: "https://json-schema.org/draft/2020-12/schema",
  title: "Genie role v1",
  description: "Portable resolved role. Save this schema as genie-role.schema.json beside your role JSON. Import validates the role again on the Genie server.",
  type: "object",
  additionalProperties: false,
  required: ["format", "id", "title", "base", "prompt"],
  properties: {
    $schema: { type: "string" },
    format: { type: "string", enum: [ROLE_FORMAT] },
    id: { type: "string", pattern: "^[a-z][a-z0-9-]*$" },
    title: { type: "string", minLength: 1 },
    description: { type: "string" },
    base: { type: "string", enum: bases },
    prompt: { type: "string", minLength: 1, description: "The role's main system instructions, in Markdown." },
    instructions: { type: "string", description: "Additional instructions appended to the main prompt." },
    model: { type: "string", description: "provider/model identifier available on the target server." },
    thinking: { type: "string", enum: THINKING_LEVELS },
    files: { type: "string", enum: ["write", "read", "none"] },
    permissions: { ...strings, description: "Effective permission identifiers. Omit to use the base class defaults." },
    names: { ...strings, items: { type: "string", pattern: "^[a-z][a-z0-9_-]*$" } },
    denyCommands: strings,
    skills: { ...strings, description: "Omit to allow all installed skills; [] allows repository skills only." },
    mcp: strings,
    stages: { ...strings, items: { type: "string", enum: ["refinement", "delivery"] } },
    projects: strings,
  },
};

export interface RoleTransfer {
  format: typeof ROLE_FORMAT;
  id: string;
  title: string;
  base: RoleDef["class"];
  prompt: string;
  description?: string;
  instructions?: string;
  model?: string;
  thinking?: string;
  files?: RoleDef["files"];
  permissions?: string[];
  names?: string[];
  denyCommands?: string[];
  skills?: string[];
  mcp?: string[];
  stages?: RoleDef["stages"];
  projects?: string[];
}

type Rule = { type: string; enum?: readonly string[]; minLength?: number; pattern?: string; items?: Rule; uniqueItems?: boolean };
function validate(value: unknown, rule: Rule, field: string): void {
  if (rule.type === "string") {
    if (typeof value !== "string") throw new Error(`${field}: ожидается строка.`);
    if (rule.minLength && !value.trim().length) throw new Error(`${field}: заполните значение.`);
    if (rule.pattern && !new RegExp(rule.pattern).test(value)) throw new Error(`${field}: недопустимый формат.`);
    if (rule.enum && !rule.enum.includes(value)) throw new Error(`${field}: допустимо ${rule.enum.join(", ")}.`);
  } else {
    if (!Array.isArray(value)) throw new Error(`${field}: ожидается массив строк.`);
    for (const item of value) validate(item, rule.items ?? { type: "string" }, field);
    if (rule.uniqueItems && new Set(value).size !== value.length) throw new Error(`${field}: уберите повторяющиеся значения.`);
  }
}

/** This checked boundary uses the same constraints as the downloadable schema. */
export function parseRoleTransfer(text: string): RoleTransfer {
  if (text.length > 1_000_000) throw new Error("Файл роли должен быть не больше 1 МБ.");
  let value: unknown;
  try { value = JSON.parse(text); } catch { throw new Error("Не удалось прочитать JSON. Проверьте синтаксис файла."); }
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("Ожидается JSON-объект роли.");
  const record = value as Record<string, unknown>;
  for (const key of roleSchema.required) if (!(key in record)) throw new Error(`${key}: обязательное поле.`);
  for (const [key, v] of Object.entries(record)) {
    const rule = Object.entries(roleSchema.properties).find(([name]) => name === key)?.[1];
    if (!rule) throw new Error(`Неизвестное поле: ${key}.`);
    validate(v, rule, key);
  }
  return record as unknown as RoleTransfer;
}

export function exportRole(role: RoleDef): RoleTransfer {
  return {
    format: ROLE_FORMAT, id: role.id, title: role.title, base: role.class,
    description: role.description, prompt: role.prompt ?? "", instructions: role.instructions,
    model: role.model, thinking: role.thinking, files: role.files,
    permissions: role.capabilities, names: role.names, denyCommands: role.denyCommands,
    skills: role.skills, mcp: role.mcp, stages: role.stages, projects: role.projects,
  };
}

export function roleTransferFile(role: RoleTransfer, cfg: Pick<Catalogue, "permissions" | "classes">): string {
  const unknown = role.permissions?.filter((p) => !cfg.permissions.includes(p)) ?? [];
  if (unknown.length) throw new Error(`Неизвестные разрешения: ${unknown.join(", ")}.`);
  let file = "---\n---\n";
  const fields: [string, string | string[] | undefined][] = [
    ["title", role.title], ["description", role.description],
    // Orchestrator classes are fixed by Genie; custom copies inherit the built-in role.
    [role.base === "orchestrator" ? "extends" : "base", role.base === "orchestrator" ? (["orchestrator", "project-manager"].includes(role.id) ? undefined : "orchestrator") : role.base],
    ...["model", "thinking", "files", "names", "denyCommands", "skills", "mcp", "stages", "projects", "instructions"].map((key): [string, string | string[] | undefined] => [key, role[key as keyof RoleTransfer]]),
  ];
  if (role.permissions) {
    const { allow, deny } = allowDeny(cfg.classes[role.base] ?? [], role.permissions, cfg.permissions);
    fields.push(["allow", allow], ["deny", deny]);
  }
  for (const [key, value] of fields) if (value !== undefined) file = setFrontmatterKey(file, key, value);
  return setBody(file, role.prompt);
}

export const roleTemplate: RoleTransfer = {
  format: ROLE_FORMAT, id: "my-analyst", title: "Мой аналитик", base: "analyst",
  model: "provider/model", files: "read", stages: ["refinement"], mcp: [],
  prompt: "You are the analyst of the team. Investigate the task and prepare a verifiable implementation plan.",
};
