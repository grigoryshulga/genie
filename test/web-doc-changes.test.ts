// The Changes feed and page marks: pure logic behind the documentation changeset
// screens. Diff classification, filter state, markers, commit trailers and the
// status-aware error wording.

import assert from "node:assert/strict";
import { test } from "node:test";
import {
  attentionChangesets,
  canPublishChangeset,
  canReviewChangeset,
  changesFilterFromParams,
  changesFilterOptions,
  changesFilterToParams,
  changesServerQuery,
  changesetAuthor,
  changesetPathsLabel,
  commitChangeset,
  commitDate,
  commitStamp,
  commitTask,
  commitTrailer,
  diffStats,
  docActionError,
  filterChangesets,
  hasConflictMarkers,
  isAgentMergedChangeset,
  isAgentMergedCommit,
  isUnreviewedChangeset,
  normalizeChangesetDetail,
  normalizeMergeView,
  pageMarkers,
  parseUnifiedDiff,
  type ChangesFilter,
} from "../web/src/entities/doc/changes.ts";
import type { Changeset, CommitInfo } from "../web/src/entities/doc/model.ts";

function changeset(patch: Partial<Changeset> & { id: number }): Changeset {
  return {
    branch: `changes/${patch.id}`,
    project: "demo",
    task: null,
    author: "gshulga",
    authorName: "Григорий",
    authorKind: "human",
    title: `Изменение ${patch.id}`,
    status: "open",
    base: "aaaaaaaa",
    paths: ["demo/guides/run.md"],
    origin: "genie",
    reverts: null,
    created: "2026-10-30T09:00:00Z",
    updated: "2026-10-30T09:30:00Z",
    publishedAt: null,
    publishedCommit: null,
    closedBy: null,
    remindedAt: null,
    reviewNeeded: false,
    reviewedBy: null,
    reviewedAt: null,
    reviewDecision: null,
    mergedByAgent: false,
    ...patch,
  };
}

function commit(patch: Partial<CommitInfo> = {}): CommitInfo {
  return {
    sha: "0123456789abcdef",
    parents: ["1111111"],
    authorName: "Григорий",
    authorEmail: "g@example.com",
    date: "2026-10-30T09:30:00Z",
    subject: "docs: update the run guide",
    body: "",
    trailers: [],
    ...patch,
  };
}

test("a unified diff names added, removed, context, hunk and metadata lines", () => {
  const lines = parseUnifiedDiff(
    ["diff --git a/run.md b/run.md", "--- a/run.md", "+++ b/run.md", "@@ -1,3 +1,3 @@", " # Заголовок", "-Старый шаг", "+Новый шаг", " Хвост", "\\ No newline at end of file"].join("\n"),
  );
  assert.deepEqual(
    lines.map((line) => line.kind),
    ["meta", "meta", "meta", "hunk", "context", "removed", "added", "context", "meta"],
  );
  assert.equal(lines[5].text, "-Старый шаг");
  assert.deepEqual(diffStats(lines), { added: 1, removed: 1 });
  assert.deepEqual(parseUnifiedDiff(""), []);
  assert.deepEqual(parseUnifiedDiff("без нового перевода\n"), [{ kind: "context", text: "без нового перевода" }]);
});

test("conflict markers are rejected, ordinary equals signs are not", () => {
  assert.equal(hasConflictMarkers("<<<<<<< ours\nтекст\n=======\nчужой\n>>>>>>> theirs"), true);
  assert.equal(hasConflictMarkers("Заголовок\n=======\nподчёркивание"), false);
  assert.equal(hasConflictMarkers("<<<<<<< not at a line start"), true);
  assert.equal(hasConflictMarkers("a <<<<<<< b"), false);
});

test("the feed filter round-trips through the address and leaves the default implicit", () => {
  assert.deepEqual(changesFilterFromParams(new URLSearchParams("")), { view: "open", task: undefined, author: undefined, path: undefined });
  const filter: ChangesFilter = { view: "unreviewed", task: "GNE-1", author: "ivan", path: "demo/run.md" };
  const params = changesFilterToParams(filter);
  assert.deepEqual(changesFilterFromParams(params), filter);
  assert.equal(changesFilterToParams({ view: "open" }).toString(), "");
  assert.deepEqual(changesFilterFromParams(new URLSearchParams("view=unknown")).view, "open");
  assert.equal(changesServerQuery({ view: "open" }), "status=open");
  assert.equal(changesServerQuery({ view: "unreviewed" }), "unreviewed=true");
  assert.equal(changesServerQuery({ view: "agent" }), "");
  assert.equal(changesServerQuery({ view: "all", task: "GNE-1" }), "task=GNE-1");
});

test("each feed view keeps exactly its own changesets", () => {
  const list = [
    changeset({ id: 1, status: "open" }),
    changeset({ id: 2, status: "published", reviewNeeded: true }),
    changeset({ id: 3, status: "published", mergedByAgent: true }),
    changeset({ id: 4, status: "published", reviewNeeded: true, reviewDecision: "reviewed" }),
    changeset({ id: 5, status: "discarded" }),
    changeset({ id: 6, status: "merging", task: "GNE-9", author: "ivan" }),
  ];
  const ids = (filter: ChangesFilter) => filterChangesets(list, filter).map((item) => item.id);
  assert.deepEqual(ids({ view: "open" }), [1]);
  assert.deepEqual(ids({ view: "unreviewed" }), [2]);
  assert.deepEqual(ids({ view: "agent" }), [3]);
  assert.deepEqual(ids({ view: "all" }), [1, 2, 3, 4, 5, 6]);
  assert.deepEqual(ids({ view: "all", task: "GNE-9" }), [6]);
  assert.deepEqual(ids({ view: "all", author: "ivan" }), [6]);
  assert.deepEqual(ids({ view: "all", path: "demo/other.md" }), []);
  assert.equal(isUnreviewedChangeset(list[1]), true);
  assert.equal(isUnreviewedChangeset(list[3]), false, "a decided review is not unreviewed");
  assert.equal(isAgentMergedChangeset(list[2]), true);
  assert.equal(isAgentMergedChangeset(list[5]), false, "a merging changeset is not published yet");
  assert.deepEqual(attentionChangesets(list).map((item) => item.id), [1, 2]);
  assert.deepEqual(changesFilterOptions(list), { tasks: ["GNE-9"], authors: ["gshulga", "ivan"] });
});

