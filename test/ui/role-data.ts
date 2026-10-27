// A long explicit grant list, an entire connection and an unavailable connection.
// Shared by the development stand and the role interaction regression.
export const ROLE_GRANTS = [
  ...Array.from({ length: 48 }, (_, i) => `knowledge:read_document_${i + 1}`),
  "issues",
  "archived:read_*",
];

export const ROLE_FILE = `---
title: Аналитик интеграций
base: analyst
model: anthropic/claude-sonnet-4-6
thinking: medium
description: Изучает документацию и задачи, готовит план для команды.
files: read
denyCommands: ["git push*", "rm -rf*"]
stages: [refinement, delivery]
names: [sherlock, poirot]
mcp: ${JSON.stringify(ROLE_GRANTS)}
instructions: Сначала изучи связанные документы, затем сформулируй проверяемый план.
---
Investigate the task and prepare a plan. Do not change project files.
`;
