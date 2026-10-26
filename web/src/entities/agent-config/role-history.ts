import { splitFrontmatter, type ConfigChange } from "./model.ts";

/** Preserve multiline YAML values and prompt text when comparing recorded snapshots. */
function fields(text: string | undefined): Map<string, string> {
  const result = new Map<string, string>();
  if (text === undefined) return result;
  const fm = splitFrontmatter(text);
  let key: string | undefined;
  for (const line of fm?.lines ?? []) {
    const match = /^([\w-]+):\s*(.*)$/.exec(line);
    if (match) { key = match[1]; result.set(key, match[2]); }
    else if (key) result.set(key, `${result.get(key)}\n${line}`);
  }
  result.set("prompt", fm?.body ?? text);
  return result;
}

export function roleChangeFields(change: Pick<ConfigChange, "before" | "after">) {
  const before = fields(change.before), after = fields(change.after);
  const changes = [...new Set([...before.keys(), ...after.keys()])]
    .filter((key) => before.get(key) !== after.get(key))
    .map((key) => ({ key, before: before.get(key), after: after.get(key) }));
  return changes.length || change.before === change.after ? changes : [{ key: "file", before: change.before, after: change.after }];
}
