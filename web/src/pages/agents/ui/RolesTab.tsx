// Roles: the list, a role's page and its editors. Every edit rewrites fields of
// the role's file (`<data>/agents/<id>.md`); for a built-in role without a file
// that creates an override holding only the changed fields.

import { Fragment, type ReactNode, useState } from "react";
import { Link } from "react-router";
import {
  allowDeny,
  basePermissions,
  type Catalogue,
  CLASS_TITLE,
  FILES_TITLE,
  FIXED_PERMISSIONS,
  ORIGIN_TITLE,
  PERMISSION_GROUPS,
  type RoleDef,
  type RoleDetail,
  setBody,
  setFrontmatterKey,
  splitFrontmatter,
  STAGE_TITLE,
  useDeleteConfig,
  useRole,
  useSaveConfig,
} from "@/entities/agent-config";
import { useMeta, useProjects } from "@/entities/project";
import { ModelPicker } from "@/features/agent-model";
import { ChipInput, ConfirmDialog, Icon, Markdown, Modal, useToast } from "@/shared/ui";
import { Badge, FileEditor, History, Problems, Section, useAction } from "./common.tsx";

export function RolesTab({ cfg, selected, onSelect }: { cfg: Catalogue; selected?: string; onSelect: (id: string) => void }) {
  const team = cfg.roles.filter((r) => r.class !== "orchestrator");
  const orch = cfg.roles.filter((r) => r.class === "orchestrator");
  const current = selected ?? team[0]?.id;
  const titleOf = (id: string) => cfg.roles.find((x) => x.id === id)?.title ?? id;
  const pick = (r: RoleDef) => (
    <button type="button" key={r.id} className={`pick${r.id === current ? " on" : ""}`} onClick={() => onSelect(r.id)} aria-current={r.id === current}>
      <span className="t">
        <span className={`ag-class c-${r.class}`} aria-hidden="true">
          {r.title.slice(0, 1).toUpperCase()}
        </span>
        <span>{r.title}</span>
        {r.origin !== "builtin" && <Badge tone={r.origin === "custom" ? "accent" : "amber"}>{ORIGIN_TITLE[r.origin]}</Badge>}
      </span>
      <span className="s">
        <span className="mono">{r.id}</span> · {r.extends ? `на основе «${titleOf(r.extends)}»` : CLASS_TITLE[r.class]}
        {r.projects ? ` · ${r.projects.join(", ")}` : ""}
      </span>
    </button>
  );
  return (
    <div className="split">
      <section className="split-list ag-list" aria-label="Роли">
        {team.map(pick)}
        {orch.length > 0 && <span className="label">Оркестратор</span>}
        {orch.map(pick)}
      </section>
      <section className="split-main" aria-label="Роль">
        {current ? <RolePane key={current} id={current} cfg={cfg} /> : <div className="pane-empty">Ролей нет</div>}
      </section>
    </div>
  );
}

type Dialog = "file" | "prompt" | "settings" | "skills" | "mcp" | "delete";

