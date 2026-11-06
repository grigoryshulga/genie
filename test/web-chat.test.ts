// The project chat's pure logic: the @ completion, grouping, the manager's answering state and merging pages.

import assert from "node:assert/strict";
import { test } from "node:test";
import type { ChatMessage } from "../web/src/shared/api/generated/ChatMessage.ts";
import { keysFor, matchesAny } from "../web/src/shared/api/invalidation.ts";
import {
  authorLabel,
  chatRows,
  excerpt,
  insertMention,
  managerAnswering,
  mayHaveOlder,
  mentionAt,
  mentionOptions,
  mergeMessages,
  nextOption,
  quotedMessage,
} from "../web/src/pages/chat/model.ts";

const at = (minute: number) => `2026-05-01T12:${String(minute).padStart(2, "0")}:00Z`;
const msg = (id: number, author: string, authorRole: ChatMessage["authorRole"], text: string, over: Partial<ChatMessage> = {}): ChatMessage => ({
  id,
  at: at(id),
  author,
  authorRole,
  text,
  mentions: [],
  replyTo: null,
  ...over,
});

test("a posted message refetches the chat queries and nothing else", () => {
  const posted = { type: "chat.posted", payload: { id: 1, mentions: ["manager"] } };
  assert.ok(matchesAny(keysFor(posted) ?? [], ["chat", "latest"]), "the newest page is refetched");
  assert.equal(matchesAny(keysFor(posted) ?? [], ["tasks"]), false);
  assert.equal(matchesAny(keysFor(posted) ?? [], ["task", "GNE-1"]), false);
});

test("an @ completes the word before the caret, only where a word starts", () => {
  assert.deepEqual(mentionAt("@", 1), { start: 0, query: "" });
  assert.deepEqual(mentionAt("спросите @pm", 12), { start: 9, query: "pm" });
  assert.deepEqual(mentionAt("@pm and @ow", 11), { start: 8, query: "ow" }, "the caret decides which @ counts");
  assert.equal(mentionAt("mail@example", 12), null, "an @ inside a word is not a mention");
  assert.equal(mentionAt("@pm done", 8), null, "a space after the token closes it");
  assert.deepEqual(mentionAt("@менеджер", 9), { start: 0, query: "менеджер" });
});

test("the manager comes first and any word of an option can match the query", () => {
  const people = [
    { login: "owner", name: "Анна Смирнова" },
    { login: "boris", name: "Борис" },
  ];
  assert.deepEqual(
    mentionOptions(people, "").map((o) => o.key),
    ["pm", "owner", "boris"],
  );
  assert.equal(mentionOptions(people, "")[0]?.label, "@pm — менеджер проекта");
  assert.deepEqual(mentionOptions(people, "PM").map((o) => o.key), ["pm"], "the query ignores case");
  assert.deepEqual(mentionOptions(people, "менедж").map((o) => o.key), ["pm"], "the manager also answers to its Russian name");
  assert.deepEqual(mentionOptions(people, "смир").map((o) => o.key), ["owner"], "a member is found by the name they show");
  assert.deepEqual(mentionOptions(people, "zzz"), [], "nothing matches: no list");
});

test("picking an option replaces the @ token, keeps the text after the caret and parks the caret after the space", () => {
  const text = "@p and more";
  const token = mentionAt(text, 2);
  assert.ok(token);
  assert.deepEqual(insertMention(text, 2, token, "pm"), { text: "@pm  and more", caret: 4 });

  const typed = "Скажи @ow";
  const picked = mentionAt(typed, typed.length);
  assert.ok(picked);
  assert.deepEqual(insertMention(typed, typed.length, picked, "owner"), { text: "Скажи @owner ", caret: 13 });
});

test("arrow keys wrap around the option list", () => {
  assert.equal(nextOption(0, 3, -1), 2);
  assert.equal(nextOption(2, 3, 1), 0);
  assert.equal(nextOption(0, 0, 1), 0, "an empty list has no active option");
});

test("the manager is answering while it has chat mail, or while it works on a message tagging it", () => {
  const asked = msg(5, "owner", "human", "@pm статус?", { mentions: ["manager"] });
  const reply = msg(6, "pm", "orchestrator", "Готово");
  const idle = { state: "idle", pending: 0 };

  assert.equal(managerAnswering([asked], { state: null, pending: 1 }), true, "chat mail waits for it");
  assert.equal(managerAnswering([asked], { state: "working", pending: 0 }), true, "working on a tagged message");
  assert.equal(managerAnswering([asked, reply], { state: "working", pending: 0 }), false, "its reply came after the tag");
  assert.equal(managerAnswering([asked], idle), false, "idle is not answering");
  assert.equal(managerAnswering([msg(1, "owner", "human", "hello")], { state: "working", pending: 0 }), false, "working without a tag is not a question to it");
  assert.equal(managerAnswering([asked], undefined), false);
});

test("messages group under one header while the same author writes within five minutes", () => {
  const messages = [
    msg(1, "owner", "human", "a"),
    msg(2, "owner", "human", "b"),
    msg(3, "pm", "orchestrator", "c"),
    msg(7, "pm", "orchestrator", "d"),
    msg(20, "pm", "orchestrator", "e"),
  ];
  assert.deepEqual(
    chatRows(messages).map((r) => r.continued),
    [false, true, false, true, false],
    "a change of author starts a group; a gap of more than five minutes starts a new one",
  );
});

test("pages merge by id, once each, and a fresh copy wins", () => {
  const older = [msg(1, "a", "human", "old"), msg(2, "b", "human", "old-2")];
  const newest = [msg(3, "a", "human", "new"), msg(2, "b", "human", "edited")];
  const merged = mergeMessages(older, newest);
  assert.deepEqual(
    merged.map((m) => [m.id, m.text]),
    [
      [1, "old"],
      [2, "edited"],
      [3, "new"],
    ],
  );
});

test("a reply quotes its message only while that message is loaded", () => {
  const first = msg(1, "owner", "human", "first");
  const reply = msg(2, "pm", "orchestrator", "answer", { replyTo: 1 });
  assert.equal(quotedMessage([first, reply], 1), first);
  assert.equal(quotedMessage([reply], 1), undefined, "the original is on an older page");
  assert.equal(quotedMessage([reply], null), undefined);
});

test("a quote is the first line of the text, cut with an ellipsis, its markup shown as plain words", () => {
  assert.equal(excerpt("\n\n  Первая строка  \nвторая"), "Первая строка");
  assert.equal(excerpt("**Важно** — `код`"), "Важно — код");
  assert.equal(excerpt("- пункт списка"), "пункт списка");
  assert.equal(excerpt("## Заголовок"), "Заголовок");
  assert.equal(excerpt("abcdef", 4), "abc…");
  assert.equal(excerpt("abc", 4), "abc");
});

test("the manager's messages carry its label, people show their display name, others their login", () => {
  const names = new Map([["owner", "Анна Смирнова"]]);
  const label = "Менеджер проекта";
  assert.equal(authorLabel(msg(1, "orchestrator", "orchestrator", "x"), names, label), label);
  assert.equal(authorLabel(msg(2, "owner", "human", "x"), names, label), "Анна Смирнова");
  assert.equal(authorLabel(msg(3, "boris", "human", "x"), names, label), "boris");
});

test("an older page is asked for while the last page was full", () => {
  assert.equal(mayHaveOlder(true, undefined), true, "the newest page was full");
  assert.equal(mayHaveOlder(false, undefined), false, "the newest page was short: nothing older");
  assert.equal(mayHaveOlder(true, false), false, "an older page came back short: that was the oldest");
  assert.equal(mayHaveOlder(false, true), true);
});
