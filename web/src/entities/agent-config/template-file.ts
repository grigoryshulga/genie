// The editable file, separate from the server's resolved catalogue view.
import type { Catalogue, MemberDef, Relation, TeamDef, TemplateDetail } from "./model.ts";

export type TemplateMember = Omit<MemberDef, "key"> & { key?: string };
export interface TemplateFile {
  title: string;
  description?: string;
  stage?: TeamDef["stage"];
  workspace?: TeamDef["workspace"];
  mail?: TeamDef["mail"];
  members: TemplateMember[];
  relations?: Relation[];
  charter?: string;
  projects?: string[];
}
export type TemplateDraft = TemplateFile & Record<string, unknown>;
export const memberKey = (member: TemplateMember) => member.key || member.role;

export function templateFile(t: TeamDef): TemplateFile {
  return {
    title: t.title, description: t.description, stage: t.stage, workspace: t.workspace, mail: t.mail,
    members: t.members.map((m) => ({ ...m })),
    ...(t.relationsDerived ? {} : { relations: t.relations.map((r) => ({ ...r, to: [...r.to] })) }),
    ...(t.charter ? { charter: t.charter } : {}), ...(t.projects ? { projects: [...t.projects] } : {}),
  };
}

/** Keep unknown file fields and legacy workspace semantics when editing existing files. */
export function templateDraft(detail: TemplateDetail): TemplateDraft {
  const raw = detail.file.path.startsWith("teams/") ? detail.file.content : detail.builtin;
  let parsed: Record<string, unknown> = {};
  try {
    const value: unknown = raw ? JSON.parse(raw) : undefined;
    if (value && typeof value === "object" && !Array.isArray(value)) parsed = value as Record<string, unknown>;
  } catch { /* The resolved last good version remains editable. */ }
  const draft: TemplateDraft = { ...parsed, ...templateFile(detail.template) };
  if (detail.template.relationsDerived) delete draft.relations;
  delete draft.worktree;
  return draft;
}

/** Keys identify graph endpoints; renaming a member must rename both sides atomically. */
export function updateTemplateMember(draft: TemplateDraft, index: number, patch: Partial<TemplateMember>): TemplateDraft {
  const before = memberKey(draft.members[index]);
  const member = { ...draft.members[index], ...patch };
  const after = memberKey(member);
  return { ...draft, members: draft.members.map((m, i) => i === index ? member : m), ...(draft.relations ? { relations: draft.relations.map((r) => ({ ...r, from: r.from === before ? after : r.from, to: r.to.map((key) => key === before ? after : key) })) } : {}) };
}

export function removeTemplateMember(draft: TemplateDraft, index: number): TemplateDraft {
  const key = memberKey(draft.members[index]);
  return { ...draft, members: draft.members.filter((_, i) => i !== index), ...(draft.relations ? { relations: draft.relations.filter((r) => r.from !== key).map((r) => ({ ...r, to: r.to.filter((to) => to !== key) })).filter((r) => r.to.length) } : {}) };
}

export function nextMemberKey(role: string, members: TemplateMember[]): string {
  const taken = new Set(members.map(memberKey));
  let key = role, i = 2;
  while (taken.has(key)) key = `${role}-${i++}`;
  return key;
}

/** Preview the server's default_relations process for an unsaved automatic template. */
export function templateRelations(draft: TemplateFile, roles: Catalogue["roles"]): Relation[] {
  if (draft.relations !== undefined) return draft.relations;
  const all = (kind: string) => draft.members.filter((m) => roles.find((r) => r.id === m.role)?.class === kind).map(memberKey);
  const [analyst, executor, reviewer, tester, documenter] = ["analyst", "executor", "reviewer", "tester", "documenter"].map((kind) => all(kind)[0]);
  const out: Relation[] = [];
  const add = (from: string, to: string[], type: Relation["type"], note: string, on?: string) => out.push({ from, to, type, note, ...(on ? { on } : {}) });
  if (draft.stage === "refinement" || !executor) {
    if (analyst && reviewer) { add(analyst, [reviewer], "handoff", "the findings"); add(reviewer, [analyst], "returns", "gaps and risks in the findings"); }
    const voice = analyst || reviewer;
    if (voice) add(voice, ["orchestrator"], "reports", "the findings");
  } else {
    if (analyst) { add(analyst, [executor], "handoff", "the plan is ready"); add(executor, [analyst], "consults", "questions about the plan"); }
    const checkers = [...all("tester"), ...all("reviewer")];
    if (checkers.length) add(executor, checkers, "handoff", "the work is submitted for review", "review");
    for (const key of all("tester")) { if (reviewer) add(key, [reviewer], "handoff", "the test report"); add(key, [executor], "returns", "failing tests"); }
    if (reviewer) { add(reviewer, [executor], "returns", "review findings", "changes_requested"); if (documenter) add(reviewer, [documenter], "handoff", "the work is approved", "approved"); }
    add(reviewer || tester || executor, ["orchestrator"], "reports", "the verdict");
  }
  if (documenter) add(documenter, ["orchestrator"], "reports", "the documentation is ready");
  return out;
}