function RolePane({ id, cfg }: { id: string; cfg: Catalogue }) {
  const detail = useRole(id);
  const [dialog, setDialog] = useState<Dialog>();
  const remove = useDeleteConfig();
  const act = useAction();
  const d = detail.data;
  if (!d) return <div className="pane-empty">{detail.error ? detail.error.message : "Загрузка…"}</div>;
  const r = d.role;
  const admin = d.admin;
  const hasFile = d.file.content !== null;
  const close = () => setDialog(undefined);
  const parent = r.extends ? (cfg.roles.find((x) => x.id === r.extends)?.title ?? r.extends) : undefined;
  const used = [
    ...d.usedBy.templates.map((t) => (
      <Link key={`t-${t}`} to={`/agents?tab=templates&id=${t}`}>
        шаблон «{cfg.teams.find((x) => x.id === t)?.title ?? t}»
      </Link>
    )),
    ...d.usedBy.automations.map((a) => (
      <Link key={`a-${a.id}`} to={`/automations?rule=${a.id}`}>
        автоматизация «{a.name}»{a.project ? ` (${a.project})` : ""}
      </Link>
    )),
  ];
  return (
    <>
      <div className="pane-head">
        <div className="ttl">
          <h2>{r.title}</h2>
          <span className="sub">
            <span className="mono">{r.id}</span> · {ORIGIN_TITLE[r.origin]}
            {parent ? `, на основе «${parent}»` : ""} · класс {CLASS_TITLE[r.class]}
            {r.path ? ` · ${r.path}` : ""}
          </span>
          {r.description && <p className="desc">{r.description}</p>}
        </div>
        <button type="button" className="btn" onClick={() => setDialog("prompt")}>
          Промпт
        </button>
        {admin && (
          <button type="button" className="btn" onClick={() => setDialog("file")}>
            <Icon.file size={13} />
            Файл
          </button>
        )}
        {admin && hasFile && d.builtin !== null && (
          <button type="button" className="btn ghost" onClick={() => void act(() => remove("role", r.id, d.file.hash), "Роль снова встроенная")}>
            Вернуть встроенную
          </button>
        )}
        {admin && hasFile && d.builtin === null && (
          <button type="button" className="btn ghost" onClick={() => setDialog("delete")}>
            <Icon.trash size={13} />
            Удалить
          </button>
        )}
      </div>
      <div className="pane-body">
        <Problems items={d.problems} />
        {r.class === "orchestrator" ? (
          <Section title="Разрешения">
            <p className="muted ag-empty">Права оркестратора фиксированы: он разбирает задачи, собирает команды и принимает работу.</p>
          </Section>
        ) : (
          <Permissions detail={d} cfg={cfg} />
        )}
        <div className="ag-cards">
          <Workplace role={r} sandbox={cfg.sandbox} onEdit={admin ? () => setDialog("settings") : undefined} />
          <SkillsCard role={r} cfg={cfg} onEdit={admin ? () => setDialog("skills") : undefined} />
          <McpCard role={r} cfg={cfg} onEdit={admin && cfg.mcp.length > 0 ? () => setDialog("mcp") : undefined} />
        </div>
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
            </>
          ) : (
            "Пока не используется ни в шаблонах, ни в автоматизациях."
          )}
        </p>
        {admin && (
          <Section title="История изменений">
            <History item={`role:${r.id}`} admin={admin} />
          </Section>
        )}
      </div>
      {dialog === "file" && (
        <FileEditor
          kind="role"
          id={r.id}
          title={`agents/${r.id}.md`}
          hint={
            hasFile
              ? "Настройки во frontmatter, промпт — в теле файла."
              : "Файла ещё нет: сохранение создаст переопределение встроенной роли. Незаданные поля и пустое тело оставят встроенные значения."
          }
          initial={d.file.content ?? "---\n---\n"}
          baseHash={d.file.hash}
          problems={d.problems}
          onClose={close}
        />
      )}
      {dialog === "prompt" && (admin ? <PromptEditor detail={d} onClose={close} /> : <PromptView role={r} onClose={close} />)}
      {dialog === "settings" && <SettingsDialog detail={d} onClose={close} />}
      {dialog === "skills" && <SkillsDialog detail={d} cfg={cfg} onClose={close} />}
      {dialog === "mcp" && <McpDialog detail={d} cfg={cfg} onClose={close} />}
      {dialog === "delete" && (
        <ConfirmDialog
          title={`Удалить роль ${r.id}?`}
          confirmLabel="Удалить"
          danger
          onClose={close}
          onConfirm={() => {
            close();
            void act(() => remove("role", r.id, d.file.hash), "Роль удалена");
          }}
        >
          Сервер откажет, если роль используют шаблоны или автоматизации. Участники идущих команд с этой ролью остановятся с ошибкой.
        </ConfirmDialog>
      )}
    </>
  );
}

