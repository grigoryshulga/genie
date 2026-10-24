// Skills and MCP connections: what the library holds, who uses it, and the
// editors for administrators.

import { Fragment, useState } from "react";
import { Link } from "react-router";
import {
  sectionProblems,
  type Catalogue,
  type RoleDef,
  splitFrontmatter,
  useDeleteConfig,
  useMcpCalls,
  useMcpConfig,
  useSkill,
  useSkillFile,
  useSkillFiles,
} from "@/entities/agent-config";
import { displayName } from "@/entities/member";
import { bytes, timeAgo } from "@/shared/lib";
import { Badge, ConfirmDialog, Dialog, Icon, LoadingState, Markdown, Section, useAct } from "@/shared/ui";
import { FileEditor } from "./FileEditor.tsx";
import { History } from "./History.tsx";
import { Problems } from "./Problems.tsx";
// `pages.css` holds the pane/split-pane chrome these library tabs render.

export function SkillsTab({
  cfg,
  selected,
  onSelect,
  readOnly = false,
  base = "/agents",
}: {
  cfg: Catalogue;
  selected?: string;
  onSelect: (name: string) => void;
  /** The catalogue (/agents) forces read-only even for a server administrator. */
  readOnly?: boolean;
  /** Where this tab lives: cross-links to the roles tab stay inside the same mode. */
  base?: string;
}) {
  const current = selected ?? cfg.skills[0]?.name;
  const users = (name: string) => cfg.roles.filter((r) => r.skills?.includes(name));
  return (
    <div className="split ag-skills-layout">
      <section className="split-list ag-list" aria-label="Навыки">
        <Problems items={sectionProblems(cfg.problems, "skills")} />
        {cfg.skills.map((s) => (
          <button type="button" key={s.name} className={`pick plain${s.name === current ? " on" : ""}`} onClick={() => onSelect(s.name)} aria-current={s.name === current}>
            <span className="t">
              <span className="g-mono">{s.name}</span>
            </span>
            <span className="s">{users(s.name).length ? users(s.name).map((r) => r.title).join(", ") : "все роли без своего списка навыков"}</span>
          </button>
        ))}
        {!cfg.skills.length && (
          <p className="g-muted ag-list-note">
            Навыков пока нет. Навык — каталог с <span className="g-mono">SKILL.md</span> в <span className="g-mono">&lt;data&gt;/skills</span> или в каталогах{" "}
            <span className="g-mono">skills.paths</span>; готовые наборы (anthropics/skills, pi-skills) подключаются как есть.
          </p>
        )}
      </section>
      <section className="split-main" aria-label="Навык">
        {current ? <SkillPane key={current} name={current} cfg={cfg} readOnly={readOnly} base={base} /> : <div className="ag-pane-empty">Выберите или добавьте навык</div>}
      </section>
    </div>
  );
}

function SkillPane({ name, cfg, readOnly = false, base = "/agents" }: { name: string; cfg: Catalogue; readOnly?: boolean; base?: string }) {
  const skill = useSkill(name);
  const [editing, setEditing] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const remove = useDeleteConfig();
  const act = useAct();
  const d = skill.data;
  if (!d) {
    if (skill.error) return <div className="ag-pane-empty">{skill.error.message}</div>;
    return <div className="ag-pane-empty"><LoadingState label="Загрузка навыка" lines={3} width={260} /></div>;
  }
  const body = d.content ? (splitFrontmatter(d.content)?.body ?? d.content) : "";
  const admin = d.admin && !readOnly;
  return (
    <>
      <div className="pane-head">
        <div className="ttl">
          <h2 className="g-mono">
            {d.skill.name}
            {!admin && <span className="ag-readonly">Только чтение</span>}
          </h2>
          <span className="sub g-mono">{d.skill.dir}</span>
          {d.skill.description && <p className="desc">{d.skill.description}</p>}
        </div>
        {admin && d.editable && (
          <>
            <button type="button" className="g-btn secondary" onClick={() => setEditing(true)}>
              <Icon.file size={13} />
              SKILL.md
            </button>
            <button type="button" className="g-btn ghost" onClick={() => setConfirmDelete(true)}>
              <Icon.trash size={13} />
              Удалить
            </button>
          </>
        )}
      </div>
      <div className="pane-body">
        {admin && !d.editable && <p className="g-muted ag-note">Навык из каталога skills.paths: его меняют там, где он лежит (например, в клоне репозитория с навыками).</p>}
        <Section title="Инструкции">
          <div className="ag-skill-body">
            <Markdown text={body} />
          </div>
        </Section>
        <SkillFiles name={d.skill.name} files={d.files} canEdit={admin && d.editable} />
        <p className="ag-used">
          {d.usedBy.length ? (
            <>
              Используют роли:{" "}
              {d.usedBy.map((r, i) => (
                <Fragment key={r}>
                  {i > 0 && ", "}
                  <Link to={`${base}?tab=roles&id=${r}`}>{cfg.roles.find((x) => x.id === r)?.title ?? r}</Link>
                </Fragment>
              ))}
              .
            </>
          ) : (
            "Роли его не перечисляют: он достаётся ролям без своего списка навыков, если установлен и для pi."
          )}
        </p>
        {admin && (
          <Section title="История изменений">
            <History item={`skill:${d.skill.name}`} admin={admin} />
          </Section>
        )}
      </div>
      {editing && (
        <FileEditor kind="skill" id={d.skill.name} title={`skills/${d.skill.name}/SKILL.md`} initial={d.content ?? ""} baseHash={d.hash} onClose={() => setEditing(false)} />
      )}
      {confirmDelete && (
        <ConfirmDialog
          title={`Удалить навык ${d.skill.name}?`}
          confirmLabel="Удалить"
          danger
          onClose={() => setConfirmDelete(false)}
          onConfirm={() => {
            setConfirmDelete(false);
            void act(() => remove("skill", d.skill.name), "Навык удалён");
          }}
        >
          Каталог навыка удаляется целиком. Роли, которые его перечисляют, получат предупреждение.
        </ConfirmDialog>
      )}
    </>
  );
}