test("page marks name another author's draft, the review queue and a pending merge", () => {
  const marks = pageMarkers({
    openChangesets: [{ id: 7, author: "ivan", task: "GNE-3" }, { id: 8, author: "agent" }],
    unreviewed: [changeset({ id: 9, status: "published", reviewNeeded: true })],
    merges: [
      { id: 4, changeset: 11, status: "pending" },
      { id: 5, changeset: 12, status: "resolved" },
      { id: 6, changeset: 13, status: "running" },
    ],
  });
  assert.deepEqual(marks.map((mark) => mark.label), ["Открыт черновик: ivan, GNE-3", "Открыт черновик: agent", "Не проверено", "Сводится", "Сводится"]);
  assert.deepEqual(marks.map((mark) => mark.changeset), [7, 8, 9, 11, 13]);
  assert.equal(marks[2].tone, "accent");
  assert.equal(marks[3].merge, 4);
  assert.deepEqual(pageMarkers({}), []);
});

test("publishing and reviewing follow the author and the project role", () => {
  assert.equal(canPublishChangeset({ author: "ivan", status: "open" }, "ivan"), true);
  assert.equal(canPublishChangeset({ author: "ivan", status: "open" }, "chen"), false);
  assert.equal(canPublishChangeset({ author: "ivan", status: "published" }, "ivan"), false);
  assert.equal(canPublishChangeset({ author: "ivan", status: "open" }, undefined), false);
  assert.equal(canReviewChangeset("owner"), true);
  assert.equal(canReviewChangeset("admin"), true);
  assert.equal(canReviewChangeset("member"), false);
  assert.equal(canReviewChangeset(undefined), false);
});

test("commit trailers name the task, the changeset and the merge agent", () => {
  const merge = commit({ trailers: [["Genie-Task", "GNE-110"], ["Genie-Changeset", "12"], ["Genie-Merged-By", "doc-merger"]] });
  assert.equal(commitTask(merge), "GNE-110");
  assert.equal(commitChangeset(merge), 12);
  assert.equal(commitTrailer(merge, "genie-merged-by"), "doc-merger");
  assert.equal(isAgentMergedCommit(merge), true);
  assert.equal(isAgentMergedCommit(commit()), false);
  assert.equal(commitChangeset(commit({ trailers: [["Genie-Changeset", "not a number"]] })), undefined);
  assert.equal(commitDate("2026-10-30T09:30:00Z"), "30.10.2026");
  assert.equal(commitStamp("2026-10-30T09:30:00Z"), "30.10.2026, 09:30");
  assert.equal(commitDate("не дата"), "не дата");
});

test("changeset wording stays human", () => {
  assert.equal(changesetPathsLabel(1), "1 страница");
  assert.equal(changesetPathsLabel(3), "3 страницы");
  assert.equal(changesetPathsLabel(11), "11 страниц");
  assert.equal(changesetAuthor({ author: "ivan", authorName: "Иван", authorKind: "human" }), "Иван (ivan)");
  assert.equal(changesetAuthor({ author: "agent", authorName: "agent", authorKind: "agent" }), "agent, агент");
});

test("server failures say what a person can do next", () => {
  assert.match(docActionError({ status: 409, message: "wrong state" }), /Обновите страницу/);
  assert.match(docActionError({ status: 403, message: "locked" }), /Недостаточно прав/);
  assert.match(docActionError({ status: 404, message: "not found" }), /не найдены/);
  assert.equal(docActionError({ message: "Парсер отказал" }), "Парсер отказал");
  assert.equal(docActionError({}), "Не удалось выполнить действие");
});

test("detail envelopes are accepted nested or flat", () => {
  const one = changeset({ id: 12, status: "published" });
  const nested = normalizeChangesetDetail({ changeset: one, commits: [commit()], diff: "+a" });
  assert.equal(nested.changeset.id, 12);
  assert.equal(nested.commits.length, 1);
  assert.equal(nested.diff, "+a");
  const flat = normalizeChangesetDetail({ ...one, commits: [commit()], diff: "-b" });
  assert.equal(flat.changeset.id, 12);
  assert.equal(flat.diff, "-b");
  const merge = normalizeMergeView({
    id: 4,
    changeset: 12,
    status: "pending",
    main: "aaa",
    tip: "bbb",
    conflicts: ["demo/run.md"],
    attempts: 1,
    error: null,
    resolvedBy: null,
    resolution: null,
    contradiction: false,
    job: null,
    note: "",
    created: "2026-10-30T09:00:00Z",
    updated: "2026-10-30T09:30:00Z",
  });
  assert.equal(merge.merge.id, 4);
  assert.deepEqual(merge.pages.map((page) => page.path), ["demo/run.md"]);
});
