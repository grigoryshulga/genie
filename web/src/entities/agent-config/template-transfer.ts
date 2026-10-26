import { THINKING_LEVELS } from "./role-transfer.ts";
import { templateFile, memberKey, type TemplateFile } from "./template-file.ts";
import { validateTransfer } from "./transfer-schema.ts";
import type { Catalogue, TeamDef } from "./model.ts";

export const TEMPLATE_FORMAT = "genie.team/v1";
const idRule = { type: "string", pattern: "^[a-z][a-z0-9-]*$" };
const text = { type: "string" };
const strings = { type: "array", items: { type: "string", minLength: 1 }, uniqueItems: true };
export const templateSchema = {
  $schema: "https://json-schema.org/draft/2020-12/schema", title: "Genie team template v1", type: "object", additionalProperties: false,
  required: ["format", "id", "title", "members"],
  properties: {
    $schema: text, format: { type: "string", enum: [TEMPLATE_FORMAT] }, id: idRule,
    title: { type: "string", minLength: 1 }, description: text,
    stage: { type: "string", enum: ["refinement", "delivery"] }, workspace: { type: "string", enum: ["worktree", "repo", "scratch"] }, mail: { type: "string", enum: ["open", "flow"] },
    charter: text, projects: strings,
    members: { type: "array", minItems: 1, items: { type: "object", additionalProperties: false, required: ["role"], properties: {
      role: idRule, key: idRule, name: { type: "string", pattern: "^[a-z][a-z0-9_-]*$" }, model: text, thinking: { type: "string", enum: THINKING_LEVELS }, instructions: text,
    } } },
    relations: { type: "array", items: { type: "object", additionalProperties: false, required: ["from", "to", "type"], properties: {
      from: idRule, to: { ...strings, minItems: 1, items: idRule }, type: { type: "string", enum: ["handoff", "returns", "reports", "consults"] },
      on: { type: "string", enum: ["inbox", "draft", "refining", "ready", "in_progress", "review", "changes_requested", "approved", "needs_owner", "done", "cancelled"] }, note: text,
    } } },
  },
};
export interface TemplateTransfer extends TemplateFile { format: typeof TEMPLATE_FORMAT; id: string }

export function parseTemplateTransfer(text: string): TemplateTransfer {
  if (text.length > 1_000_000) throw new Error("Файл шаблона должен быть не больше 1 МБ.");
  let value: unknown;
  try { value = JSON.parse(text); } catch { throw new Error("Не удалось прочитать JSON. Проверьте синтаксис файла."); }
  validateTransfer(value, templateSchema, "");
  return value as TemplateTransfer;
}

/** The server remains authoritative for stage permissions, cycles, limits and installed roles. */
export function templateTransferFile(value: TemplateTransfer, cfg: Pick<Catalogue, "roles">): TemplateFile {
  validateTransfer(value, { ...templateSchema, additionalProperties: true }, "");
  const { format: _format, id: _id, ...file } = value;
  const result = { ...file } as TemplateFile & { $schema?: string };
  delete result.$schema;
  const keys = value.members.map(memberKey);
  if (new Set(keys).size !== keys.length) throw new Error("Участники: ключи должны быть разными.");
  for (const member of value.members) {
    if (memberKey(member) === "orchestrator") throw new Error("Ключ orchestrator зарезервирован.");
    const role = cfg.roles.find((r) => r.id === member.role);
    if (!role || role.class === "orchestrator") throw new Error(`Недоступная роль: ${member.role}.`);
  }
  for (const r of value.relations ?? []) {
    if (!keys.includes(r.from)) throw new Error(`Связь: нет участника ${r.from}.`);
    if (r.on && !["handoff", "returns"].includes(r.type)) throw new Error("Условие статуса доступно только для передачи и возврата работы.");
    for (const to of r.to) {
      if (to === r.from || (to !== "orchestrator" && !keys.includes(to))) throw new Error(`Связь: недопустимый получатель ${to}.`);
      if ((r.type === "reports") !== (to === "orchestrator")) throw new Error("Доклады направляются менеджеру задачи; остальные связи — участникам.");
    }
  }
  return result;
}

export const exportTemplate = (t: TeamDef): TemplateTransfer => ({ format: TEMPLATE_FORMAT, id: t.id, ...templateFile(t) });
export const templateExample: TemplateTransfer = { format: TEMPLATE_FORMAT, id: "my-team", title: "Моя команда", stage: "delivery", workspace: "worktree", members: [{ role: "executor" }, { role: "reviewer" }] };
