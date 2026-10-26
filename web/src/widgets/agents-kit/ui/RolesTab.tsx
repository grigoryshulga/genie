// Roles: the list, a role's page and its editors. Every edit rewrites fields of
// the role's file (`<data>/agents/<id>.md`); for a built-in role without a file
// that creates an override holding only the changed fields. Settings are edited
// on the page; prompt and raw-file editors live in their own files nearby.

import { Fragment, type ReactNode, useState } from "react";
import { Link } from "react-router";
import {
  sectionProblems,
  type Catalogue,
  CLASS_TITLE,
  FILES_TITLE,
  ORIGIN_TITLE,
  type RoleDef,
  type RoleDetail,
  STAGE_TITLE,
  setFrontmatterKey,
  useDeleteConfig,
  useRole,
  useSaveConfig,
} from "@/entities/agent-config";
import { agentTone } from "@/entities/member";
import { Badge, ConfirmDialog, Icon, IconButton, LoadingState, RenameButton, RenameField, Section, useAct } from "@/shared/ui";
import { FileEditor } from "./FileEditor.tsx";
import { History } from "./History.tsx";
import { Problems } from "./Problems.tsx";
import { PermissionFields, PermissionReference } from "./PermissionFields.tsx";
import { PromptEditor, PromptView } from "./PromptDialog.tsx";
import { RoleSettings } from "./RoleSettings.tsx";