/** The skill's supporting files (scripts, references, templates its instructions name). */
function SkillFiles({ name, files, canEdit }: { name: string; files: string[]; canEdit: boolean }) {
  const others = files.filter((f) => f !== "SKILL.md");
  const [open, setOpen] = useState<string>();
  const [removing, setRemoving] = useState<string>();
  const [folder, setFolder] = useState("");
  const [busy, setBusy] = useState(false);
  const { upload, remove } = useSkillFiles();
  const act = useAct();
  if (!others.length && !canEdit) return null;
  const put = async (list: File[]) => {
    if (!list.length) return;
    const dir = folder.trim().replace(/^\/+|\/+$/g, "");
    setBusy(true);
    await act(
      async () => {
        for (const f of list) await upload(name, dir ? `${dir}/${f.name}` : f.name, f);
      },
      list.length === 1 ? `Загружен ${list[0].name}` : `Загружено файлов: ${list.length}`,
    );
    setBusy(false);
  };
  const aside = canEdit ? (
    <div className="ag-upload">
      <input className="ag-select" value={folder} onChange={(e) => setFolder(e.target.value)} placeholder="в папку, например scripts" aria-label="Папка для файлов" />
      <label className={`g-btn secondary sm${busy ? " disabled" : ""}`}>
        <Icon.plus size={13} />
        {busy ? "Загружаю…" : "Загрузить"}
        <input
          type="file"
          multiple
          hidden
          disabled={busy}
          aria-label="Файлы навыка"
          onChange={(e) => {
            const list = Array.from(e.target.files ?? []);
            e.target.value = "";
            void put(list);
          }}
        />
      </label>
    </div>
  ) : undefined;
  return (
    <Section title="Файлы" aside={aside}>
      {others.length ? (
        <ul className="ag-file-list" aria-label="Файлы навыка">
          {others.map((f) => (
            <li key={f}>
              <button type="button" className="ag-file" onClick={() => setOpen(f)}>
                {f}
              </button>
              {canEdit && (
                <button type="button" className="g-icon-btn" aria-label={`Удалить ${f}`} onClick={() => setRemoving(f)}>
                  <Icon.trash size={13} />
                </button>
              )}
            </li>
          ))}
        </ul>
      ) : (
        <p className="g-muted ag-empty">Дополнительных файлов нет. Лимит файла — 5 МБ.</p>
      )}
      {open && <SkillFileView name={name} path={open} onClose={() => setOpen(undefined)} />}
      {removing && (
        <ConfirmDialog
          title={`Удалить ${removing}?`}
          confirmLabel="Удалить"
          danger
          onClose={() => setRemoving(undefined)}
          onConfirm={() => {
            const f = removing;
            setRemoving(undefined);
            void act(() => remove(name, f), "Файл удалён");
          }}
        >
          Файл удаляется из каталога навыка, опустевшая папка — вместе с ним.
        </ConfirmDialog>
      )}
    </Section>
  );
}

function SkillFileView({ name, path, onClose }: { name: string; path: string; onClose: () => void }) {
  const f = useSkillFile(name, path);
  return (
    <Dialog title={<span className="g-mono">{path}</span>} onClose={onClose} wide>
      <div>
        {!f.data ? (
          f.error ? <p className="g-muted">{f.error.message}</p> : <LoadingState label="Загрузка файла" lines={5} />
        ) : f.data.text !== null ? (
          <pre className="ag-file-text">{f.data.text}</pre>
        ) : (
          <p className="g-muted">Двоичный или большой файл ({bytes(f.data.size)}): здесь его не показать.</p>
        )}
      </div>
    </Dialog>
  );
}

