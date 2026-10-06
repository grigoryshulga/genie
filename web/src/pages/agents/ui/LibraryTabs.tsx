// Skills and MCP connections: what the library holds, who uses it, and the
// editors for administrators.

import { Fragment, useState } from "react";
import { Link } from "react-router";
import {
  type Catalogue,
  checkMcp,
  type McpCheck,
  type RoleDef,
  splitFrontmatter,
  toolUsers,
  useDeleteConfig,
  useMcpCalls,
  useMcpConfig,
  useSkill,
  useSkillFile,
  useSkillFiles,
} from "@/entities/agent-config";
import { displayName } from "@/entities/member";
import { bytes, plural, timeAgo } from "@/shared/lib";
import { ConfirmDialog, Icon, Markdown, Modal } from "@/shared/ui";
import { Badge, FileEditor, History, Problems, Section, useAction } from "./common.tsx";

export function SkillsTab({ cfg, selected, onSelect }: { cfg: Catalogue; selected?: string; onSelect: (name: string) => void }) {
  const current = selected ?? cfg.skills[0]?.name;
  const users = (name: string) => cfg.roles.filter((r) => r.skills?.includes(name));
  return (
    <div className="split">
      <section className="split-list" aria-label="Навыки">
        {cfg.skills.map((s) => (
          <button type="button" key={s.name} className={`pick plain${s.name === current ? " on" : ""}`} onClick={() => onSelect(s.name)} aria-current={s.name === current}>
            <span className="t">
              <span className="mono">{s.name}</span>
            </span>
            <span className="s">{users(s.name).length ? users(s.name).map((r) => r.title).join(", ") : "все роли без своего списка навыков"}</span>
          </button>
        ))}
        {!cfg.skills.length && (
          <p className="muted ag-list-note">
            Навыков пока нет. Навык — каталог с <span className="mono">SKILL.md</span> в <span className="mono">&lt;data&gt;/skills</span> или в каталогах{" "}
            <span className="mono">skills.paths</span>; готовые наборы (anthropics/skills, pi-skills) подключаются как есть.
          </p>
        )}
      </section>
      <section className="split-main" aria-label="Навык">
        {current ? <SkillPane key={current} name={current} cfg={cfg} /> : <div className="pane-empty">Выберите или добавьте навык</div>}
      </section>
    </div>
  );
}

function SkillPane({ name, cfg }: { name: string; cfg: Catalogue }) {
  const skill = useSkill(name);
  const [editing, setEditing] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const remove = useDeleteConfig();
  const act = useAction();
  const d = skill.data;
  if (!d) return <div className="pane-empty">{skill.error ? skill.error.message : "Загрузка…"}</div>;
  const body = d.content ? (splitFrontmatter(d.content)?.body ?? d.content) : "";
  return (
    <>
      <div className="pane-head">
        <div className="ttl">
          <h2 className="mono">{d.skill.name}</h2>
          <span className="sub mono">{d.skill.dir}</span>
          {d.skill.description && <p className="desc">{d.skill.description}</p>}
        </div>
        {d.admin && d.editable && (
          <>
            <button type="button" className="btn" onClick={() => setEditing(true)}>
              <Icon.file size={13} />
              SKILL.md
            </button>
            <button type="button" className="btn ghost" onClick={() => setConfirmDelete(true)}>
              <Icon.trash size={13} />
              Удалить
            </button>
          </>
        )}
      </div>
      <div className="pane-body">
        {d.admin && !d.editable && <p className="muted ag-note">Навык из каталога skills.paths: его меняют там, где он лежит (например, в клоне репозитория с навыками).</p>}
        <Section title="Инструкции">
          <div className="ag-skill-body">
            <Markdown text={body} />
          </div>
        </Section>
        <SkillFiles name={d.skill.name} files={d.files} canEdit={d.admin && d.editable} />
        <p className="ag-used">
          {d.usedBy.length ? (
            <>
              Используют роли:{" "}
              {d.usedBy.map((r, i) => (
                <Fragment key={r}>
                  {i > 0 && ", "}
                  <Link to={`/agents?tab=roles&id=${r}`}>{cfg.roles.find((x) => x.id === r)?.title ?? r}</Link>
                </Fragment>
              ))}
              .
            </>
          ) : (
            "Роли его не перечисляют: он достаётся ролям без своего списка навыков, если установлен и для pi."
          )}
        </p>
        {d.admin && (
          <Section title="История изменений">
            <History item={`skill:${d.skill.name}`} admin={d.admin} />
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
  const act = useAction();
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
      <label className={`btn${busy ? " disabled" : ""}`}>
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
                <button type="button" className="icon-btn" aria-label={`Удалить ${f}`} onClick={() => setRemoving(f)}>
                  <Icon.trash size={13} />
                </button>
              )}
            </li>
          ))}
        </ul>
      ) : (
        <p className="muted ag-empty">Кроме SKILL.md файлов нет. Сюда кладут скрипты, справочники и шаблоны, на которые ссылаются инструкции навыка (до 5 МБ каждый).</p>
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
    <Modal label={path} onClose={onClose} wide>
      <div className="mh">
        <Icon.file size={13} />
        <span className="mono">{path}</span>
        <span className="grow" />
        <button type="button" className="icon-btn" onClick={onClose} aria-label="Закрыть">
          <Icon.close />
        </button>
      </div>
      <div className="mb">
        {!f.data ? (
          <p className="muted">{f.error ? f.error.message : "Загрузка…"}</p>
        ) : f.data.text !== null ? (
          <pre className="ag-file-text">{f.data.text}</pre>
        ) : (
          <p className="muted">Двоичный или большой файл ({bytes(f.data.size)}): здесь его не показать.</p>
        )}
      </div>
    </Modal>
  );
}

