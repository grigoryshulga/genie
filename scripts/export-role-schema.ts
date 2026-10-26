// Regenerate the public artifact from the same schema the role importer validates.
import { readFileSync, writeFileSync } from "node:fs";
import { roleSchema } from "../web/src/entities/agent-config/role-transfer.ts";
const path = new URL("../web/public/genie-role.schema.json", import.meta.url);
const text = JSON.stringify(roleSchema, null, 2) + "\n";
if (process.argv.includes("--check")) {
  if (readFileSync(path, "utf8") !== text) throw new Error("Role schema is stale: run node scripts/export-role-schema.ts");
} else writeFileSync(path, text);
