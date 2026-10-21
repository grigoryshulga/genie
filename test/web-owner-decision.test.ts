// The decision box of a task waiting for the owner: what the buttons send and when the merge waits.

import assert from "node:assert/strict";
import { test } from "node:test";
import { answerText, cardKind, mergeHeld, radioIndex } from "../web/src/features/owner-decision/model.ts";

test("a picked option goes first, the words after it", () => {
  assert.equal(answerText("Сохранять", ""), "Выбран вариант: «Сохранять»");
  assert.equal(answerText("Сохранять", "  до 30 дней "), "Выбран вариант: «Сохранять»\n\nдо 30 дней");
  assert.equal(answerText(undefined, " CSV "), "CSV");
  assert.equal(answerText(undefined, "  "), "", "nothing to send");
});

test("an action this web does not know shows as a plain question", () => {
  assert.equal(cardKind({ kind: "ask-owner-question" }), "ask-owner-question");
  assert.equal(cardKind({ kind: "ask-for-merge-pr" }), "ask-for-merge-pr");
  assert.equal(cardKind({ kind: "ask-free-form" }), undefined);
  assert.equal(cardKind({ kind: "ask-doc-change" }), undefined, "a kind from a newer server");
  assert.equal(cardKind(undefined), undefined);
});

test("the merge button waits for what the policy asks and says why", () => {
  assert.equal(mergeHeld("api", "open", "passed", true), undefined);
  assert.equal(mergeHeld("api", "open", "none", true), undefined, "no checks on the host: the server decides");
  assert.match(mergeHeld("api", "open", "pending", true) ?? "", /политика api ждёт зелёных проверок, а они ещё идут/);
  assert.match(mergeHeld("api", "open", "failed", true) ?? "", /упали/);
  assert.equal(mergeHeld("api", "open", "failed", false), undefined, "checks are not required here");
  assert.equal(mergeHeld("api", "merged", "passed", true), "Запрос уже слит.");
  assert.equal(mergeHeld("api", "closed", "passed", true), "Запрос закрыт на хостинге.");
});

test("the answer radiogroup walks with every arrow and keeps one tab stop", () => {
  // Down and right go forward, up and left go back, and both ends wrap: one tab stop, arrows inside it.
  assert.equal(radioIndex(0, "ArrowDown", 3), 1);
  assert.equal(radioIndex(0, "ArrowRight", 3), 1);
  assert.equal(radioIndex(1, "ArrowUp", 3), 0);
  assert.equal(radioIndex(1, "ArrowLeft", 3), 0);
  assert.equal(radioIndex(2, "ArrowDown", 3), 0, "the last option wraps to the first");
  assert.equal(radioIndex(0, "ArrowUp", 3), 2, "the first wraps to the last");
  assert.equal(radioIndex(1, "Home", 3), 0);
  assert.equal(radioIndex(1, "End", 3), 2);
  // A key that is not a navigation key leaves the focus where it is, so Space and Enter stay the button's.
  assert.equal(radioIndex(1, "Enter", 3), 1);
  assert.equal(radioIndex(1, " ", 3), 1);
  assert.equal(radioIndex(-1, "ArrowDown", 3), 0, "nothing focused yet starts at the first option");
  assert.equal(radioIndex(0, "ArrowDown", 0), -1, "an empty group has nowhere to go");
});