const MCP_EXAMPLE = `{
  "mcpServers": {
    "github": {
      "description": "GitHub: issues, pull requests, code",
      "type": "http",
      "url": "https://api.githubcopilot.com/mcp/",
      "headers": { "Authorization": "Bearer \${env:GITHUB_MCP_TOKEN}" }
    }
  }
}
`;

export function McpTab({ cfg }: { cfg: Catalogue }) {
  const mcp = useMcpConfig();
  const [editing, setEditing] = useState(false);
  const [checks, setChecks] = useState<Record<string, McpCheck | "busy">>({});
  const d = mcp.data;
  const servers = d?.servers ?? cfg.mcp;
  const users = (id: string) => cfg.roles.filter((r) => r.mcp.some((g) => g === "*" || g === id || g.startsWith(`${id}:`)));
  const check = async (id: string) => {
    setChecks((c) => ({ ...c, [id]: "busy" }));
    const result = await checkMcp(id).catch((e: unknown): McpCheck => ({ ok: false, ms: 0, error: e instanceof Error ? e.message : String(e) }));
    setChecks((c) => ({ ...c, [id]: result }));
  };
  const cols = d?.admin ? 5 : 4;
  return (
    <div className="ag-page">
      <div className="ag-page-head">
        <div className="ag-intro">
          <p>
            {cfg.mcpGateway
              ? "Агенты ходят в подключения через шлюз genie: секреты остаются на сервере, агент видит только выданные его роли инструменты, каждый вызов попадает в журнал проекта. Подключение, которое харнесс открывает сам, помечено «напрямую»."
              : "Шлюз genie выключен (runtime.mcpGateway): харнесс получает подключения вместе с секретами, вызовы genie не видит."}
          </p>
          {cfg.mcpAdapterLoaded && (
            <p className="ag-warn-line" title="Уберите на сервере: pi remove npm:pi-mcp-adapter">
              pi загружает pi-mcp-adapter: он подменяет встроенную поддержку MCP, его инструменты агентам закрыты.
            </p>
          )}
        </div>
        {d?.admin && (
          <button type="button" className="btn" onClick={() => setEditing(true)}>
            <Icon.file size={13} />
            mcp.json
          </button>
        )}
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
              {d?.admin && <th aria-label="Проверка" />}
            </tr>
          </thead>
          <tbody>
            {servers.map((s) => {
              const result = checks[s.id];
              const through = cfg.mcpGateway && s.gateway;
              return (
                <Fragment key={s.id}>
                  <tr>
                    <td>
                      <span className="mono">{s.id}</span>
                      {s.description && <div className="muted">{s.description}</div>}
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
                          <Link to={`/agents?tab=roles&id=${r.id}`}>{r.title}</Link>
                        </span>
                      ))}
                      {!users(s.id).length && <span className="muted">ни одной роли</span>}
                    </td>
                    {d?.admin && (
                      <td className="ag-cell-action">
                        <button type="button" className="btn" disabled={result === "busy"} onClick={() => void check(s.id)}>
                          {result === "busy" ? "Проверяю…" : "Проверить"}
                        </button>
                      </td>
                    )}
                  </tr>
                  {result && result !== "busy" && (
                    <tr className="ag-check-row">
                      <td colSpan={cols}>
                        <CheckResult server={s.id} result={result} roles={cfg.roles} />
                      </td>
                    </tr>
                  )}
                </Fragment>
              );
            })}
            {!servers.length && (
              <tr>
                <td colSpan={cols} className="muted">
                  Подключений пока нет.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {cfg.mcpGateway && <Calls project={cfg.project} roles={cfg.roles} />}
      {d?.admin && (
        <Section title="История изменений">
          <History item="mcp" admin />
        </Section>
      )}
      {editing && d?.file && (
        <FileEditor
          kind="mcp"
          id=""
          title="mcp.json"
          hint={
            <>
              Поля genie: <span className="mono">description</span>, <span className="mono">projects</span> и <span className="mono">gateway</span> (
              <span className="mono">false</span> — отдать подключение харнессу напрямую, без шлюза). Секреты не пишите в файл — только{" "}
              <span className="mono">{"${env:ИМЯ}"}</span>.
            </>
          }
          initial={d.file.content ?? MCP_EXAMPLE}
          baseHash={d.file.hash}
          problems={d.problems}
          onClose={() => setEditing(false)}
        />
      )}
    </div>
  );
}