/** The role's prompt to read (people who do not edit the configuration). */
function PromptView({ role: r, onClose }: { role: RoleDef; onClose: () => void }) {
  return (
    <Modal label={`Промпт роли ${r.id}`} onClose={onClose} wide>
      <div className="mh">
        <Icon.file size={13} />
        <span className="ag-prompt-title">
          Промпт роли <span className="mono">{r.id}</span>
        </span>
        <button type="button" className="icon-btn" onClick={onClose} aria-label="Закрыть">
          <Icon.close />
        </button>
      </div>
      <div className="mb">
        {r.prompt ? <Markdown text={r.prompt} /> : <p className="muted">Промпта нет.</p>}
        {r.instructions && (
          <>
            <h4 className="ag-sub">Особенности роли</h4>
            <Markdown text={r.instructions} />
          </>
        )}
      </div>
    </Modal>
  );
}

/**
 * The role's prompt (the body of its file) with a live preview. An empty text
 * leaves the built-in prompt, or the parent role's, in force; the built-in text
 * is at hand to start from.
 */
function PromptEditor({ detail, onClose }: { detail: RoleDetail; onClose: () => void }) {
  const r = detail.role;
  const file = detail.file.content;
  const builtin = detail.builtin === null ? undefined : (splitFrontmatter(detail.builtin)?.body ?? detail.builtin).trim();
  const [text, setText] = useState(() => (file === null ? "" : (splitFrontmatter(file)?.body ?? file).trim()));
  const [view, setView] = useState<"text" | "preview">("text");
  const [showBuiltin, setShowBuiltin] = useState(false);
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const save = useSaveConfig();
  const toast = useToast();
  const fallback = r.extends ? `промпт роли ${r.extends}` : builtin !== undefined ? "встроенный промпт" : undefined;
  const shown = text.trim() || (r.extends ? (r.prompt ?? "") : (builtin ?? ""));
  const submit = async () => {
    setBusy(true);
    try {
      await save("role", r.id, { content: setBody(file ?? "---\n---\n", text) }, detail.file.hash);
      toast("Промпт сохранён: агенты получат его со следующего старта сессии");
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal label={`Промпт роли ${r.id}`} onClose={onClose} wide>
      <div className="mh">
        <Icon.file size={13} />
        <span className="ag-prompt-title">
          Промпт роли <span className="mono">{r.id}</span>
        </span>
        <div className="seg ag-prompt-seg" role="tablist" aria-label="Вид">
          <button type="button" role="tab" aria-selected={view === "text"} className={view === "text" ? "on" : ""} onClick={() => setView("text")}>
            Текст
          </button>
          <button type="button" role="tab" aria-selected={view === "preview"} className={view === "preview" ? "on" : ""} onClick={() => setView("preview")}>
            Просмотр
          </button>
        </div>
        <button type="button" className="icon-btn" onClick={onClose} aria-label="Закрыть">
          <Icon.close />
        </button>
      </div>
      <div className="mb">
        <p className="muted ag-hint">
          Markdown. Промпт идёт в системный промпт агента после общих правил genie; настройки роли (разрешения, навыки, MCP) задаются отдельно.
          {fallback ? ` Пустой текст — действует ${fallback}.` : ""}
        </p>
        <div className={`ag-prompt-edit show-${view}`}>
          <textarea
            className="mono code-edit"
            value={text}
            onChange={(e) => setText(e.target.value)}
            spellCheck={false}
            aria-label="Текст промпта"
            placeholder={fallback ? `Пусто — ${fallback}` : "Кто ты в команде, как работаешь, что сдаёшь"}
          />
          <div className="ag-prompt-preview" aria-label="Предпросмотр">
            {!text.trim() && fallback && <div className="ag-hint muted">Сейчас действует {fallback}:</div>}
            {shown ? <Markdown text={shown} /> : <p className="muted">Промпта нет.</p>}
          </div>
        </div>
        {builtin !== undefined && (
          <details className="ag-prompt" open={showBuiltin} onToggle={(e) => setShowBuiltin(e.currentTarget.open)}>
            <summary>Встроенный текст роли</summary>
            <pre className="ag-file-text ag-builtin">{builtin}</pre>
            <button type="button" className="btn" onClick={() => setText(builtin)} disabled={text.trim() === builtin}>
              Взять встроенный
            </button>
          </details>
        )}
        {error && (
          <div className="auth-error" role="alert">
            {error}
          </div>
        )}
      </div>
      <div className="mf">
        {file === null ? "Сохранение создаст файл роли с этим промптом" : `agents/${r.id}.md`}
        <span className="grow" />
        <button type="button" className="btn ghost" onClick={onClose}>
          Отмена
        </button>
        <button type="button" className="btn primary" disabled={busy} onClick={() => void submit()}>
          Сохранить
        </button>
      </div>
    </Modal>
  );
}

/** Save changed fields of the role file. */
function useRoleEdit(detail: RoleDetail) {
  const save = useSaveConfig();
  return (fields: [string, string | string[] | undefined][]) => {
    let text = detail.file.content ?? "---\n---\n";
    for (const [k, v] of fields) text = setFrontmatterKey(text, k, v);
    return save("role", detail.role.id, { content: text }, detail.file.hash);
  };
}

function Permissions({ detail, cfg }: { detail: RoleDetail; cfg: Catalogue }) {
  const r = detail.role;
  const base = basePermissions(r, cfg.roles, cfg.classes);
  const [wanted, setWanted] = useState<string[]>(r.capabilities);
  const edit = useRoleEdit(detail);
  const act = useAction();
  const changed = wanted.length !== r.capabilities.length || wanted.some((c) => !r.capabilities.includes(c));
  const toggle = (c: string, on: boolean) => setWanted((w) => (on ? [...w, c] : w.filter((x) => x !== c)));
  const baseName = r.extends ? `роли «${cfg.roles.find((x) => x.id === r.extends)?.title ?? r.extends}»` : `класса «${CLASS_TITLE[r.class]}»`;
  const diffs = PERMISSION_GROUPS.flatMap((g) => g.items).filter((p) => wanted.includes(p.id) !== base.includes(p.id)).length;
  const save = () => {
    const { allow, deny } = allowDeny(base, wanted, cfg.permissions);
    void act(() => edit([["allow", allow], ["deny", deny]]), "Разрешения сохранены: сервер применяет их сразу");
  };
  return (
    <Section
      title="Разрешения"
      note={diffs ? `Отличий от ${baseName}: ${diffs}` : `Как у ${baseName}`}
      aside={
        detail.admin &&
        changed && (
          <span className="ag-actions">
            <button type="button" className="btn ghost" onClick={() => setWanted(r.capabilities)}>
              Отменить
            </button>
            <button type="button" className="btn primary" onClick={save}>
              Сохранить разрешения
            </button>
          </span>
        )
      }
    >
      <div className="ag-perms">
        {PERMISSION_GROUPS.map((g) => (
          <fieldset key={g.title} disabled={!detail.admin}>
            <legend>{g.title}</legend>
            {g.items.map((p) => {
              const on = wanted.includes(p.id);
              const inBase = base.includes(p.id);
              return (
                <label key={p.id} className={`ag-perm${on ? " on" : ""}`} title={`${p.id}: ${p.text}`}>
                  <input type="checkbox" checked={on} onChange={(e) => toggle(p.id, e.target.checked)} />
                  <span className="mono">{p.id}</span>
                  <span className="txt">{p.text}</span>
                  {on !== inBase && <span className={`ag-tag ${on ? "plus" : "minus"}`}>{on ? "добавлено" : "убрано"}</span>}
                </label>
              );
            })}
          </fieldset>
        ))}
      </div>
      <p className="muted ag-note">Разрешения проверяет сервер, отзыв действует сразу. {FIXED_PERMISSIONS}</p>
    </Section>
  );
}

function Card({ title, onEdit, children }: { title: string; onEdit?: () => void; children: ReactNode }) {
  return (
    <section className="ag-card">
      <div className="ag-card-head">
        <h3>{title}</h3>
        {onEdit && (
          <button type="button" className="btn ghost sm" onClick={onEdit}>
            Изменить
          </button>
        )}
      </div>
      {children}
    </section>
  );
}

const SOFT = "Соблюдает харнесс агента: через shell агент может обойти это ограничение, пока агенты не работают в контейнерах";

function Workplace({ role: r, sandbox, onEdit }: { role: RoleDef; sandbox?: Catalogue["sandbox"]; onEdit?: () => void }) {
  const soft = (
    <span className="ag-soft" title={SOFT}>
      мягко
    </span>
  );
  return (
    <Card title="Рабочее место" onEdit={onEdit}>
      <dl className="ag-dl">
        <dt>Файлы</dt>
        <dd>
          {FILES_TITLE[r.files]}
          {r.files !== "write" && soft}
        </dd>
        <dt>Команды</dt>
        <dd>
          {r.denyCommands.length ? (
            <>
              нельзя <span className="mono">{r.denyCommands.join(", ")}</span>
              {soft}
            </>
          ) : (
            "без запретов"
          )}
        </dd>
        <dt>Изоляция</dt>
        <dd>
          {sandbox?.active ? (
            <span title="Агент работает в песочнице bubblewrap: пишет только в свой рабочий каталог, не видит данные сервера, другие проекты и секреты">
              песочница
            </span>
          ) : (
            <span className="ag-soft" title={sandbox?.note}>
              нет песочницы
            </span>
          )}
        </dd>
        <dt>Стадии</dt>
        <dd>{r.stages.map((s) => STAGE_TITLE[s]).join(", ") || "ни одной"}</dd>
        <dt>Модель</dt>
        <dd title={r.model ? undefined : "Модель из roleModels сервера, иначе модель pi по умолчанию"}>
          {r.model ? `${r.model}${r.thinking ? ` · ${r.thinking}` : ""}` : <span className="muted">по умолчанию</span>}
        </dd>
        {r.projects && (
          <>
            <dt>Проекты</dt>
            <dd>{r.projects.join(", ")}</dd>
          </>
        )}
        {r.names.length > 0 && (
          <>
            <dt>Имена</dt>
            <dd>{r.names.join(", ")}</dd>
          </>
        )}
      </dl>
    </Card>
  );
}

function SkillsCard({ role: r, cfg, onEdit }: { role: RoleDef; cfg: Catalogue; onEdit?: () => void }) {
  const missing = (r.skills ?? []).filter((s) => !cfg.skills.some((x) => x.name === s));
  return (
    <Card title="Навыки" onEdit={onEdit}>
      {r.skills?.length ? (
        <div className="ag-chips">
          {r.skills.map((s) => (
            <span key={s} className={`ag-chip mono${missing.includes(s) ? " missing" : ""}`} title={cfg.skills.find((x) => x.name === s)?.description}>
              {s}
            </span>
          ))}
        </div>
      ) : null}
      <p className="muted ag-empty">{r.skills === undefined ? "Все навыки pi и навыки репозитория." : r.skills.length ? "Только эти и навыки репозитория." : "Только навыки репозитория."}</p>
      {missing.length > 0 && <p className="ag-warn">Не установлены: {missing.join(", ")} — агент их не получит.</p>}
    </Card>
  );
}

/** A grant (`server`, `server:tools`, `*`) as its connection and tools. */
const splitGrant = (g: string) => (g.includes(":") ? [g.slice(0, g.indexOf(":")), g.slice(g.indexOf(":") + 1)] : [g, ""]);

function McpCard({ role: r, cfg, onEdit }: { role: RoleDef; cfg: Catalogue; onEdit?: () => void }) {
  return (
    <Card title="MCP" onEdit={onEdit}>
      {cfg.mcpAdapterLoaded && r.mcp.length > 0 && (
        <p className="ag-warn-line" title="Уберите на сервере: pi remove npm:pi-mcp-adapter">
          pi загружает pi-mcp-adapter: он подменяет встроенную поддержку MCP, его инструменты агентам закрыты.
        </p>
      )}
      {r.mcp.length ? (
        <ul className="ag-grant-list">
          {r.mcp.map((g) => {
            const [server, tools] = splitGrant(g);
            const s = cfg.mcp.find((x) => x.id === server);
            return (
              <li key={g} title={s?.description}>
                <span className="mono">{g === "*" ? "все" : server}</span>
                <span className="muted">{tools || "все инструменты"}</span>
                {s && <Badge tone={s.gateway ? "green" : undefined}>{s.gateway ? "через genie" : "напрямую"}</Badge>}
              </li>
            );
          })}
        </ul>
      ) : (
        <p className="muted ag-empty">Подключений нет.</p>
      )}
    </Card>
  );
}

const THINKING = ["", "off", "minimal", "low", "medium", "high", "xhigh", "max"];
const list = (s: string) =>
  s
    .split(/[,\n]/)
    .map((x) => x.trim())
    .filter(Boolean);

/** A dialog that edits part of the role file. */
function EditDialog({ title, note, busy, narrow, onClose, onSave, children }: { title: string; note?: string; busy?: boolean; narrow?: boolean; onClose: () => void; onSave: () => void; children: ReactNode }) {
  return (
    <Modal label={title} onClose={onClose} wide={!narrow}>
      <div className="mh">
        {title}
        <span className="grow" />
        <button type="button" className="icon-btn" onClick={onClose} aria-label="Закрыть">
          <Icon.close />
        </button>
      </div>
      <form
        className="mb ag-form"
        id="ag-edit"
        onSubmit={(e) => {
          e.preventDefault();
          onSave();
        }}
      >
        {children}
      </form>
      <div className="mf">
        {note}
        <span className="grow" />
        <button type="button" className="btn ghost" onClick={onClose}>
          Отмена
        </button>
        <button type="submit" form="ag-edit" className="btn primary" disabled={busy}>
          Сохранить
        </button>
      </div>
    </Modal>
  );
}

function SettingsDialog({ detail, onClose }: { detail: RoleDetail; onClose: () => void }) {
  const r = detail.role;
  const initial = {
    title: r.title,
    description: r.description,
    model: r.model ?? "",
    thinking: r.thinking ?? "",
    files: r.files as string,
    stages: r.stages as string[],
    projects: r.projects ?? [],
    denyCommands: r.denyCommands,
    names: r.names,
    instructions: r.instructions ?? "",
  };
  const [f, setF] = useState(initial);
  const [busy, setBusy] = useState(false);
  const edit = useRoleEdit(detail);
  const act = useAction();
  const meta = useMeta().data;
  const projects = useProjects().data;
  const set = <K extends keyof typeof f>(k: K, v: (typeof f)[K]) => setF((x) => ({ ...x, [k]: v }));
  const save = async () => {
    const fields: [string, string | string[] | undefined][] = [];
    const text = (k: "title" | "description" | "model" | "thinking" | "instructions") => {
      if (f[k] !== initial[k]) fields.push([k, f[k].trim() ? f[k].trim() : undefined]);
    };
    const items = (k: "projects" | "denyCommands" | "names") => {
      if (f[k].join("\n") !== initial[k].join("\n")) fields.push([k, f[k].length ? f[k] : undefined]);
    };
    text("title");
    text("description");
    text("model");
    text("thinking");
    text("instructions");
    if (f.files !== initial.files) fields.push(["files", f.files]);
    if (f.stages.join() !== initial.stages.join()) fields.push(["stages", f.stages]);
    items("projects");
    items("denyCommands");
    items("names");
    if (!fields.length) return onClose();
    setBusy(true);
    if (await act(() => edit(fields), "Настройки роли сохранены")) onClose();
    setBusy(false);
  };
  const hint = (text: string) => <span className="hint">{text}</span>;
  return (
    <EditDialog title={`Настройки роли ${r.id}`} note="Промпт, модель и навыки дойдут до агентов со следующего старта их сессии" busy={busy} onClose={onClose} onSave={() => void save()} narrow>
      <div className="ag-settings">
        <section>
          <h4>О роли</h4>
          <label htmlFor="rs-title">Название</label>
          <input id="rs-title" className="in" value={f.title} onChange={(e) => set("title", e.target.value)} required />
          <label htmlFor="rs-desc">
            Описание
            {hint("Видят оркестратор и люди при выборе роли")}
          </label>
          <textarea id="rs-desc" className="in" rows={2} value={f.description} onChange={(e) => set("description", e.target.value)} />
        </section>
        <section>
          <h4>Модель</h4>
          <label htmlFor="rs-model">
            Модель
            {hint("Агенту её можно сменить в чате")}
          </label>
          <ModelPicker id="rs-model" value={f.model} onChange={(m) => set("model", m)} fallback={meta?.roleModels?.[r.id]?.model} />
          <span className="lbl">Размышление</span>
          <Segments label="Размышление" value={f.thinking} options={THINKING.map((t) => [t, t || "по умолчанию"])} onChange={(v) => set("thinking", v)} />
        </section>
        <section>
          <h4>Доступ</h4>
          <span className="lbl">Файлы проекта</span>
          <Segments label="Файлы проекта" value={f.files} options={(["write", "read", "none"] as const).map((x) => [x, FILES_TITLE[x]])} onChange={(v) => set("files", v)} />
          <span className="lbl">
            Стадии задачи
            {hint("Когда роль можно звать в команду")}
          </span>
          <div className="ag-toggles">
            {(["refinement", "delivery"] as const).map((s) => {
              const on = f.stages.includes(s);
              return (
                <button key={s} type="button" aria-pressed={on} className={on ? "on" : ""} onClick={() => set("stages", on ? f.stages.filter((x) => x !== s) : [...f.stages, s])}>
                  <Icon.check />
                  {STAGE_TITLE[s]}
                </button>
              );
            })}
          </div>
          <label htmlFor="rs-deny">
            Запрещённые команды
            {hint("Shell; * — любой текст")}
          </label>
          <ChipInput id="rs-deny" values={f.denyCommands} onChange={(v) => set("denyCommands", v)} placeholder="git push*" mono />
        </section>
        <section>
          <h4>Команда</h4>
          <label htmlFor="rs-projects">
            Проекты
            {hint("Пусто — роль видна во всех")}
          </label>
          <ChipInput id="rs-projects" values={f.projects} onChange={(v) => set("projects", v)} placeholder="Добавить проект" suggestions={projects?.map((p) => p.slug)} />
          <label htmlFor="rs-names">
            Имена участников
            {hint("Пусто — имена по классу")}
          </label>
          <ChipInput id="rs-names" values={f.names} onChange={(v) => set("names", v)} placeholder="Новое имя" />
        </section>
        <section>
          <label htmlFor="rs-extra">
            Особенности роли
            {hint("Дописываются к промпту")}
          </label>
          <textarea id="rs-extra" className="in" rows={3} value={f.instructions} onChange={(e) => set("instructions", e.target.value)} />
        </section>
      </div>
    </EditDialog>
  );
}

/** One choice of a few, as a segmented control. */
function Segments({ label, value, options, onChange }: { label: string; value: string; options: [string, string][]; onChange: (v: string) => void }) {
  return (
    <div role="radiogroup" aria-label={label} className="mm-seg ag-seg">
      {options.map(([v, title]) => (
        <button key={v || "default"} type="button" role="radio" aria-checked={value === v} className={value === v ? "on" : ""} onClick={() => onChange(v)}>
          {title}
        </button>
      ))}
    </div>
  );
}

function SkillsDialog({ detail, cfg, onClose }: { detail: RoleDetail; cfg: Catalogue; onClose: () => void }) {
  const r = detail.role;
  const [only, setOnly] = useState(r.skills !== undefined);
  const [chosen, setChosen] = useState<string[]>(r.skills ?? []);
  const [busy, setBusy] = useState(false);
  const edit = useRoleEdit(detail);
  const act = useAction();
  const save = async () => {
    setBusy(true);
    if (await act(() => edit([["skills", only ? chosen : undefined]]), "Навыки роли сохранены")) onClose();
    setBusy(false);
  };
  return (
    <EditDialog title={`Навыки роли ${r.id}`} note="Агенты получат навыки со следующего старта сессии" busy={busy} onClose={onClose} onSave={() => void save()}>
      <div className="ag-choice">
        <label>
          <input type="radio" checked={!only} onChange={() => setOnly(false)} />
          Все навыки, установленные для pi, и навыки репозитория
        </label>
        <label>
          <input type="radio" checked={only} onChange={() => setOnly(true)} />
          Только выбранные и навыки репозитория
        </label>
        {only && (
          <div className="ag-checks">
            {cfg.skills.map((s) => (
              <label key={s.name} title={s.description}>
                <input type="checkbox" checked={chosen.includes(s.name)} onChange={(e) => setChosen((c) => (e.target.checked ? [...c, s.name] : c.filter((x) => x !== s.name)))} />
                <span className="mono">{s.name}</span>
                <span className="muted">{s.description}</span>
              </label>
            ))}
            {!cfg.skills.length && <span className="muted">В библиотеке пока нет навыков: добавьте их на вкладке «Навыки».</span>}
          </div>
        )}
      </div>
    </EditDialog>
  );
}

type Grant = { mode: "none" | "all" | "tools"; tools: string };

function McpDialog({ detail, cfg, onClose }: { detail: RoleDetail; cfg: Catalogue; onClose: () => void }) {
  const r = detail.role;
  const [grants, setGrants] = useState(
    (): Record<string, Grant> =>
      Object.fromEntries(
        cfg.mcp.map((s) => {
          const whole = r.mcp.includes(s.id) || r.mcp.includes("*");
          const tools = r.mcp.filter((g) => g.startsWith(`${s.id}:`)).map((g) => g.slice(s.id.length + 1));
          return [s.id, { mode: whole ? "all" : tools.length ? "tools" : "none", tools: tools.join(", ") } satisfies Grant];
        }),
      ),
  );
  const [busy, setBusy] = useState(false);
  const edit = useRoleEdit(detail);
  const act = useAction();
  const unknown = r.mcp.filter((g) => g !== "*" && !cfg.mcp.some((s) => s.id === g.split(":")[0]));
  const save = async () => {
    const out: string[] = [...unknown];
    for (const s of cfg.mcp) {
      const g = grants[s.id];
      if (g.mode === "all") out.push(s.id);
      if (g.mode === "tools") out.push(...list(g.tools).map((t) => `${s.id}:${t}`));
    }
    setBusy(true);
    if (await act(() => edit([["mcp", out.length ? out : undefined]]), "Доступ к MCP сохранён")) onClose();
    setBusy(false);
  };
  return (
    <EditDialog title={`MCP роли ${r.id}`} note="Закрытое подключение перестаёт работать сразу; новое агент получит со следующего старта сессии" busy={busy} onClose={onClose} onSave={() => void save()}>
      <div className="ag-grants">
        {cfg.mcp.map((s) => {
          const g = grants[s.id];
          const set = (v: Partial<Grant>) => setGrants((x) => ({ ...x, [s.id]: { ...x[s.id], ...v } }));
          return (
            <div key={s.id} className="ag-grant">
              <span className="mono">{s.id}</span>
              <span className="muted">{s.description}</span>
              <select value={g.mode} onChange={(e) => set({ mode: e.target.value as Grant["mode"] })} aria-label={`Доступ к ${s.id}`}>
                <option value="none">нет доступа</option>
                <option value="all">все инструменты</option>
                <option value="tools">только инструменты…</option>
              </select>
              {g.mode === "tools" && <input value={g.tools} onChange={(e) => set({ tools: e.target.value })} placeholder="get_*, list_commits" aria-label={`Инструменты ${s.id}`} />}
            </div>
          );
        })}
      </div>
    </EditDialog>
  );
}