export function RolesTab({
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
  const [search, setSearch] = useState("");
  const [editingRole, setEditingRole] = useState<string>();
  const [pendingRole, setPendingRole] = useState<string>();
  const matches = (r: RoleDef) => `${r.title} ${r.id} ${r.description}`.toLocaleLowerCase().includes(search.toLocaleLowerCase());
  const team = cfg.roles.filter((r) => r.class !== "orchestrator");
  const orch = cfg.roles.filter((r) => r.class === "orchestrator");
  const current = selected ?? team[0]?.id ?? orch[0]?.id;
  const pick = (r: RoleDef) => (
    <button type="button" key={r.id} className={`pick${r.id === current ? " on" : ""}`} onClick={() => { if (r.id === current) return; if (editingRole === current) setPendingRole(r.id); else onSelect(r.id); }} aria-current={r.id === current}>
      <span className="t">
        <span className="ag-class" data-tone={agentTone(r.id, cfg.roles)} aria-hidden="true">
          {r.title.slice(0, 1).toUpperCase()}
        </span>
        <span>{r.title}</span>
        {cfg.problems.some((p) => p.item === `role:${r.id}`) && <span className="ag-problem-dot" role="img" aria-label="Есть проблемы" title="Есть проблемы в настройках роли">!</span>}
      </span>
    </button>
  );
  return (
    <div className="split ag-roles-layout">
      <section className="split-list ag-list" aria-label="Роли">
        <div className="side-head"><span className="side-title">Роли</span></div>
        <div className="ag-list-search"><input type="search" className="g-input" aria-label="Поиск ролей" placeholder="Найти роль…" value={search} onChange={(e) => setSearch(e.target.value)} /></div>
        <Problems items={sectionProblems(cfg.problems, "roles").filter((p) => p.item !== `role:${current}`)} />
        {team.filter(matches).map(pick)}
        {orch.filter(matches).length > 0 && <span className="label">Оркестратор</span>}
        {orch.filter(matches).map(pick)}
        {!cfg.roles.some(matches) && <p className="g-muted ag-list-note">Ничего не найдено.</p>}
      </section>
      <section className="split-main" aria-label="Роль">
        {current ? <RolePane key={current} id={current} cfg={cfg} readOnly={readOnly} base={base} onEditingChange={(editing) => setEditingRole(editing ? current : undefined)} /> : <div className="ag-pane-empty">Ролей нет</div>}
      </section>
      {pendingRole && <ConfirmDialog title="Отменить изменения роли?" confirmLabel="Отменить и перейти" onClose={() => setPendingRole(undefined)} onConfirm={() => {
        setEditingRole(undefined);
        onSelect(pendingRole);
        setPendingRole(undefined);
      }}>Несохранённые изменения будут потеряны.</ConfirmDialog>}
    </div>
  );
}

type Dialog = "file" | "prompt" | "delete";

function RolePane({ id, cfg, readOnly = false, base = "/agents", onEditingChange }: { id: string; cfg: Catalogue; readOnly?: boolean; base?: string; onEditingChange: (editing: boolean) => void }) {
  const detail = useRole(id);
  const [dialog, setDialog] = useState<Dialog>();
  const remove = useDeleteConfig();
  const saveConfig = useSaveConfig();
  const [renaming, setRenaming] = useState(false);
  const [editing, setEditing] = useState(false);
  const edit = (value: boolean) => { setEditing(value); onEditingChange(value); };
  const act = useAct();
  const d = detail.data;
  if (!d) {
    if (detail.error) return <div className="ag-pane-empty">{detail.error.message}</div>;
    return <div className="ag-pane-empty"><LoadingState label="Загрузка роли" lines={3} width={260} /></div>;
  }
  const r = d.role;
  const admin = d.admin && !readOnly;
  const hasFile = d.file.content !== null;
  const close = () => setDialog(undefined);
  // A rename is one frontmatter field of the role file, saved like any other edit of it.
  const rename = (title: string) => {
    const text = setFrontmatterKey(d.file.content ?? "---\n---\n", "title", title);
    return act(() => saveConfig("role", r.id, { content: text }, d.file.hash), "Название роли сохранено");
  };
  const parent = r.extends ? (cfg.roles.find((x) => x.id === r.extends)?.title ?? r.extends) : undefined;
  const used = [
    ...d.usedBy.templates.map((t) => (
      <Link key={`t-${t}`} to={`${base}?tab=templates&id=${t}`}>
        шаблон «{cfg.teams.find((x) => x.id === t)?.title ?? t}»
      </Link>
    )),
    ...d.usedBy.automations.map((a) => (
      <Link key={`a-${a.id}`} to={`/manage/project/automations?rule=${a.id}`}>
        автоматизация «{a.name}»{a.project ? ` (${a.project})` : ""}
      </Link>
    )),
  ];
  return (
    <>
      <div className="pane-head">
        <div className="ttl">
          {admin && !editing && renaming ? (
            <RenameField className="ag-role-rename" value={r.title} label="Название роли" onCommit={(title) => { setRenaming(false); void rename(title); }} onCancel={() => setRenaming(false)} />
          ) : (
            <div className={admin ? "g-rename-host" : undefined}>
              <h2>
                {r.title}
                {!admin && <span className="ag-readonly">Только чтение</span>}
              </h2>
              {admin && !editing && <RenameButton label="Переименовать роль" onClick={() => setRenaming(true)} />}
            </div>
          )}
          {r.description && <p className="desc">{r.description}</p>}
        </div>
        {admin && !editing && (
          <IconButton label="Редактировать роль" onClick={() => edit(true)}>
            <Icon.pencil size={14} />
          </IconButton>
        )}
        {admin && !editing && hasFile && (
          <IconButton
            label={d.builtin !== null ? "Вернуть встроенную" : "Удалить роль"}
            onClick={() => (d.builtin !== null ? void act(() => remove("role", r.id, d.file.hash), "Роль снова встроенная") : setDialog("delete"))}
          >
            <Icon.trash size={14} />
          </IconButton>
        )}
        <button type="button" className="g-btn secondary" disabled={editing} onClick={() => setDialog("prompt")}>
          Промпт
        </button>
      </div>
      <div className="pane-body">
        <Problems items={d.problems} />
        {editing && admin ? <RoleSettings detail={d} cfg={cfg} onClose={() => edit(false)} /> : <>
        <div className="ag-role-overview">
          <Workplace role={r} sandbox={cfg.sandbox} />
        </div>
        {r.instructions && <Card title="Особенности роли"><p className="ag-role-instructions">{r.instructions}</p></Card>}
        {r.class === "orchestrator" ? (
          <Section title="Разрешённые действия">
            <p className="g-muted ag-empty">Права оркестратора фиксированы: он разбирает задачи, собирает команды и принимает работу.</p>
          </Section>
        ) : (
          <Permissions detail={d} />
        )}
        <div className="ag-cards ag-role-sections">
          <SkillsCard role={r} cfg={cfg} />
          <McpCard role={r} cfg={cfg} />
        </div>
        </>}
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
        <details className="ag-advanced">
          <summary>Технические сведения и обслуживание</summary>
          <p className="g-muted ag-note"><code>{r.id}</code> · {ORIGIN_TITLE[r.origin]} · {CLASS_TITLE[r.class]}{parent ? ` · на основе «${parent}»` : ""}{r.path ? ` · ${r.path}` : ""}</p>
          <div className="ag-actions">
            {admin && !editing && (
              <button type="button" className="g-btn secondary" onClick={() => setDialog("file")}>
                <Icon.file size={13} />
                Файл
              </button>
            )}
          </div>
        </details>
        {admin && <details className="ag-advanced"><summary>История изменений</summary><History item={`role:${r.id}`} admin={admin} /></details>}
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

function Permissions({ detail }: { detail: RoleDetail }) {
  const r = detail.role;
  return <details className="ag-advanced ag-permissions-view">
    <summary>Разрешённые действия · {r.capabilities.length}</summary>
    <PermissionFields wanted={r.capabilities} />
    <PermissionReference />
  </details>;
}

function Card({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="ag-card">
      <div className="ag-card-head">
        <h3>{title}</h3>
      </div>
      {children}
    </section>
  );
}

const SOFT = "Соблюдает харнесс агента: через shell агент может обойти это ограничение, пока агенты не работают в контейнерах";

function Workplace({ role: r, sandbox }: { role: RoleDef; sandbox?: Catalogue["sandbox"] }) {
  const soft = (
    <span className="ag-soft" title={SOFT}>
      мягко
    </span>
  );
  return <>
    <Card title="Модель">
      <dl className="ag-dl">
        <dt>Модель</dt><dd>{r.model || <span className="g-muted">по умолчанию</span>}</dd>
        <dt>Размышление</dt><dd>{r.thinking || "по умолчанию"}</dd>
      </dl>
    </Card>
    <Card title="Файлы и команды">
      <dl className="ag-dl">
        <dt>Файлы</dt><dd>{FILES_TITLE[r.files]}{r.files !== "write" && soft}</dd>
        <dt>Команды</dt><dd>{r.denyCommands.length ? <>нельзя <span className="g-mono">{r.denyCommands.join(", ")}</span>{soft}</> : "без запретов"}</dd>
        <dt>Изоляция</dt><dd title={sandbox?.note}>{sandbox?.active ? "песочница" : "нет песочницы"}</dd>
      </dl>
    </Card>
    <Card title="Участие в работе">
      <dl className="ag-dl">
        <dt>Стадии</dt><dd>{r.stages.map((s) => STAGE_TITLE[s]).join(", ") || "ни одной"}</dd>
        <dt>Проекты</dt><dd>{r.projects?.join(", ") || "все проекты"}</dd>
        <dt>Имена</dt><dd>{r.names.join(", ") || "по классу роли"}</dd>
      </dl>
    </Card>
  </>;
}

function SkillsCard({ role: r, cfg }: { role: RoleDef; cfg: Catalogue }) {
  const missing = (r.skills ?? []).filter((s) => !cfg.skills.some((x) => x.name === s));
  return (
    <Card title="Навыки">
      {r.skills?.length ? (
        <div className="ag-chips">
          {r.skills.map((s) => (
            <span key={s} className={`ag-chip mono${missing.includes(s) ? " missing" : ""}`} title={cfg.skills.find((x) => x.name === s)?.description}>
              {s}
            </span>
          ))}
        </div>
      ) : null}
      <p className="g-muted ag-empty">{r.skills === undefined ? "Все навыки pi и навыки репозитория." : r.skills.length ? "Только эти и навыки репозитория." : "Только навыки репозитория."}</p>
      {missing.length > 0 && <p className="ag-warn">Не установлены: {missing.join(", ")} — агент их не получит.</p>}
    </Card>
  );
}

/** Keep long tool lists behind one disclosure per connection. Unknown grants stay visible. */
function McpCard({ role: r, cfg }: { role: RoleDef; cfg: Catalogue }) {
  const groups = new Map<string, string[]>();
  for (const grant of r.mcp) {
    const colon = grant.indexOf(":");
    const server = colon < 0 ? grant : grant.slice(0, colon);
    const tools = groups.get(server) ?? [];
    tools.push(colon < 0 ? "*" : grant.slice(colon + 1));
    groups.set(server, tools);
  }
  return <Card title="Инструменты MCP">
    {groups.size ? <div className="ag-mcp-connections">
      {[...groups].map(([id, tools]) => {
        const server = cfg.mcp.find((s) => s.id === id);
        const all = tools.includes("*");
        return <details key={id} className="ag-mcp-connection">
          <summary>
            <strong>{id === "*" ? "Все подключения" : id}</strong>
            <span className="g-muted">{all ? "все инструменты" : `Выбрано: ${tools.length}`}</span>
            {server && <Badge tone={server.gateway ? "green" : undefined}>{server.gateway ? "через genie" : "напрямую"}</Badge>}
          </summary>
          {server?.description && <p className="g-muted">{server.description}</p>}
          {!server && id !== "*" && <p className="ag-warn">Подключение вне библиотеки.</p>}
          {all ? <p className="g-muted">Доступны все инструменты подключения.</p> : <ul className="ag-mcp-tools">{tools.map((tool) => <li key={tool}><code>{tool}</code></li>)}</ul>}
        </details>;
      })}
    </div> : <p className="g-muted ag-empty">Подключений нет.</p>}
  </Card>;
}