/** What a check found: the tools and which roles get each, or why the connection does not start. */
function CheckResult({ server, result, roles }: { server: string; result: McpCheck; roles: RoleDef[] }) {
  if (!result.ok) {
    return (
      <div className="ag-check bad" role="alert">
        <Badge tone="red">не запускается</Badge>
        <pre>{result.error}</pre>
      </div>
    );
  }
  const tools = result.tools ?? [];
  const info = [result.serverInfo?.name, result.serverInfo?.version].filter(Boolean).join(" ");
  const count = `${tools.length} ${plural(tools.length, "инструмент", "инструмента", "инструментов")}`;
  return (
    <div className="ag-check">
      <div className="ag-check-head">
        <Badge tone="green">работает</Badge>
        <span className="muted">{[info, count, `${result.ms} мс`].filter(Boolean).join(" · ")}</span>
      </div>
      {tools.length > 0 && (
        <ul className="ag-tools" aria-label={`Инструменты ${server}`}>
          {tools.map((t) => {
            const who = toolUsers(roles, server, t.name);
            return (
              <li key={t.name}>
                <span className="mono">{t.name}</span>
                <span className="d muted">{t.description}</span>
                <span className={`who${who.length ? "" : " none"}`}>{who.length ? who.join(", ") : "ни одной роли"}</span>
              </li>
            );
          })}
        </ul>
      )}
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
        <p className="muted ag-empty">{calls.isPending ? "Загрузка…" : "Вызовов через шлюз в этом проекте пока не было."}</p>
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
                      <div className="muted">
                        {roleTitle(p.role).toLowerCase()}
                        {c.subject ? (
                          <>
                            {" · "}
                            <Link to={`/team/${c.subject}`}>{c.subject}</Link>
                          </>
                        ) : null}
                      </div>
                    </td>
                    <td className="mono">
                      {p.server}:{p.tool}
                    </td>
                    <td>
                      <div className="ag-how">
                        <Badge tone={p.refused ? "red" : p.ok ? "green" : "amber"}>{p.refused ? "отказано" : p.ok ? "ок" : "ошибка"}</Badge>
                        <span className="muted">{p.ms} мс</span>
                      </div>
                      {p.refused ? (
                        <div className="muted ag-call-error" title={p.error}>
                          роли не выдан этот инструмент
                        </div>
                      ) : (
                        p.error && <div className="muted ag-call-error">{p.error}</div>
                      )}
                    </td>
                    <td className="mono ag-args">{p.args}</td>
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
