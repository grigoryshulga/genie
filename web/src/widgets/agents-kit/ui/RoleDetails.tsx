import type { ReactNode } from "react";
import { FILES_TITLE, STAGE_TITLE, type Catalogue, type RoleDef } from "@/entities/agent-config";
import { Badge, Markdown } from "@/shared/ui";
import { PermissionFields } from "./PermissionFields.tsx";

function Card({ title, children }: { title: string; children: ReactNode }) {
  return <section className="role-section"><h3>{title}</h3>{children}</section>;
}

export function RolePrompt({ role }: { role: RoleDef }) {
  return <section className="role-prompt" aria-label="Промпт роли">
    {role.prompt ? <Markdown text={role.prompt} /> : <p className="g-muted">Промпт не задан.</p>}
    {role.instructions && <div className="role-prompt-extra"><h4>Дополнительные инструкции</h4><Markdown text={role.instructions} /></div>}
  </section>;
}

export function RoleDetails({ role: r, cfg }: { role: RoleDef; cfg: Catalogue }) {
  return <div className="role-settings-view">
    <Card title="Доступ к проекту"><dl className="role-values">
      <dt>Файлы</dt><dd>{FILES_TITLE[r.files]}</dd>
      <dt>Запрещённые команды</dt><dd>{r.denyCommands.length ? <code>{r.denyCommands.join(", ")}</code> : "Нет запретов"}</dd>
      <dt>Изоляция</dt><dd title={cfg.sandbox?.note}>{cfg.sandbox?.active ? "Песочница" : "Без песочницы"}</dd>
    </dl></Card>
    <Card title="Участие в работе"><dl className="role-values">
      <dt>Стадии</dt><dd>{r.stages.map((s) => STAGE_TITLE[s]).join(", ") || "Не участвует"}</dd>
      <dt>Проекты</dt><dd>{r.projects?.join(", ") || "Все проекты"}</dd>
      <dt>Имена участников</dt><dd>{r.names.join(", ") || "По классу роли"}</dd>
    </dl></Card>
    <Card title="Разрешённые действия">{r.class === "orchestrator" ? <p className="g-muted">Права оркестратора фиксированы системой.</p> : <PermissionFields wanted={r.capabilities} />}</Card>
    <SkillsCard role={r} cfg={cfg} />
    <McpCard role={r} cfg={cfg} />
  </div>;
}

export function SkillsCard({ role: r, cfg }: { role: RoleDef; cfg: Catalogue }) {
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
      {!r.skills?.length && <p className="g-muted ag-empty">{r.skills === undefined ? "Все навыки" : "Навыки репозитория"}</p>}
      {missing.length > 0 && <p className="ag-warn">Не установлены: {missing.join(", ")} — агент их не получит.</p>}
    </Card>
  );
}

/** Keep long tool lists behind one disclosure per connection. Unknown grants stay visible. */
export function McpCard({ role: r, cfg }: { role: RoleDef; cfg: Catalogue }) {
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
