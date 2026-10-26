// Team templates: the list, a template's page (members, relations as a table and
// a graph, what each member gets) and the structured editor over the template's
// JSON file (`<data>/teams/<id>.json`).

import { Fragment, useState } from "react";
import { Link } from "react-router";
import {
  sectionProblems,
  type Catalogue,
  MAIL_TITLE,
  type MemberDef,
  ON_STATUSES,
  ON_TITLE,
  ORIGIN_TITLE,
  type Relation,
  RELATION_TITLE,
  type RelKind,
  STAGE_TITLE,
  type TeamDef,
  TemplateGraph,
  type TemplateDetail,
  useDeleteConfig,
  usePreview,
  useSaveConfig,
  useTemplate,
  WORKSPACE_TITLE,
} from "@/entities/agent-config";
import { displayName } from "@/entities/member";
import { plural } from "@/shared/lib";
import { Badge, ConfirmDialog, Dialog, Icon, LoadingState, Section, useAct, useToast } from "@/shared/ui";
import { FileEditor } from "./FileEditor.tsx";
import { History } from "./History.tsx";
import { Problems } from "./Problems.tsx";

export function TemplatesTab({
  cfg,
  selected,
  onSelect,
  readOnly = false,
  base = "/agents",
}: {
  cfg: Catalogue;
  selected?: string;
  onSelect: (id: string) => void;
  /** The catalogue (/agents) forces read-only even for a server administrator. */
  readOnly?: boolean;
  /** Where this tab lives: cross-links to the other tabs stay inside the same mode. */
  base?: string;
}) {
  const current = selected ?? cfg.teams[0]?.id;
  return (
    <div className="split ag-templates-layout">
      <section className="split-list ag-list" aria-label="Шаблоны команд">
        <div className="side-head"><span className="side-title">Шаблоны команд</span></div>
        <Problems items={sectionProblems(cfg.problems, "templates").filter((p) => p.item !== `team:${current}`)} />
        {cfg.teams.map((t) => (
          <button type="button" key={t.id} className={`pick${t.id === current ? " on" : ""}`} onClick={() => onSelect(t.id)} aria-current={t.id === current}>
            <span className="t">
              <span className="ag-avatars" aria-hidden="true">
                {t.members.slice(0, 5).map((m) => {
                  const role = cfg.roles.find((r) => r.id === m.role);
                  return (
                    <span key={m.key} className={`ag-class c-${role?.class ?? "executor"}`}>
                      {(role?.title ?? m.role).slice(0, 1).toUpperCase()}
                    </span>
                  );
                })}
              </span>
              <span>{t.title}</span>
              {t.origin !== "builtin" && <Badge tone={t.origin === "custom" ? "accent" : "amber"}>{ORIGIN_TITLE[t.origin]}</Badge>}
            </span>
            <span className="s">
              {[STAGE_TITLE[t.stage], WORKSPACE_TITLE[t.workspace], `${t.members.length} ${plural(t.members.length, "участник", "участника", "участников")}`, t.projects ? t.projects.join(", ") : ""]
                .filter(Boolean)
                .join(" · ")}
            </span>
          </button>
        ))}
      </section>
      <section className="split-main" aria-label="Шаблон">
        {current ? <TemplatePane key={current} id={current} cfg={cfg} readOnly={readOnly} base={base} /> : <div className="ag-pane-empty">Шаблонов нет</div>}
      </section>
    </div>
  );
}

