// The board's lanes and the things its headline says, without a browser: the lane order and names
// the owner decided, what a card becomes where it is dropped, and what the hero states.

import test from "node:test";
import assert from "node:assert/strict";

import { LANES, groupByLane, laneDropId, laneOf, moveTargets, needsQuestion, resolveDrop, statusDropId } from "../web/src/entities/task/lanes.ts";
import { boardHeadline, boardStrip } from "../web/src/pages/board/model.ts";
import { STATUS_ORDER } from "../web/src/entities/task/model.ts";

const task = (id: string, status: string) => ({ id, status }) as never;

test("the six lanes are the owner's six, in the owner's order", () => {
  assert.deepEqual(
    LANES.map((l) => l.name),
    ["Может быть", "Нужно решение", "Подготовка", "Очередь", "В работе", "Готово"],
  );
  assert.deepEqual(
    LANES.map((l) => l.id),
    ["maybe", "decision", "prep", "queue", "work", "done"],
  );
});

test("every status except cancelled lives in exactly one lane", () => {
  const placed = LANES.flatMap((l) => l.statuses);
  assert.equal(new Set(placed).size, placed.length, "a status is in two lanes");
  for (const status of STATUS_ORDER) {
    if (status === "cancelled") assert.equal(laneOf(status), undefined, "cancelled belongs to no lane");
    else assert.ok(laneOf(status), `${status} belongs to a lane`);
  }
});

test("drop chips are the exact statuses each lane offers", () => {
  assert.deepEqual(LANES.find((l) => l.id === "prep")?.drops, ["draft", "refining"]);
  assert.deepEqual(LANES.find((l) => l.id === "work")?.drops, ["in_progress", "review"]);
  assert.deepEqual(LANES.find((l) => l.id === "done")?.drops, ["approved", "done"]);
  for (const lane of LANES) for (const status of lane.drops) assert.ok(lane.statuses.includes(status), `${status} chip sits in its own lane`);
});

test("a drop on a lane takes the card to that lane's default status, never sideways", () => {
  const prep = LANES.find((l) => l.id === "prep")!;
  assert.equal(resolveDrop(laneDropId(prep), "inbox"), "refining");
  assert.equal(resolveDrop(laneDropId(prep), "draft"), undefined, "already in this lane");
  assert.equal(resolveDrop(undefined, "inbox"), undefined, "released outside the board");
  assert.equal(resolveDrop("nonsense", "inbox"), undefined);
});

test("a drop on a status chip takes exactly that status", () => {
  assert.equal(resolveDrop(statusDropId("draft"), "inbox"), "draft");
  assert.equal(resolveDrop(statusDropId("review"), "in_progress"), "review");
  assert.equal(resolveDrop(statusDropId("review"), "review"), undefined, "already there");
});

test("the move menu offers every other status by lane", () => {
  const inbox = moveTargets("inbox");
  assert.equal(inbox.length, LANES.reduce((n, l) => n + l.drops.filter((s) => s !== "inbox").length, 0));
  assert.ok(!inbox.some((m) => m.status === "inbox"));
  assert.deepEqual(inbox.find((m) => m.status === "needs_owner")?.lane.name, "Нужно решение");
});

test("asking the owner is the only move that needs a question", () => {
  assert.equal(needsQuestion("needs_owner"), true);
  for (const m of moveTargets("inbox")) if (m.status !== "needs_owner") assert.equal(needsQuestion(m.status), false);
});

test("grouping keeps lane order and knows the cancelled ones", () => {
  const { lanes, cancelled } = groupByLane([task("T-3", "review"), task("T-1", "inbox"), task("T-2", "cancelled"), task("T-4", "draft")]);
  assert.deepEqual(lanes.maybe.map((t) => t.id), ["T-1"]);
  assert.deepEqual(lanes.work.map((t) => t.id), ["T-3"]);
  assert.deepEqual(lanes.prep.map((t) => t.id), ["T-4"]);
  assert.deepEqual(cancelled.map((t) => t.id), ["T-2"]);
  assert.deepEqual(lanes.queue, []);
});

test("the headline puts the owner's decision first and only then wears the orange stop", () => {
  const waiting = boardHeadline({ waiting: 1, working: 2, review: 0, open: 5 });
  assert.equal(waiting.title, "1 решение ждёт вас");
  assert.equal(waiting.lead, "Остальное идёт само: 2 команды работают.");
  assert.equal(waiting.stop, true);

  const two = boardHeadline({ waiting: 2, working: 0, review: 3, open: 9 });
  assert.equal(two.title, "2 решения ждут вас");
  assert.equal(two.lead, "Остальное идёт само: 3 на ревью.");
  assert.equal(boardHeadline({ waiting: 1, working: 0, review: 0, open: 1 }).lead, "Больше ничего не требует вашего внимания.");
});

test("without a waiting decision the hero says what is going on instead", () => {
  const self = boardHeadline({ waiting: 0, working: 2, review: 1, open: 7 });
  assert.equal(self.title, "Всё идёт само");
  assert.equal(self.stop, false);
  assert.equal(boardHeadline({ waiting: 0, working: 0, review: 0, open: 4 }).lead, "Решений от вас не нужно. Открытых задач: 4, агенты свободны.");
  assert.equal(boardHeadline({ waiting: 0, working: 0, review: 0, open: 0 }).title, "Здесь тихо");
});

test("the strip speaks for the character's state", () => {
  const pulse = { waiting: 2, working: 1, review: 0, open: 3 };
  assert.equal(boardStrip("surprise", pulse).say, "Нужно ваше решение");
  assert.equal(boardStrip("surprise", pulse).sub, "Ждут ответа: 2");
  assert.equal(boardStrip("surprise", pulse, { id: "GNE-9", title: "Fix the thing" }).say, "Нужно ваше решение: GNE-9");
  assert.equal(boardStrip("think", pulse).say, "Агенты работают");
  assert.equal(boardStrip("sleep", { waiting: 0, working: 0, review: 0, open: 0 }).say, "Всё спокойно. Genie ждёт новых задач");
});
