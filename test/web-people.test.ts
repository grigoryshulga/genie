// People in the web: who can be responsible for a task.

import assert from "node:assert/strict";
import { test } from "node:test";
import { doctorSummary, hoursText, initials, type Membership, responsibleChoices } from "../web/src/entities/project/model.ts";

const person = (login: string, name = "", disabled = false) => ({ id: login.length, login, name, isAdmin: false, disabled, created: "" });

test("people who can be responsible: members who write, the current one kept", () => {
  const members: Membership[] = [
    { user: person("anna", "Анна"), role: "admin" },
    { user: person("vic"), role: "viewer" },
    { user: person("gone", "", true), role: "member" },
    { user: person("boris"), role: "member" },
  ];
  assert.deepEqual(
    responsibleChoices(members).map((c) => c.label),
    ["Анна (@anna)", "@boris"],
    "viewers and disabled people are not offered",
  );
  assert.deepEqual(
    responsibleChoices(members, "gone").map((c) => c.login),
    ["gone", "anna", "boris"],
    "the person responsible stays visible after they left",
  );
  assert.equal(initials("Анна Петрова"), "АП");
  assert.equal(initials("boris"), "BO");
});

test("the server page speaks in words: hours and readiness", () => {
  assert.equal(hoursText(null), "—");
  assert.equal(hoursText(0.25), "15 мин");
  assert.equal(hoursText(5.5), "5,5 ч");
  assert.equal(hoursText(72), "3,0 дн");
  assert.deepEqual(doctorSummary([{ area: "web", level: "ok", text: "" }]), { level: "ok", text: "Всё готово" });
  assert.deepEqual(
    doctorSummary([
      { area: "pi", level: "fail", text: "" },
      { area: "channels", level: "warn", text: "" },
    ]),
    { level: "fail", text: "Нужно исправить: 1, предупреждений: 1" },
  );
});
