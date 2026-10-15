import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";
import { transformWithOxc } from "vite";

test("task focus effect does not return the browser's scroll promise as cleanup", async () => {
  const source = readFileSync(new URL("../web/src/widgets/task-list/ui/TaskList.tsx", import.meta.url), "utf8");
  const transformed = (await transformWithOxc(source, "TaskList.tsx", { jsx: { runtime: "automatic" } })).code;
  const compiled = transformed.replace(/import \{([^}]+)\} from (["'][^"']+["']);/g, (_, names, id) =>
    `const {${names.replace(/ as /g, ": ")}} = require(${id});`,
  ).replace("export function TaskList", "function TaskList") + "\nexports.TaskList = TaskList;";
  for (const present of [false, true]) {
    let effect: (() => unknown) | undefined;
    let scrolled = false;
    const exports: Record<string, Function> = {};
    runInNewContext(compiled, {
      exports,
      require: (id: string) => {
        if (id === "react") return {
          useRef: () => ({ current: present ? { scrollIntoView: () => { scrolled = true; return Promise.resolve(); } } : null }),
          useEffect: (callback: () => unknown) => { effect = callback; },
        };
        if (id === "react/jsx-runtime") return { jsx: () => null, jsxs: () => null };
        return {};
      },
    });
    exports.TaskList({ tasks: [], statuses: [], grouped: false, teams: new Map(), focused: "task-1" });
    assert.ok(effect);
    const destroy = effect();
    assert.equal(scrolled, present);
    // React calls every defined effect return value when the component unmounts.
    if (destroy !== undefined) (destroy as () => void)();
    assert.equal(destroy, undefined);
  }
});