export function McpTab({ cfg, readOnly = false, base = "/agents" }: { cfg: Catalogue; readOnly?: boolean; base?: string }) {
  const mcp = useMcpConfig();
  const d = mcp.data;
  const servers = d?.servers ?? cfg.mcp;
  const users = (id: string) => cfg.roles.filter((r) => r.mcp.some((g) => g === "*" || g === id || g.startsWith(`${id}:`)));
  return (
    <div className="ag-page">
      <div className="ag-page-head">
        <div className="ag-intro">
          <p>
            {cfg.mcpGateway
              ? "Через шлюз: секреты на сервере, вызовы в журнале проекта."
              : "Шлюз выключен: подключения передаются агентам с секретами, без журнала вызовов."}
          </p>
        </div>
        {/* Управление подключениями — в «Настройки» → «Управление» → «MCP»; каталог не редактирует. */}
      </div>
      {d?.problems && <Problems items={d.problems} />}
      <div className="ag-table-wrap">
        <table className="ag-table">
          <thead>
            <tr>
              <th>Подключение</th>
              <th>Как</th>
              <th>Проекты</th>
              <th>Роли с доступом</th>
            </tr>
          </thead>
          <tbody>
            {servers.map((s) => {
              const through = cfg.mcpGateway && s.gateway;
              return (
                <tr key={s.id}>
                  <td>
                    <span className="g-mono">{s.id}</span>
                    {s.description && <div className="g-muted">{s.description}</div>}
                  </td>
                  <td>
                    <div className="ag-how">
                      {s.transport === "http" ? "HTTP" : "команда"}
                      <span title={through ? "Секреты остаются на сервере, вызовы — в журнале проекта" : "Харнесс получает подключение с секретами; вызовы не видны genie"}>
                        <Badge tone={through ? "green" : "amber"}>{through ? "через genie" : "напрямую"}</Badge>
                      </span>
                    </div>
                  </td>
                  <td>{s.projects?.join(", ") ?? "все"}</td>
                  <td>
                    {users(s.id).map((r, i) => (
                      <span key={r.id}>
                        {i > 0 && ", "}
                        <Link to={`${base}?tab=roles&id=${r.id}`}>{r.title}</Link>
                      </span>
                    ))}
                    {!users(s.id).length && <span className="g-muted">ни одной роли</span>}
                  </td>
                </tr>
              );
            })}
            {!servers.length && (
              <tr>
                <td colSpan={4} className="g-muted">
                  Подключений пока нет.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {cfg.mcpGateway && <Calls project={cfg.project} roles={cfg.roles} />}
    </div>
  );
}

/** The project's tool calls through the gateway, newest first. */
function Calls({ project, roles }: { project: string; roles: RoleDef[] }) {
  const roleTitle = (id: string) => roles.find((r) => r.id === id)?.title ?? id;
  const calls = useMcpCalls(project);
  const [server, setServer] = useState("");
  const all = calls.data ?? [];
  const servers = [...new Set(all.map((c) => c.payload.server))].sort();
  const list = all.filter((c) => !server || c.payload.server === server);
  const filter =
    servers.length > 1 ? (
      <select className="ag-select" value={server} onChange={(e) => setServer(e.target.value)} aria-label="Подключение">
        <option value="">все подключения</option>
        {servers.map((s) => (
          <option key={s} value={s}>
            {s}
          </option>
        ))}
      </select>
    ) : undefined;
  return (
    <Section title="Вызовы инструментов" aside={filter}>
      {!list.length ? (
        calls.isPending ? (
          <LoadingState label="Загрузка вызовов" lines={2} width={360} />
        ) : (
          <p className="g-muted ag-empty">Вызовов через шлюз в этом проекте пока не было.</p>
        )
      ) : (
        <div className="ag-table-wrap">
          <table className="ag-table ag-calls">
            <thead>
              <tr>
                <th>Когда</th>
                <th>Агент</th>
                <th>Инструмент</th>
                <th>Итог</th>
                <th>Аргументы</th>
              </tr>
            </thead>
            <tbody>
              {list.map((c) => {
                const p = c.payload;
                return (
                  <tr key={c.id}>
                    <td title={c.at}>{timeAgo(c.at)}</td>
                    <td>
                      {p.job ? `Задание ${p.job}` : displayName(c.actor)}
                      <div className="g-muted">
                        {roleTitle(p.role).toLowerCase()}
                        {c.subject ? (
                          <>
                            {" · "}
                            <Link to={`/team/${c.subject}`}>{c.subject}</Link>
                          </>
                        ) : null}
                      </div>
                    </td>
                    <td className="g-mono">
                      {p.server}:{p.tool}
                    </td>
                    <td>
                      <div className="ag-how">
                        <Badge tone={p.refused ? "red" : p.ok ? "green" : "amber"}>{p.refused ? "отказано" : p.ok ? "ок" : "ошибка"}</Badge>
                        <span className="g-muted">{p.ms} мс</span>
                      </div>
                      {p.refused ? (
                        <div className="g-muted ag-call-error" title={p.error}>
                          роли не выдан этот инструмент
                        </div>
                      ) : (
                        p.error && <div className="g-muted ag-call-error">{p.error}</div>
                      )}
                    </td>
                    <td className="g-mono ag-args">{p.args}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </Section>
  );
}
