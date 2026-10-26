// The avatar shape contract: a rounded square is an agent, a circle is a person. Read from the source
// because the class is picked in a React component and the shape lives in the entity's own sheet.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";

const WEB_SRC = path.resolve(import.meta.dirname, "../web/src");
const read = (file: string) => readFileSync(path.join(WEB_SRC, file), "utf8");

test("member avatars are shaped by role", () => {
  const css = read("entities/member/ui/avatar.css");
  const agent = /\.av\.agent\s*\{([^}]*)\}/.exec(css);
  const human = /\.av\.human\s*\{([^}]*)\}/.exec(css);
  assert.ok(agent, "the sheet styles agent avatars");
  assert.ok(human, "the sheet styles people avatars");
  assert.match(agent[1], /border-radius:\s*25%/);
  assert.match(human[1], /border-radius:\s*50%/);
  // The working ring follows the box, so it is not a circle drawn around a square.
  assert.match(css, /\.av\.agent\.working::after\s*\{[^}]*border-radius:\s*inherit/);

  const source = read("entities/member/ui/Avatar.tsx");
  assert.match(source, /const shape = role === "human" \? "human" : "agent"/);
  assert.match(source, /\["av", shape,/);
  // Everyone who is not a person is an agent, the orchestrator included.
  assert.doesNotMatch(source, /orchestrator"\s*\?\s*"human"/);
});

test("the surfaces that show member avatars take them from that component", () => {
  for (const file of ["widgets/task-list/ui/TaskList.tsx", "widgets/task-detail/ui/TaskDetail.tsx", "pages/epic/ui/EpicPage.tsx"]) {
    const source = read(file);
    assert.match(source, /@\/entities\/member/, `${file} takes the avatars from the member entity`);
    assert.match(source, /<Avatars\b/, `${file} renders the member avatars`);
  }
});

test("an agent's colour follows its role, a custom role the role it extends", async () => {
  const { agentTone, AGENT_TONES } = await import("../web/src/entities/member/tone.ts");
  for (const role of AGENT_TONES) assert.equal(agentTone(role), role);
  const roles = [
    { id: "abap-dev", extends: "executor", class: "executor" },
    { id: "qa", class: "tester" },
    { id: "loop-a", extends: "loop-b" },
    { id: "loop-b", extends: "loop-a" },
  ];
  assert.equal(agentTone("abap-dev", roles), "executor");
  assert.equal(agentTone("qa", roles), "tester", "without a base the process class decides");
  assert.equal(agentTone("loop-a", roles), undefined, "a cycle ends without a colour");
  assert.equal(agentTone("human"), undefined, "people keep their own look");
  assert.equal(agentTone("custom"), undefined, "an unknown role keeps the accent");
});
