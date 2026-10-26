import { useState } from "react";
import { Link } from "react-router";
import { capitalized, MAIL_TITLE, ON_TITLE, RELATION_TITLE, STAGE_TITLE, TemplateGraph, WORKSPACE_TITLE, usePreview, type Catalogue, type MemberDef, type Relation, type TeamDef } from "@/entities/agent-config";
import { agentTone, displayName } from "@/entities/member";
import { Button, ErrorState, Field, Icon, IconButton, Input, LoadingState, Markdown, useAct } from "@/shared/ui";

export function TemplateTeam({ template: t, cfg, base }: { template: TeamDef; cfg: Catalogue; base: string }) {
  const title = (key: string) => memberTitle(t.members.find((m) => m.key === key), cfg) || (key === "orchestrator" ? "Менеджер задачи" : key);
  return <div className="template-team">
    <TemplateGraph compact members={t.members} relations={t.relations} roles={cfg.roles} problems={t.warnings} />
    {t.charter && <section className="template-section"><h3>Общие правила</h3><Markdown text={t.charter} /></section>}
    <section className="template-section"><h3>Участники <span className="g-muted">{t.members.length}</span></h3>
      <div className="template-members">{t.members.map((member) => <details key={member.key} className="template-member">
        <summary><MemberAvatar member={member} cfg={cfg} /><span>{memberTitle(member, cfg)}</span>{member.key !== member.role && <code>{member.key}</code>}</summary>
        <div className="template-member-body"><dl className="template-values">
          <dt>Роль</dt><dd><Link to={`${base}?tab=roles&id=${member.role}`}>{cfg.roles.find((r) => r.id === member.role)?.title ?? member.role}</Link></dd>
          {member.model && <><dt>Модель</dt><dd className="g-mono">{member.model}</dd></>}
          {member.thinking && <><dt>Размышление</dt><dd>{member.thinking}</dd></>}
        </dl>{member.instructions && <Markdown text={member.instructions} />}</div>
      </details>)}</div>
    </section>
    <details className="template-flow-details"><summary>Связи <span className="g-muted">{t.relations.length}</span></summary><RelationList relations={t.relations} title={title} /></details>
  </div>;
}
export function TemplateSettingsView({ template: t }: { template: TeamDef }) {
  return <dl className="template-values"><dt>Стадия</dt><dd>{STAGE_TITLE[t.stage]}</dd><dt>Рабочее место</dt><dd>{WORKSPACE_TITLE[t.workspace]}</dd><dt>Почта</dt><dd>{MAIL_TITLE[t.mail]}</dd><dt>Проекты</dt><dd>{t.projects?.join(", ") || "Все проекты"}</dd></dl>;
}
export function MemberAvatar({ member, cfg }: { member: Pick<MemberDef, "role">; cfg: Catalogue }) {
  return <span className="ag-class" data-tone={agentTone(member.role, cfg.roles)} aria-hidden="true">{(cfg.roles.find((r) => r.id === member.role)?.title ?? member.role).slice(0, 1).toUpperCase()}</span>;
}
export function memberTitle(member: MemberDef | undefined, cfg: Catalogue): string {
  return member ? member.name ? capitalized(member.name) : cfg.roles.find((r) => r.id === member.role)?.title ?? member.role : "";
}
function RelationList({ relations, title }: { relations: Relation[]; title: (key: string) => string }) {
  return relations.length ? <ul className="template-relations">{relations.map((r, i) => <li key={i}><div><strong>{title(r.from)}</strong><span className="template-relation-kind">{RELATION_TITLE[r.type]}</span><strong>{r.to.map(title).join(", ")}</strong></div>{r.on && <span>{ON_TITLE[r.on] ?? r.on}</span>}{r.note && <p>{r.note}</p>}</li>)}</ul> : <p className="g-muted">Связей нет.</p>;
}

export function TemplatePrompts({ id }: { id: string }) {
  const [task, setTask] = useState("");
  const [asked, setAsked] = useState<string>();
  const preview = usePreview(id, asked);
  const act = useAct();
  return <section className="template-prompts" aria-label="Промпты участников">
    <form className="template-preview-form" onSubmit={(e) => { e.preventDefault(); const next = task.trim() || undefined; if (next === asked) void preview.refetch(); else setAsked(next); }}>
      <Field label="Задача"><Input value={task} onChange={(e) => setTask(e.target.value)} placeholder="Пример задачи" /></Field>
      <Button type="submit" busy={preview.isFetching}><Icon.eye />Показать</Button>
    </form>
    {preview.isPending ? <LoadingState label="Загрузка промптов" /> : preview.error ? <ErrorState message={preview.error.message} onRetry={() => void preview.refetch()} /> : <>
      {preview.data.warnings.length > 0 && <ul className="ag-warn">{preview.data.warnings.map((w) => <li key={w}>{w}</li>)}</ul>}
      {preview.data.members.map((member) => <details className="template-kickoff" key={member.key}><summary>{displayName(member.name)}{member.key !== member.role && <code>{member.key}</code>}</summary><div className="template-kickoff-body"><div className="config-icon-group"><IconButton label={`Скопировать промпт ${member.key}`} onClick={() => void act(() => navigator.clipboard.writeText(member.kickoff), "Промпт скопирован")}><Icon.copy /></IconButton></div><pre>{member.kickoff}</pre></div></details>)}
    </>}
  </section>;
}
