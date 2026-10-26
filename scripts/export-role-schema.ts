// Regenerate the public artifacts from the schemas their importers validate.
import { readFileSync, writeFileSync } from "node:fs";
import { roleSchema } from "../web/src/entities/agent-config/role-transfer.ts";
import { templateSchema } from "../web/src/entities/agent-config/template-transfer.ts";
for (const [name, schema] of [["genie-role", roleSchema], ["genie-team", templateSchema]] as const) {
  const path = new URL(`../web/public/${name}.schema.json`, import.meta.url);
  const text = JSON.stringify(schema, null, 2) + "\n";
  if (process.argv.includes("--check")) {
    if (readFileSync(path, "utf8") !== text) throw new Error(`${name} schema is stale: run node scripts/export-role-schema.ts`);
  } else writeFileSync(path, text);
}
