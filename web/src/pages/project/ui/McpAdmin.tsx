// The MCP tab of /server, for server administrators: the connections of
// mcp.json with checks and hidden tools, the environment variables they read
// and the calls through the gateway. What roles get of a connection is edited
// on /agents.

import { Fragment, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import {
  agentKeys,
  checkMcp,
  deleteMcpSecret,
  globMatch,
  type McpAdmin,
  type McpAdminServer,
  type McpCheck,
  type McpEntry,
  type McpSecretInfo,
  type McpUsageRow,
  setMcpSecret,
  useAgentConfig,
  useDeleteMcpServer,
  useMcpAdmin,
  useMcpUsage,
  useSaveMcpServer,
} from "@/entities/agent-config";
import { useProjects } from "@/entities/project";
import { bytes, plural, timeAgo } from "@/shared/lib";
import { ApiError } from "@/shared/api";
import { Badge, Problems, Section } from "@/pages/agents/ui/common.tsx";
import { ChipInput, ConfirmDialog, Icon, Modal } from "@/shared/ui";
import { useAct } from "./ProjectPage.tsx";
import "@/pages/agents/ui/agents.css";

/** A role as the MCP tab reads it: who holds a grant of a connection. */
interface GrantHolder {
  id: string;
  title: string;
  mcp: string[];
}

export function McpAdminSection() {
  const admin = useMcpAdmin();
  const d = admin.data;
  const [editing, setEditing] = useState<string | null>(); // a connection id, `null` for a new one
  return (
    <div className="st-body sv-wide">
      <div className="st-head">
        <h2>MCP</h2>
        <p>Подключения из mcp.json: агенты ходят в них через шлюз, секреты остаются на сервере. Что из них достаётся ролям, выбирают на странице «Агенты».</p>
      </div>
      {!d ? (
        <p className="muted">{admin.error ? admin.error.message : "Загрузка…"}</p>
      ) : (
        <>
          <Problems items={d.problems} />
          <Servers d={d} onEdit={setEditing} />
          <Secrets d={d} onEdit={setEditing} />
          <Usage />
        </>
      )}
      {d && editing !== undefined && (editing === null || d.servers.some((s) => s.id === editing)) && (
        <ServerEditor key={editing ?? "#new"} server={editing ? d.servers.find((s) => s.id === editing) : undefined} hash={d.file.hash} onClose={() => setEditing(undefined)} />
      )}
    </div>
  );
}

/**
 * A write to mcp.json or the secrets: failures become an error toast, and a
 * 409 (the file changed since it was read) becomes a refetch and a request
 * to redo the edit on fresh data.
 */
function useMcpWrite() {
  const act = useAct();
  const qc = useQueryClient();
  return async (fn: () => Promise<unknown>, ok?: string): Promise<boolean> =>
    act(async () => {
      try {
        await fn();
      } catch (e) {
        if (e instanceof ApiError && e.status === 409) {
          qc.invalidateQueries({ queryKey: agentKeys.mcpAdmin });
          throw new ApiError("mcp.json изменился с момента чтения: данные обновлены, повторите действие");
        }
        throw e;
      }
    }, ok);
}

/** The roles whose grants (`server`, `server:tools`, `*`) name the connection. */
const grantUsers = (roles: GrantHolder[] | undefined, id: string) => (roles ?? []).filter((r) => r.mcp.some((g) => g === "*" || g === id || g.startsWith(`${id}:`)));

/** The connections of mcp.json: the table, the inline check results with the hidden-tools toggles. */
function Servers({ d, onEdit }: { d: McpAdmin; onEdit: (id: string | null) => void }) {
  const save = useSaveMcpServer();
  const del = useDeleteMcpServer();
  const write = useMcpWrite();
  const cfg = useAgentConfig();
  const [checking, setChecking] = useState<string>();
  const [results, setResults] = useState<Record<string, McpCheck>>({});
  const [confirming, setConfirming] = useState<McpAdminServer>();
  const check = async (id: string) => {
    setChecking(id);
    const result = await checkMcp(id).catch((e: unknown): McpCheck => ({ ok: false, ms: 0, error: e instanceof Error ? e.message : String(e) }));
    setChecking(undefined);
    setResults((r) => ({ ...r, [id]: result }));
  };
  const toggleHidden = (s: McpAdminServer, tool: string) => {
    const hidden = s.hiddenTools.some((p) => globMatch(p, tool));
    const next = hidden ? s.hiddenTools.filter((p) => !globMatch(p, tool)) : [...s.hiddenTools, tool];
    const entry: McpEntry = { ...s.entry };
    if (next.length) entry.hiddenTools = next;
    else delete entry.hiddenTools;
    void write(() => save(s.id, entry, d.file.hash), hidden ? "Инструмент снова виден" : "Инструмент скрыт");
  };
  const users = grantUsers(cfg.data?.roles, confirming?.id ?? "");
  return (
    <Section
      title="Подключения"
      note={`${d.servers.length} ${plural(d.servers.length, "подключение", "подключения", "подключений")} в mcp.json`}
      aside={
        <button type="button" className="btn" onClick={() => onEdit(null)}>
          <Icon.plus size={13} />
          Новое подключение
        </button>
      }
    >
      <div className="ag-table-wrap">
        <table className="ag-table">
          <thead>
            <tr>
              <th>Подключение</th>
              <th>Как</th>
              <th>Проекты</th>
              <th>Переменные</th>
              <th>Скрыто</th>
              <th>Проверка</th>
              <th aria-label="Действия" />
            </tr>
          </thead>
          <tbody>
            {d.servers.map((s) => {
              const result = results[s.id];
              const through = (cfg.data?.mcpGateway ?? true) && s.gateway;
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
                      <div className="ag-how">
                        {s.envRefs.map((name) => {
                          const set = d.secrets.find((x) => x.name === name);
                          const why = set ? `значение на сервере${set.updated ? `, обновлено ${timeAgo(set.updated)}` : ""}${set.hint ? ` (${set.hint})` : ""}` : "значения нет — задайте его ниже";
                          return (
                            <span key={name} title={why}>
                              <Badge tone={set ? "green" : undefined}>{name}</Badge>
                            </span>
                          );
                        })}
                        {!s.envRefs.length && <span className="muted">—</span>}
                      </div>
                    </td>
                    <td>
                      {s.hiddenTools.length ? (
                        <span title={s.hiddenTools.join(", ")}>
                          {s.hiddenTools.length} {plural(s.hiddenTools.length, "шаблон", "шаблона", "шаблонов")}
                        </span>
                      ) : (
                        <span className="muted">—</span>
                      )}
                    </td>
                    <td title={s.checkedAt ? `проверено ${timeAgo(s.checkedAt)}` : undefined}>{s.checkedAt ? timeAgo(s.checkedAt) : <span className="muted">не проверялось</span>}</td>
                    <td className="ag-cell-action">
                      <div className="ag-how mc-actions">
                        <button type="button" className="btn" disabled={checking === s.id} onClick={() => void check(s.id)}>
                          {checking === s.id ? "Проверяю…" : "Проверить"}
                        </button>
                        <button type="button" className="btn ghost" onClick={() => onEdit(s.id)}>
                          Изменить
                        </button>
                        <button type="button" className="btn ghost" onClick={() => setConfirming(s)}>
                          Удалить
                        </button>
                      </div>
                    </td>
                  </tr>
                  {result && (
                    <tr className="ag-check-row">
                      <td colSpan={7}>
                        <CheckTools server={s} result={result} roles={cfg.data?.roles} onToggle={(tool) => toggleHidden(s, tool)} />
                      </td>
                    </tr>
                  )}
                </Fragment>
              );
            })}
            {!d.servers.length && (
              <tr>
                <td colSpan={7} className="muted">
                  Подключений пока нет.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {confirming && (
        <ConfirmDialog
          title={`Удалить подключение ${confirming.id}?`}
          confirmLabel="Удалить"
          danger
          onClose={() => setConfirming(undefined)}
          onConfirm={() => {
            const s = confirming;
            setConfirming(undefined);
            void write(() => del(s.id, d.file.hash), "Подключение удалено");
          }}
        >
          Подключение удаляется из mcp.json, кэш его инструментов забывается.
          {users.length > 0 && <> Доступ продолжают перечислять роли ({users.map((r) => r.title).join(", ")}): пока их не поправят, агенты этих ролей подключение не получат.</>}
        </ConfirmDialog>
      )}
    </Section>
  );
}

/** What a check found: the tools, with a way to hide each, or why the connection does not start. */
function CheckTools({ server, result, roles, onToggle }: { server: McpAdminServer; result: McpCheck; roles?: GrantHolder[]; onToggle: (tool: string) => void }) {
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
  const who = grantUsers(roles, server.id);
  return (
    <div className="ag-check">
      <div className="ag-check-head">
        <Badge tone="green">работает</Badge>
        <span className="muted">{[info, count, `${result.ms} мс`].filter(Boolean).join(" · ")}</span>
      </div>
      {tools.length > 0 && (
        <ul className="ag-tools mc-admin-tools" aria-label={`Инструменты ${server.id}`}>
          {tools.map((t) => {
            const hidden = server.hiddenTools.some((p) => globMatch(p, t.name));
            return (
              <li key={t.name}>
                <span className="mono">{t.name}</span>
                <span className="d muted">{t.description}</span>
                <span className={`who${who.length ? "" : " none"}`}>{who.length ? who.map((r) => r.title).join(", ") : "ни одной роли"}</span>
                {hidden && <Badge tone="muted">скрыт</Badge>}
                <button type="button" className="btn ghost" onClick={() => onToggle(t.name)}>
                  {hidden ? "Вернуть" : "Скрыть"}
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

type Kv = { k: string; v: string };

const str = (v: unknown) => (typeof v === "string" ? v : "");
const strList = (v: unknown) => (Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : []);

/** The key → value rows of an entry's header or env object; a non-object (or nothing) is no rows. */
const kvOf = (v: unknown): Kv[] => {
  const o = typeof v === "object" && v !== null && !Array.isArray(v) ? v : {};
  return Object.entries(o).map(([k, value]) => ({ k, v: typeof value === "string" ? value : String(value ?? "") }));
};

/**
 * Create or replace one connection. Two views of the same entry: a form that
 * touches the known fields and keeps the rest of the entry as it is, and the
 * entry as JSON.
 */
function ServerEditor({ server, hash, onClose }: { server?: McpAdminServer; hash: string; onClose: () => void }) {
  const save = useSaveMcpServer();
  const write = useMcpWrite();
  const allProjects = useProjects().data?.map((p) => p.slug);
  const [mode, setMode] = useState<"form" | "json">("form");
  const [id, setId] = useState(server?.id ?? "");
  const [description, setDescription] = useState(server?.description ?? "");
  const [transport, setTransport] = useState<"http" | "stdio">(server?.transport ?? "http");
  const [url, setUrl] = useState(str(server?.entry.url));
  const [headers, setHeaders] = useState<Kv[]>(kvOf(server?.entry.headers));
  const [command, setCommand] = useState(str(server?.entry.command));
  const [args, setArgs] = useState<string[]>(strList(server?.entry.args));
  const [env, setEnv] = useState<Kv[]>(kvOf(server?.entry.env));
  const [projects, setProjects] = useState<string[]>(server?.projects ?? []);
  const [gateway, setGateway] = useState(server?.gateway ?? true);
  const [patterns, setPatterns] = useState<string[]>(server?.hiddenTools ?? []);
  const [json, setJson] = useState(() => JSON.stringify(server?.entry ?? (transport === "http" ? { url: "" } : { command: "" }), null, 2));
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const tools = server?.tools ?? [];
  const knownEntry = server?.entry ?? {};

  /** The entry the form stands for now: known fields replaced, unknown ones kept. */
  const buildEntry = (): McpEntry => {
    const out: McpEntry = { ...knownEntry };
    const put = (key: string, value: unknown) => {
      if (value === undefined) delete out[key];
      else out[key] = value;
    };
    put("description", description.trim() || undefined);
    put("projects", projects.length ? [...projects] : undefined);
    put("gateway", gateway ? undefined : false);
    put("hiddenTools", patterns.length ? [...patterns] : undefined);
    if (transport === "http") {
      put("url", url.trim() || undefined);
      const h = Object.fromEntries(headers.filter((r) => r.k.trim()).map((r) => [r.k.trim(), r.v]));
      put("headers", Object.keys(h).length ? h : undefined);
      delete out.command;
      delete out.args;
      delete out.env;
    } else {
      put("command", command.trim() || undefined);
      put("args", args.length ? [...args] : undefined);
      const e = Object.fromEntries(env.filter((r) => r.k.trim()).map((r) => [r.k.trim(), r.v]));
      put("env", Object.keys(e).length ? e : undefined);
      delete out.url;
      delete out.headers;
    }
    return out;
  };

  /** Load an entry into the form; the transport falls back to what it was when neither `url` nor `command` is there. */
  const readEntry = (o: McpEntry) => {
    setDescription(str(o.description));
    if (o.url !== undefined) setTransport("http");
    else if (o.command !== undefined) setTransport("stdio");
    setUrl(str(o.url));
    setHeaders(kvOf(o.headers));
    setCommand(str(o.command));
    setArgs(strList(o.args));
    setEnv(kvOf(o.env));
    setProjects(strList(o.projects));
    setGateway(o.gateway !== false);
    setPatterns(strList(o.hiddenTools));
  };

  const switchMode = (to: "form" | "json") => {
    if (to === mode) return;
    setError(undefined);
    if (to === "json") setJson(JSON.stringify(buildEntry(), null, 2));
    else {
      try {
        const parsed: unknown = JSON.parse(json);
        if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error("нужен объект {…}");
        // A validated non-array object: exactly what an mcp.json entry is.
        readEntry(parsed as McpEntry);
      } catch (e) {
        setError(`JSON не разбирается: ${e instanceof Error ? e.message : String(e)}`);
        return;
      }
    }
    setMode(to);
  };

  const submit = async () => {
    setError(undefined);
    if (!/^[a-z0-9_-]+$/.test(id)) {
      setError("Имя подключения: строчные латинские буквы, цифры, `_` и `-`");
      return;
    }
    let entry: McpEntry;
    if (mode === "json") {
      try {
        const parsed: unknown = JSON.parse(json);
        if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error("нужен объект {…}");
        // A validated non-array object: exactly what an mcp.json entry is.
        entry = parsed as McpEntry;
      } catch (e) {
        setError(`JSON не разбирается: ${e instanceof Error ? e.message : String(e)}`);
        return;
      }
    } else {
      if (transport === "http" && !url.trim()) {
        setError("Укажите URL подключения");
        return;
      }
      if (transport === "stdio" && !command.trim()) {
        setError("Укажите команду запуска");
        return;
      }
      entry = buildEntry();
    }
    setBusy(true);
    const ok = await write(() => save(id, entry, server ? hash : undefined), "Подключение сохранено");
    setBusy(false);
    if (ok) onClose();
  };

  // Exact tool names the checkbox list manages; the chips hold the other patterns.
  const exact = patterns.filter((p) => tools.some((t) => t.name === p));
  const extra = patterns.filter((p) => !exact.includes(p));
  const setExtra = (v: string[]) => setPatterns([...exact, ...v]);
  const toggleTool = (tool: string) => {
    const matched = patterns.filter((p) => globMatch(p, tool));
    setPatterns(matched.length ? patterns.filter((p) => !matched.includes(p)) : [...patterns, tool]);
  };

  return (
    <Modal label={server ? `Подключение ${server.id}` : "Новое подключение"} onClose={onClose} wide>
      <div className="mh">
        {server ? `Изменить подключение ${server.id}` : "Новое подключение"}
        <span className="grow" />
        <div className="seg" role="group" aria-label="Как править">
          <button type="button" className={mode === "form" ? "on" : ""} aria-pressed={mode === "form"} onClick={() => switchMode("form")}>
            Форма
          </button>
          <button type="button" className={mode === "json" ? "on" : ""} aria-pressed={mode === "json"} onClick={() => switchMode("json")}>
            JSON
          </button>
        </div>
        <button type="button" className="icon-btn" onClick={onClose} aria-label="Закрыть">
          <Icon.close />
        </button>
      </div>
      <form
        className="mb ag-form"
        id="mc-edit"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        {mode === "form" ? (
          <>
            <div className="mc-grid">
              <label className="field">
                Имя
                <input className="mono" value={id} disabled={!!server} pattern="[a-z0-9_-]+" title="строчные латинские буквы, цифры, `_` и `-`" placeholder="github" required onChange={(e) => setId(e.target.value)} />
              </label>
              <label className="field">
                Описание
                <input value={description} placeholder="GitHub: issues, pull requests" onChange={(e) => setDescription(e.target.value)} />
              </label>
            </div>
            <div className="field">
              Транспорт
              <div>
                <div className="seg" role="group" aria-label="Транспорт">
                  <button type="button" className={transport === "http" ? "on" : ""} aria-pressed={transport === "http"} onClick={() => setTransport("http")}>
                    HTTP
                  </button>
                  <button type="button" className={transport === "stdio" ? "on" : ""} aria-pressed={transport === "stdio"} onClick={() => setTransport("stdio")}>
                    Команда
                  </button>
                </div>
              </div>
            </div>
            {transport === "http" ? (
              <>
                <label className="field">
                  URL
                  <input className="mono" value={url} placeholder="https://api.example.com/mcp/" onChange={(e) => setUrl(e.target.value)} />
                </label>
                <KvRows label="Заголовки" hint="значение может читать секрет: ${env:ИМЯ}" rows={headers} onChange={setHeaders} valuePlaceholder={"Bearer ${env:GITHUB_MCP_TOKEN}"} />
              </>
            ) : (
              <>
                <label className="field">
                  Команда
                  <input className="mono" value={command} placeholder="npx -y @example/mcp" onChange={(e) => setCommand(e.target.value)} />
                </label>
                <div className="field">
                  Аргументы
                  <ChipInput values={args} onChange={setArgs} mono placeholder="аргумент" />
                </div>
                <KvRows label="Переменные окружения" hint="значение может читать секрет: ${env:ИМЯ}" rows={env} onChange={setEnv} valuePlaceholder={"${env:TOKEN}"} />
              </>
            )}
            <div className="field">
              Проекты
              <ChipInput id="mc-projects" values={projects} onChange={setProjects} suggestions={allProjects} placeholder="Пусто — во всех проектах" />
            </div>
            <label className="pj-check">
              <input type="checkbox" checked={gateway} onChange={(e) => setGateway(e.target.checked)} />
              Через шлюз genie (секреты на сервере, вызовы в журнале)
            </label>
            {!gateway && <p className="ag-warn">Без шлюза харнесс получает подключение вместе с секретами, вызовы genie не видны.</p>}
            <div className="field">
              Скрытые инструменты
              {tools.length > 0 ? (
                <>
                  <div className="ag-checks mc-tool-list">
                    {tools.map((t) => {
                      const hidden = patterns.some((p) => globMatch(p, t.name));
                      return (
                        <label key={t.name} title={t.description}>
                          <input type="checkbox" checked={hidden} onChange={() => toggleTool(t.name)} />
                          <span className="mono">{t.name}</span>
                          <span className="muted">{t.description}</span>
                        </label>
                      );
                    })}
                  </div>
                  <div className="field">
                    <span>
                      Шаблоны<span className="pj-hint"> * — любой текст, ? — один символ; скрытое агентам не достаётся</span>
                    </span>
                    <ChipInput values={extra} onChange={setExtra} mono placeholder="get_*" />
                  </div>
                </>
              ) : (
                <ChipInput values={patterns} onChange={setPatterns} mono placeholder="шаблоны, например get_*" />
              )}
            </div>
          </>
        ) : (
          <label className="field">
            Подключение, как оно встанет в <span className="mono">mcp.json</span>
            <textarea className="mono mc-json" rows={16} spellCheck={false} value={json} onChange={(e) => setJson(e.target.value)} />
          </label>
        )}
        {error && (
          <p className="mc-error" role="alert">
            {error}
          </p>
        )}
      </form>
      <div className="mf">
        <span className="grow" />
        <button type="button" className="btn ghost" onClick={onClose}>
          Отмена
        </button>
        <button type="submit" form="mc-edit" className="btn primary" disabled={busy}>
          Сохранить
        </button>
      </div>
    </Modal>
  );
}

/** Key → value rows of the editor: headers or environment variables, with a way to add and remove. */
function KvRows({ label, hint, rows, onChange, valuePlaceholder }: { label: string; hint?: string; rows: Kv[]; onChange: (rows: Kv[]) => void; valuePlaceholder?: string }) {
  return (
    <div className="field">
      <span>
        {label}
        {hint && <span className="pj-hint"> {hint}</span>}
      </span>
      {rows.map((r, i) => (
        <div key={i} className="mc-kv">
          <input className="mono" value={r.k} placeholder="имя" aria-label={`${label}: имя`} onChange={(e) => onChange(rows.map((x, j) => (j === i ? { ...x, k: e.target.value } : x)))} />
          <input className="mono" value={r.v} placeholder={valuePlaceholder ?? "значение"} aria-label={`${label}: значение`} onChange={(e) => onChange(rows.map((x, j) => (j === i ? { ...x, v: e.target.value } : x)))} />
          <button type="button" className="icon-btn" aria-label={`Убрать ${r.k || "строку"}`} onClick={() => onChange(rows.filter((_, j) => j !== i))}>
            <Icon.trash size={13} />
          </button>
        </div>
      ))}
      <div>
        <button type="button" className="btn ghost" onClick={() => onChange([...rows, { k: "", v: "" }])}>
          <Icon.plus size={13} />
          Добавить
        </button>
      </div>
    </div>
  );
}

/** The environment variables MCP entries read: values live in the server's database, never in mcp.json. */
function Secrets({ d, onEdit }: { d: McpAdmin; onEdit: (id: string | null) => void }) {
  const write = useMcpWrite();
  const qc = useQueryClient();
  const [setting, setSetting] = useState<string>();
  const [creating, setCreating] = useState(false);
  const [confirming, setConfirming] = useState<McpSecretInfo>();
  const users = (name: string) => d.servers.filter((s) => s.envRefs.includes(name));
  return (
    <Section
      title="Переменные окружения"
      note="Секреты, на которые подключения ссылаются как ${env:ИМЯ}; значения стоят в базе сервера"
      aside={
        <button type="button" className="btn" onClick={() => setCreating(true)}>
          <Icon.plus size={13} />
          Новая переменная
        </button>
      }
    >
      <div className="ag-table-wrap">
        <table className="ag-table">
          <thead>
            <tr>
              <th>Переменная</th>
              <th>Значение</th>
              <th>Используют</th>
              <th aria-label="Действия" />
            </tr>
          </thead>
          <tbody>
            {d.secrets.map((s) => (
              <tr key={s.name}>
                <td className="mono">{s.name}</td>
                <td className="muted" title={s.updated ? `обновлено ${timeAgo(s.updated)}` : undefined}>
                  {s.unreadable ? "значение показать нельзя" : s.hint || "есть"}
                </td>
                <td>
                  <div className="ag-how">
                    {s.usedBy.map((id) => (
                      <button key={id} type="button" className="mc-server-chip mono" title="Открыть подключение в редакторе" onClick={() => onEdit(id)}>
                        {id}
                      </button>
                    ))}
                    {!s.usedBy.length && <span className="muted">—</span>}
                  </div>
                </td>
                <td className="ag-cell-action">
                  <div className="ag-how mc-actions">
                    <button type="button" className="btn" onClick={() => setSetting(s.name)}>
                      Задать значение
                    </button>
                    <button type="button" className="btn ghost" onClick={() => setConfirming(s)}>
                      Удалить
                    </button>
                  </div>
                </td>
              </tr>
            ))}
            {d.secretsUnset.map((name) => (
              <tr key={name} className="mc-unset">
                <td className="mono">{name}</td>
                <td className="muted">значения нет</td>
                <td>
                  <div className="ag-how">
                    {users(name).map((s) => (
                      <button key={s.id} type="button" className="mc-server-chip mono" title="Открыть подключение в редакторе" onClick={() => onEdit(s.id)}>
                        {s.id}
                      </button>
                    ))}
                  </div>
                </td>
                <td className="ag-cell-action">
                  <button type="button" className="btn" onClick={() => setSetting(name)}>
                    Задать значение
                  </button>
                </td>
              </tr>
            ))}
            {!d.secrets.length && !d.secretsUnset.length && (
              <tr>
                <td colSpan={4} className="muted">
                  Подключения пока не читают переменных: секреты пишите в mcp.json только как {"${env:ИМЯ}"}.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
      {setting && <SecretForm name={setting} onDone={() => setSetting(undefined)} onClose={() => setSetting(undefined)} />}
      {creating && <SecretForm onDone={() => setCreating(false)} onClose={() => setCreating(false)} />}
      {confirming && (
        <ConfirmDialog
          title={`Удалить ${confirming.name}?`}
          confirmLabel="Удалить"
          danger
          onClose={() => setConfirming(undefined)}
          onConfirm={() => {
            const s = confirming;
            setConfirming(undefined);
            void write(
              async () => {
                await deleteMcpSecret(s.name);
                await qc.invalidateQueries({ queryKey: agentKeys.mcpAdmin });
              },
              "Переменная удалена",
            );
          }}
        >
          Значение стирается с сервера. Подключения, читающие переменную, останутся без него, пока его не зададут снова.
        </ConfirmDialog>
      )}
    </Section>
  );
}

/** Give a variable (a new one when `name` is absent) a value; a blank value removes it. */
function SecretForm({ name, onDone, onClose }: { name?: string; onDone: () => void; onClose: () => void }) {
  const [value, setValue] = useState("");
  const [newName, setNewName] = useState("");
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const write = useMcpWrite();
  const qc = useQueryClient();
  const submit = async () => {
    setError(undefined);
    if (!name && !/^[A-Za-z_][A-Za-z0-9_]*$/.test(newName)) {
      setError("Имя переменной: латинские буквы, цифры и `_`, не с цифры");
      return;
    }
    setBusy(true);
    const ok = await write(
      async () => {
        await setMcpSecret(name ?? newName, value);
        await qc.invalidateQueries({ queryKey: agentKeys.mcpAdmin });
      },
      value.trim() ? "Значение сохранено" : "Переменная удалена",
    );
    setBusy(false);
    if (ok) onDone();
  };
  return (
    <Modal label={name ? `Значение ${name}` : "Новая переменная"} onClose={onClose}>
      <div className="mh">{name ? <span className="mono">{name}</span> : "Новая переменная"}</div>
      <form
        className="mb ag-form"
        id="mc-secret"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        {!name && (
          <label className="field">
            Имя
            <input className="mono" value={newName} placeholder="GITHUB_MCP_TOKEN" required onChange={(e) => setNewName(e.target.value)} />
          </label>
        )}
        <label className="field">
          Значение
          <input type="password" autoComplete="new-password" value={value} onChange={(e) => setValue(e.target.value)} />
          <span className="pj-hint">{value.trim() ? "Значение уйдёт в базу сервера и назад не прочитается." : "Пустое значение удаляет переменную."}</span>
        </label>
        {error && (
          <p className="mc-error" role="alert">
            {error}
          </p>
        )}
      </form>
      <div className="mf">
        <span className="grow" />
        <button type="button" className="btn ghost" onClick={onClose}>
          Отмена
        </button>
        <button type="submit" form="mc-secret" className="btn primary" disabled={busy}>
          Сохранить
        </button>
      </div>
    </Modal>
  );
}

interface UsageTotals {
  calls: number;
  errors: number;
  ms: number;
  bytesIn: number;
  bytesOut: number;
}

const addRow = (t: UsageTotals, r: McpUsageRow) => {
  t.calls += r.calls;
  t.errors += r.errors;
  t.ms += r.ms;
  t.bytesIn += r.bytesIn;
  t.bytesOut += r.bytesOut;
};

/** One connection of the usage report: its sums and its tools', biggest first. */
function usageServer(id: string, perTool: Map<string, UsageTotals>) {
  const totals: UsageTotals = { calls: 0, errors: 0, ms: 0, bytesIn: 0, bytesOut: 0 };
  for (const t of perTool.values()) {
    totals.calls += t.calls;
    totals.errors += t.errors;
    totals.ms += t.ms;
    totals.bytesIn += t.bytesIn;
    totals.bytesOut += t.bytesOut;
  }
  return { id, totals, tools: [...perTool.entries()].sort((a, b) => b[1].calls - a[1].calls) };
}

/** The calls through the gateway: per connection, with the per-tool drill-down. */
function Usage() {
  const [days, setDays] = useState(30);
  const usage = useMcpUsage(days);
  const [open, setOpen] = useState<string>();
  const rows = usage.data?.rows ?? [];
  const perServer = new Map<string, Map<string, UsageTotals>>();
  for (const r of rows) {
    const perTool = perServer.get(r.server) ?? new Map<string, UsageTotals>();
    const t = perTool.get(r.tool) ?? { calls: 0, errors: 0, ms: 0, bytesIn: 0, bytesOut: 0 };
    addRow(t, r);
    perTool.set(r.tool, t);
    perServer.set(r.server, perTool);
  }
  const list = [...perServer.entries()].map(([id, perTool]) => usageServer(id, perTool)).sort((a, b) => b.totals.calls - a.totals.calls);
  return (
    <Section
      title="Статистика"
      note="Вызовы инструментов через шлюз: сколько, с ошибками ли и какой это двинуло трафик"
      aside={
        <div className="seg" role="group" aria-label="Период">
          {[7, 30, 90].map((v) => (
            <button key={v} type="button" className={v === days ? "on" : ""} aria-pressed={v === days} onClick={() => setDays(v)}>
              {v === 7 ? "Неделя" : v === 30 ? "Месяц" : "90 дней"}
            </button>
          ))}
        </div>
      }
    >
      {usage.isPending ? (
        <p className="muted ag-empty">Загрузка…</p>
      ) : usage.isError ? (
        <p className="muted ag-empty">{usage.error.message}</p>
      ) : !list.length ? (
        <p className="muted ag-empty">Вызовов через шлюз пока не было</p>
      ) : (
        <div className="ag-table-wrap">
          <table className="ag-table mc-usage">
            <thead>
              <tr>
                <th>Подключение</th>
                <th>Вызовы</th>
                <th>Ошибки</th>
                <th>Среднее, мс</th>
                <th>Принято</th>
                <th>Отправлено</th>
              </tr>
            </thead>
            <tbody>
              {list.map((s) => (
                <Fragment key={s.id}>
                  <tr>
                    <td>
                      <button type="button" className="mc-expand" aria-expanded={open === s.id} onClick={() => setOpen(open === s.id ? undefined : s.id)}>
                        <Icon.chevron size={12} />
                        <span className="mono">{s.id}</span>
                        <span className="muted">
                          {s.tools.length} {plural(s.tools.length, "инструмент", "инструмента", "инструментов")}
                        </span>
                      </button>
                    </td>
                    <UsageCells t={s.totals} />
                  </tr>
                  {open === s.id &&
                    s.tools.map(([tool, t]) => (
                      <tr key={tool} className="mc-tool-row">
                        <td className="mono">{tool}</td>
                        <UsageCells t={t} />
                      </tr>
                    ))}
                </Fragment>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Section>
  );
}

/** The number cells of one usage row: calls, errors, average milliseconds and the traffic both ways. */
function UsageCells({ t }: { t: UsageTotals }) {
  return (
    <>
      <td>{t.calls}</td>
      <td className={t.errors ? "mc-errors" : undefined}>{t.errors}</td>
      <td>{t.calls ? Math.round(t.ms / t.calls) : 0}</td>
      <td>{bytes(t.bytesIn)}</td>
      <td>{bytes(t.bytesOut)}</td>
    </>
  );
}
