/** Small shared validator for the constraints in our downloadable JSON Schemas. */
export interface TransferRule {
  type: string;
  enum?: readonly string[];
  minLength?: number;
  pattern?: string;
  items?: TransferRule;
  uniqueItems?: boolean;
  minItems?: number;
  required?: readonly string[];
  properties?: Record<string, TransferRule>;
  additionalProperties?: boolean;
}

export function validateTransfer(value: unknown, rule: TransferRule, field: string): void {
  if (rule.type === "string") {
    if (typeof value !== "string") throw new Error(`${field}: ожидается строка.`);
    if (rule.minLength && !value.trim().length) throw new Error(`${field}: заполните значение.`);
    if (rule.pattern && !new RegExp(rule.pattern).test(value)) throw new Error(`${field}: недопустимый формат.`);
    if (rule.enum && !rule.enum.includes(value)) throw new Error(`${field}: допустимо ${rule.enum.join(", ")}.`);
  } else if (rule.type === "array") {
    if (!Array.isArray(value)) throw new Error(`${field}: ожидается массив.`);
    if (rule.minItems && value.length < rule.minItems) throw new Error(`${field}: добавьте хотя бы ${rule.minItems} элемент.`);
    value.forEach((item, index) => validateTransfer(item, rule.items ?? { type: "string" }, `${field}[${index}]`));
    if (rule.uniqueItems && new Set(value.map((item) => JSON.stringify(item))).size !== value.length) throw new Error(`${field}: уберите повторяющиеся значения.`);
  } else if (rule.type === "object") {
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`${field}: ожидается JSON-объект.`);
    const record = value as Record<string, unknown>;
    for (const key of rule.required ?? []) if (!(key in record)) throw new Error(`${field ? field + "." : ""}${key}: обязательное поле.`);
    for (const [key, item] of Object.entries(record)) {
      const path = field ? `${field}.${key}` : key;
      const child = rule.properties && Object.hasOwn(rule.properties, key) ? rule.properties[key] : undefined;
      if (child) validateTransfer(item, child, path);
      else if (rule.additionalProperties === false) throw new Error(`Неизвестное поле: ${path}.`);
    }
  }
}