function TemplatePane({ id, cfg, readOnly = false, base = "/agents" }: { id: string; cfg: Catalogue; readOnly?: boolean; base?: string }) {
  const detail = useTemplate(id);
  const [mode, setMode] = useState<"form" | "file">();
  const [confirmDelete, setConfirmDelete] = useState(false);
  const remove = useDeleteConfig();
  const act = useAct();
  const d = detail.data;
  if (!d) {
    if (detail.error) return <div className="ag-pane-empty">{detail.error.message}</div>;
    return <div className="ag-pane-empty"><LoadingState label="Загрузка шаблона" lines={3} width={260} /></div>;
  }
  const t = d.template;
  const admin = d.admin && !readOnly;
  const hasFile = d.file.content !== null && d.file.path.startsWith("teams/");
  const roleTitle = (id: string) => cfg.roles.find((r) => r.id === id)?.title ?? id;
  const title = (key: string) => {
    if (key === "orchestrator") return "Оркестратор";
    const m = t.members.find((x) => x.key === key);
    return m ? (m.name ? `${m.name[0].toUpperCase()}${m.name.slice(1)}` : roleTitle(m.role)) : key;
  };
  const used = d.usedBy.automations.map((a) => (
    <Link key={a.id} to={`/manage/project/automations?rule=${a.id}`}>
      автоматизация «{a.name}»{a.project ? ` (${a.project})` : ""}
    </Link>
  ));
  return (
    <>
      <div className="pane-head">
        <div className="ttl">
          <h2>
            {t.title}
            {!admin && <span className="ag-readonly">Только чтение</span>}
          </h2>
          <span className="sub">
            <span className="g-mono">{t.id}</span> · {ORIGIN_TITLE[t.origin]} · стадия «{STAGE_TITLE[t.stage]}» · {WORKSPACE_TITLE[t.workspace]} · почта {t.mail === "flow" ? "по связям шаблона" : "свободная"}
            {t.path ? ` · ${t.path}` : ""}
          </span>
          {t.description && <p className="desc">{t.description}</p>}
        </div>
        {admin && (
          <>
            <button type="button" className="g-btn secondary" onClick={() => setMode("form")}>
              Изменить
            </button>
            <button type="button" className="g-btn secondary" onClick={() => setMode("file")}>
              <Icon.file size={13} />
              Файл
            </button>
            {hasFile && (
              <button type="button" className="g-btn ghost" onClick={() => setConfirmDelete(true)}>
                {d.builtin !== null ? "Вернуть встроенный" : "Удалить"}
              </button>
            )}
          </>
        )}
      </div>
      <div className="pane-body">
        <Problems items={d.problems} />
        <TemplateGraph members={t.members} relations={t.relations} roles={cfg.roles} problems={[...t.warnings, ...d.problems.map((p) => p.message)]} />
        {(t.relationsDerived || t.warnings.length > 0) && (
          <div className="ag-graph-notes">
            {t.relationsDerived && <p className="g-muted ag-note">Связей в шаблоне нет: они выведены из классов участников.</p>}
            {t.warnings.map((w) => (
              <p key={w} className="ag-warn-line">
                {w}
              </p>
            ))}
          </div>
        )}
        <div className="ag-two">
          <Section title="Участники">
            <ul className="ag-members">
              {t.members.map((m) => {
                const role = cfg.roles.find((r) => r.id === m.role);
                return (
                  <li key={m.key} title={m.instructions ? `Особенности: ${m.instructions}` : undefined}>
                    <span className={`ag-class c-${role?.class ?? "executor"}`} aria-hidden="true">
                      {(role?.title ?? m.role).slice(0, 1).toUpperCase()}
                    </span>
                    <Link to={`${base}?tab=roles&id=${m.role}`}>{m.name ? `${title(m.key)} — ${roleTitle(m.role).toLowerCase()}` : roleTitle(m.role)}</Link>
                    {m.key !== m.role && <span className="g-mono g-muted">{m.key}</span>}
                    {m.instructions && <span className="ag-tag plus">особенности</span>}
                    <span className="grow" />
                    {m.model && <span className="g-muted">{m.model.replace(/^[^/]+\//, "")}</span>}
                  </li>
                );
              })}
            </ul>
          </Section>
          <Section title="Связи">
            <ul className="ag-relations">
              {t.relations.flatMap((r, i) =>
                r.to.map((to) => (
                  <li key={`${i}-${to}`}>
                    <span className="who">
                      {title(r.from)} → {title(to)}
                    </span>
                    <span className={`ag-rel ${r.type}`}>
                      {RELATION_TITLE[r.type]}
                      {r.on ? ` · ${ON_TITLE[r.on] ?? r.on}` : ""}
                    </span>
                    <span className="g-muted note">{r.note}</span>
                  </li>
                )),
              )}
              {!t.relations.length && <li className="g-muted">Связей нет.</li>}
            </ul>
          </Section>
        </div>
        {t.charter && (
          <Section title="Общие правила команды">
            <p className="ag-charter">{t.charter}</p>
          </Section>
        )}
        <KickoffPreview template={t.id} roles={cfg.roles} />
        <p className="ag-used">
          {used.length ? (
            <>
              Используется:{" "}
              {used.map((x, i) => (
                <Fragment key={i}>
                  {i > 0 && ", "}
                  {x}
                </Fragment>
              ))}
              . Оркестратор и люди собирают по нему команды.
            </>
          ) : (
            "Автоматизации его не используют; оркестратор и люди собирают по нему команды."
          )}
        </p>
        {admin && (
          <Section title="История изменений">
            <History item={`team:${t.id}`} admin={admin} />
          </Section>
        )}
      </div>
      {mode === "file" && (
        <FileEditor
          kind="template"
          id={t.id}
          title={`teams/${t.id}.json`}
          hint={hasFile ? undefined : "Файла ещё нет: сохранение создаст шаблон в каталоге данных вместо встроенного."}
          initial={hasFile ? (d.file.content ?? "") : (d.builtin ?? JSON.stringify(templateJson(t), null, 2))}
          baseHash={hasFile ? d.file.hash : ""}
          problems={d.problems}
          onClose={() => setMode(undefined)}
        />
      )}
      {mode === "form" && <TemplateForm detail={d} cfg={cfg} onClose={() => setMode(undefined)} />}
      {confirmDelete && (
        <ConfirmDialog
          title={d.builtin !== null ? `Вернуть встроенный шаблон ${t.id}?` : `Удалить шаблон ${t.id}?`}
          confirmLabel={d.builtin !== null ? "Вернуть" : "Удалить"}
          danger={d.builtin === null}
          onClose={() => setConfirmDelete(false)}
          onConfirm={() => {
            setConfirmDelete(false);
            void act(() => remove("template", t.id, d.file.hash), d.builtin !== null ? "Шаблон снова встроенный" : "Шаблон удалён");
          }}
        >
          Идущие команды сохраняют снимок шаблона и работают дальше.
        </ConfirmDialog>
      )}
    </>
  );
}

/** A template as its file would hold it (for a built-in one without a file of its own). */
function templateJson(t: TeamDef): Record<string, unknown> {
  return {
    title: t.title,
    description: t.description,
    stage: t.stage,
    workspace: t.workspace,
    ...(t.mail !== "open" ? { mail: t.mail } : {}),
    members: t.members.map((m) => ({ ...m, key: m.key === m.role ? undefined : m.key })),
    ...(t.relationsDerived ? {} : { relations: t.relations }),
    ...(t.charter ? { charter: t.charter } : {}),
    ...(t.projects ? { projects: t.projects } : {}),
  };
}

function KickoffPreview({ template, roles }: { template: string; roles: Catalogue["roles"] }) {
  const [task, setTask] = useState("");
  const [asked, setAsked] = useState<string>();
  const preview = usePreview(template, asked);
  return (
    <Section title="Что получит каждый участник">
      <form
        className="ag-preview-form"
        onSubmit={(e) => {
          e.preventDefault();
          setAsked(task.trim() || undefined);
        }}
      >
        <input value={task} onChange={(e) => setTask(e.target.value)} placeholder="Задача, например SHOP-12 (иначе пример)" aria-label="Задача для предпросмотра" />
        <button type="submit" className="g-btn secondary">
          Показать
        </button>
      </form>
      {preview.error && <p className="ag-warn">{preview.error.message}</p>}
      {preview.data?.members.map((m) => (
        <details key={m.key} className="ag-kickoff">
          <summary>
            {displayName(m.name)} <span className="g-muted">· {(roles.find((r) => r.id === m.role)?.title ?? m.role).toLowerCase()}</span>
          </summary>
          <pre className="view">{m.kickoff}</pre>
        </details>
      ))}
    </Section>
  );
}

// ------------------------------------------------------------------ structured editor

type Draft = Record<string, unknown> & { members: MemberDef[]; relations?: Relation[] };
const KINDS: RelKind[] = ["handoff", "returns", "reports", "consults"];

function parseDraft(d: TemplateDetail): Draft {
  const text = d.file.content && d.file.path.startsWith("teams/") ? d.file.content : d.builtin;
  let v: Draft | undefined;
  try {
    v = text ? (JSON.parse(text) as Draft) : undefined;
  } catch {
    v = undefined;
  }
  const draft = v ?? (templateJson(d.template) as Draft);
  // The legacy `worktree: true/false` reads as a workspace.
  if (draft.workspace === undefined && typeof draft.worktree === "boolean") {
    draft.workspace = draft.worktree ? "worktree" : "repo";
    delete draft.worktree;
  }
  draft.members = (draft.members ?? []).map((m) => ({ ...m }));
  return draft;
}

const keyOf = (m: MemberDef) => m.key || m.role;

function TemplateForm({ detail, cfg, onClose }: { detail: TemplateDetail; cfg: Catalogue; onClose: () => void }) {
  const [draft, setDraft] = useState<Draft>(() => parseDraft(detail));
  const [error, setError] = useState<string>();
  const save = useSaveConfig();
  const toast = useToast();
  const hasFile = detail.file.content !== null && detail.file.path.startsWith("teams/");
  const roles = cfg.roles.filter((r) => r.class !== "orchestrator");
  const set = (k: string, v: unknown) => setDraft((d) => ({ ...d, [k]: v === "" || v === undefined ? undefined : v }));
  const members = draft.members;
  const relations = draft.relations ?? [];
  const keys = members.map(keyOf);
  const setMember = (i: number, v: Partial<MemberDef>) => set("members", members.map((m, j) => (j === i ? { ...m, ...v } : m)));
  const setRelation = (i: number, v: Partial<Relation>) => set("relations", relations.map((r, j) => (j === i ? { ...r, ...v } : r)));
  const clean = (d: Draft): Draft => {
    const out: Draft = { ...d, members: d.members.map((m) => Object.fromEntries(Object.entries(m).filter(([, x]) => x !== undefined && x !== "")) as unknown as MemberDef) };
    if (d.relations) {
      out.relations = d.relations.map((r) => Object.fromEntries(Object.entries(r).filter(([, x]) => x !== undefined && x !== "")) as unknown as Relation);
    }
    for (const k of Object.keys(out)) if (out[k] === undefined) delete out[k];
    return out;
  };
  const submit = async () => {
    setError(undefined);
    try {
      const out = await save("template", detail.template.id, { template: clean(draft) }, hasFile ? detail.file.hash : "");
      const warn = out.problems.filter((p) => p.level === "warning");
      toast(warn.length ? `Шаблон сохранён. Предупреждения: ${warn.map((p) => p.message).join("; ")}` : "Шаблон сохранён");
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  return (
    <Dialog
      title={
        <>
          Шаблон команды <span className="g-mono">{detail.template.id}</span>
        </>
      }
      onClose={onClose}
      wide
      footer={
        <>
          <span className="ag-save-note">Идущие команды сохраняют снимок шаблона</span>
          <button type="button" className="g-btn ghost" onClick={onClose}>
            Отмена
          </button>
          <button type="button" className="g-btn primary" onClick={() => void submit()}>
            Сохранить
          </button>
        </>
      }
    >
      <div className="ag-template-form">
        <div className="ag-row">
          <label className="g-field">
            Название
            <input className="g-input" value={String(draft.title ?? "")} onChange={(e) => set("title", e.target.value)} />
          </label>
          <label className="g-field">
            Стадия
            <select className="g-select" value={String(draft.stage ?? "delivery")} onChange={(e) => set("stage", e.target.value)}>
              <option value="delivery">{STAGE_TITLE.delivery} — задача в ready и дальше</option>
              <option value="refinement">{STAGE_TITLE.refinement} — до ready</option>
            </select>
          </label>
          <label className="g-field">
            Рабочее место
            <select className="g-select" value={String(draft.workspace ?? "worktree")} onChange={(e) => set("workspace", e.target.value)}>
              {(["worktree", "repo", "scratch"] as const).map((w) => (
                <option key={w} value={w}>
                  {WORKSPACE_TITLE[w]}
                </option>
              ))}
            </select>
          </label>
        </div>
        <label className="g-field">
          Описание — по нему оркестратор выбирает шаблон
          <textarea className="g-textarea" rows={2} value={String(draft.description ?? "")} onChange={(e) => set("description", e.target.value)} />
        </label>

        <h4>Участники</h4>
        {members.map((m, i) => (
          <div key={i} className="ag-member-row">
            <select value={m.role} onChange={(e) => setMember(i, { role: e.target.value })} aria-label="Роль">
              {roles.map((r) => (
                <option key={r.id} value={r.id}>
                  {r.title} ({r.id})
                </option>
              ))}
              {!roles.some((r) => r.id === m.role) && <option value={m.role}>{m.role}</option>}
            </select>
            <input value={m.key ?? ""} onChange={(e) => setMember(i, { key: e.target.value || undefined })} placeholder={`ключ: ${m.role}`} aria-label="Ключ" />
            <input value={m.model ?? ""} onChange={(e) => setMember(i, { model: e.target.value || undefined })} placeholder="модель роли" aria-label="Модель" />
            <input value={m.instructions ?? ""} onChange={(e) => setMember(i, { instructions: e.target.value || undefined })} placeholder="инструкции участнику" aria-label="Инструкции" />
            <button type="button" className="g-icon-btn" aria-label="Убрать участника" onClick={() => set("members", members.filter((_, j) => j !== i))}>
              <Icon.trash size={13} />
            </button>
          </div>
        ))}
        <button type="button" className="g-btn ghost sm ag-add" onClick={() => set("members", [...members, { key: "", role: roles[0]?.id ?? "executor" }])}>
          <Icon.plus size={13} />
          Участник
        </button>

        <h4>Связи</h4>
        {draft.relations === undefined ? (
          <p className="g-muted ag-note">
            Связи выводятся из классов участников.{" "}
            <button type="button" className="g-btn secondary" onClick={() => set("relations", detail.template.relations)}>
              Задать явно
            </button>
          </p>
        ) : (
          <>
            {relations.map((r, i) => (
              <div key={i} className="ag-relation-row">
                <select value={r.from} onChange={(e) => setRelation(i, { from: e.target.value })} aria-label="Кто">
                  {keys.map((k) => (
                    <option key={k} value={k}>
                      {k}
                    </option>
                  ))}
                  {!keys.includes(r.from) && <option value={r.from}>{r.from}</option>}
                </select>
                <select value={r.type} onChange={(e) => setRelation(i, { type: e.target.value as RelKind, ...(e.target.value === "reports" ? { to: ["orchestrator"], on: undefined } : {}) })} aria-label="Тип связи">
                  {KINDS.map((k) => (
                    <option key={k} value={k}>
                      {RELATION_TITLE[k]}
                    </option>
                  ))}
                </select>
                {r.type === "reports" ? (
                  <span className="g-mono g-muted">orchestrator</span>
                ) : (
                  <select multiple value={r.to} onChange={(e) => setRelation(i, { to: [...e.target.selectedOptions].map((o) => o.value) })} aria-label="Кому" size={Math.min(3, keys.length)}>
                    {keys
                      .filter((k) => k !== r.from)
                      .map((k) => (
                        <option key={k} value={k}>
                          {k}
                        </option>
                      ))}
                  </select>
                )}
                {r.type === "handoff" || r.type === "returns" ? (
                  <select value={r.on ?? ""} onChange={(e) => setRelation(i, { on: e.target.value || undefined })} aria-label="Когда">
                    <option value="">без статуса</option>
                    {ON_STATUSES.map((s) => (
                      <option key={s} value={s}>
                        когда «{ON_TITLE[s] ?? s}»
                      </option>
                    ))}
                  </select>
                ) : (
                  <span />
                )}
                <input value={r.note ?? ""} onChange={(e) => setRelation(i, { note: e.target.value || undefined })} placeholder="заметка" aria-label="Заметка" />
                <button type="button" className="g-icon-btn" aria-label="Убрать связь" onClick={() => set("relations", relations.filter((_, j) => j !== i))}>
                  <Icon.trash size={13} />
                </button>
              </div>
            ))}
            <button type="button" className="g-btn ghost sm ag-add" onClick={() => set("relations", [...relations, { from: keys[0] ?? "", to: keys.slice(1, 2), type: "handoff" }])}>
              <Icon.plus size={13} />
              Связь
            </button>
          </>
        )}
        <TemplateGraph
          members={members.map((m) => ({ ...m, key: keyOf(m) }))}
          relations={draft.relations ?? detail.template.relations}
          roles={cfg.roles}
          onConnect={(from, to) =>
            set("relations", [
              ...(draft.relations ?? detail.template.relations),
              to === "orchestrator" ? { from, to: ["orchestrator"], type: "reports" } : { from, to: [to], type: "handoff" },
            ])
          }
        />
        <p className="team-graph-hint">Перетащите связь между участниками; к оркестратору — для доклада.</p>
        <label className="g-field">
          Общие правила команды
          <textarea className="g-textarea" rows={2} value={String(draft.charter ?? "")} onChange={(e) => set("charter", e.target.value)} />
        </label>
        <div className="ag-row">
          <label className="g-field">
            Почта
            <select className="g-select" value={String(draft.mail ?? "open")} onChange={(e) => set("mail", e.target.value === "open" ? undefined : e.target.value)}>
              <option value="open">{MAIL_TITLE.open}</option>
              <option value="flow">{MAIL_TITLE.flow}</option>
            </select>
          </label>
          <label className="g-field">
            Проекты — пусто: все
            <input
              value={((draft.projects as string[] | undefined) ?? []).join(", ")}
              onChange={(e) => {
                const p = e.target.value
                  .split(",")
                  .map((x) => x.trim())
                  .filter(Boolean);
                set("projects", p.length ? p : undefined);
              }}
              placeholder="shop, erp"
            />
          </label>
        </div>
        {error && (
          <div className="ag-form-error" role="alert">
            {error}
          </div>
        )}
      </div>
    </Dialog>
  );
}
