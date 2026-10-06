/** A journal event as the live stream sends it (`crates/genie-core/src/events.rs`); only what the mapping reads. */
export interface JournalEvent {
  type: string;
  subject?: string;
  payload?: unknown;
}

/** A query-key prefix: it matches every query whose key starts with it. */
export type KeyPrefix = readonly string[];

const TASK_LISTS: KeyPrefix[] = [["tasks"]];
/** Spend grows as agents work; an epic's includes its tasks', so the whole branch. */
const USAGE: KeyPrefix = ["usage"];
const USAGE_OF_TASKS: KeyPrefix = ["usage", "task"];

/** A task's own page: what its comments, criteria, artifacts and block flag change. */
const taskPage = (e: JournalEvent): KeyPrefix[] => [e.subject ? ["task", e.subject] : ["task"], ...TASK_LISTS, USAGE_OF_TASKS];

/**
 * A task's creation, deletion, move or edit also reaches the pages of other tasks
 * (an epic's children, a dependant's blockers), so every open task page, the
 * counters in `meta` and the teams' task line (`taskInfo`).
 */
const taskRelations = (): KeyPrefix[] => [["task"], ...TASK_LISTS, ["meta"], ["teams"], ["team"], USAGE_OF_TASKS];

/**
 * Teams and their agents: rosters, mail counters and live sessions. A mutation
 * that only changes those (a member added, removed, paused or restarted writes a
 * team log line, not a journal event) passes this itself: no event refetches it.
 */
export const teamKeys = (): KeyPrefix[] => [["teams"], ["team"], ["peek"]];

/**
 * The same plus where a team shows elsewhere: the task cards and pages. A team
 * that appears or goes away (`team.spawned`, stopped, deleted) touches these.
 */
export const teamState = (): KeyPrefix[] => [...teamKeys(), ...TASK_LISTS, ["task"]];

/** Delivery of a task's branch; `task-repos` is also polled, the host's checks have no event while they run. */
const delivery = (e: JournalEvent): KeyPrefix[] => [["task-repos"], e.subject ? ["task", e.subject] : ["task"]];

const DOCS: KeyPrefix[] = [["docs"], ["proposals"], ["proposal"], ["task"] /* the docs-impact hint of a task */];

/**
 * The queries one journal event can have made stale, by event type. `null`: the
 * type is unknown (a new one on the server), so anything may be stale.
 * Prefixes mirror the keys the entities' api modules use; a test pins the roots.
 */
export function keysFor(e: JournalEvent): KeyPrefix[] | null {
  switch (e.type) {
    case "task.created":
    case "task.deleted":
    case "task.updated":
    case "task.status_changed":
    case "task.team_assigned":
      return taskRelations();
    case "task.commented":
    case "task.criterion_checked":
    case "task.artifact_added":
    case "task.blocked":
    case "task.unblocked":
      return taskPage(e);
    case "mail.sent":
      return teamKeys();
    case "team.spawned":
    case "team.started":
    case "team.stopped":
      return teamState();
    case "mcp.called":
      return [["mcp-calls"], USAGE];
    case "git.pushed":
    case "git.denied":
      return [["task-repos"]];
    case "cr.opened":
    case "cr.merged":
    case "cr.closed":
    case "ci.failed":
    case "ci.passed":
    case "ci.stalled":
      return delivery(e);
    case "doc.changed":
    case "doc.proposal":
    case "doc.proposal_decided":
    case "release.published": // the changelog is a page
      return DOCS;
    default:
      return null;
  }
}

/** The prefixes for a batch of events, or `null` when everything must be refetched. */
export function keysForAll(events: readonly JournalEvent[]): KeyPrefix[] | null {
  const seen = new Set<string>();
  const out: KeyPrefix[] = [];
  for (const e of events) {
    const keys = keysFor(e);
    if (!keys) return null;
    for (const k of keys) {
      const id = JSON.stringify(k);
      if (seen.has(id)) continue;
      seen.add(id);
      out.push(k);
    }
  }
  return out;
}

/** Does a query key start with one of the prefixes? */
export function matchesAny(prefixes: readonly KeyPrefix[], key: readonly unknown[]): boolean {
  return prefixes.some((p) => p.length <= key.length && p.every((part, i) => key[i] === part));
}
