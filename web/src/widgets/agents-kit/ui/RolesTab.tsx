// Roles: the list, a role's page and its editors. Every edit rewrites fields of
// the role's file (`<data>/agents/<id>.md`); for a built-in role without a file
// that creates an override holding only the changed fields. The dialogs that
// edit those fields live in their own files nearby.

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
  STAGE_TITLE,
  useDeleteConfig,
  useRole,
} from "@/entities/agent-config";
import { Badge, ConfirmDialog, Icon, Section, useAct } from "@/shared/ui";
import { FileEditor } from "./FileEditor.tsx";
import { History } from "./History.tsx";
import { Problems } from "./Problems.tsx";
import { useRoleEdit } from "./EditDialog.tsx";
import { McpDialog } from "./McpDialog.tsx";
import { PromptEditor, PromptView } from "./PromptDialog.tsx";
import { SettingsDialog } from "./SettingsDialog.tsx";
import { SkillsDialog } from "./SkillsDialog.tsx";

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
        {current ? <RolePane key={current} id={current} cfg={cfg} readOnly={readOnly} base={base} /> : <div className="pane-empty">Ролей нет</div>}
      </section>
    </div>
  );
}

type Dialog = "file" | "prompt" | "settings" | "skills" | "mcp" | "delete";

function RolePane({ id, cfg, readOnly = false, base = "/agents" }: { id: string; cfg: Catalogue; readOnly?: boolean; base?: string }) {
  const detail = useRole(id);
  const [dialog, setDialog] = useState<Dialog>();
  const remove = useDeleteConfig();
  const act = useAct();
  const d = detail.data;
  if (!d) return <div className="pane-empty">{detail.error ? detail.error.message : "Загрузка…"}</div>;
  const r = d.role;
  const admin = d.admin && !readOnly;
  const hasFile = d.file.content !== null;
  const close = () => setDialog(undefined);
  const parent = r.extends ? (cfg.roles.find((x) => x.id === r.extends)?.title ?? r.extends) : undefined;
  const used = [
    ...d.usedBy.templates.map((t) => (
      <Link key={`t-${t}`} to={`${base}?tab=templates&id=${t}`}>
        шаблон «{cfg.teams.find((x) => x.id === t)?.title ?? t}»
      </Link>
    )),
    ...d.usedBy.automations.map((a) => (
      <Link key={`a-${a.id}`} to={`/settings/project/automations?rule=${a.id}`}>
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

function Permissions({ detail, cfg }: { detail: RoleDetail; cfg: Catalogue }) {
  const r = detail.role;
  const base = basePermissions(r, cfg.roles, cfg.classes);
  const [wanted, setWanted] = useState<string[]>(r.capabilities);
  const edit = useRoleEdit(detail);
  const act = useAct();
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

