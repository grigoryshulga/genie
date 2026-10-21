import { Fragment, useState } from "react";
import "./spawn.css";
import { useNavigate } from "react-router";
import { MAIL_TITLE, STAGE_TITLE, TemplateGraph, templatesFor, useAgentConfig, WORKSPACE_TITLE } from "@/entities/agent-config";
import { useMeta } from "@/entities/project";
import { STATUS_NAME, type Status } from "@/entities/task";
import { keysFor, request, useInvalidating } from "@/shared/api";
import { plural } from "@/shared/lib";
import { Icon, Modal, useToast } from "@/shared/ui";
import { Button, Field, IconButton, Input, Segmented, Select, Textarea } from "@/shared/ui/kit";

/**
 * Assemble a team for a task: a template (its members and relations; models can
 * be changed for this team) or roles picked one by one, and a note for the kickoff.
 */
export function SpawnTeamDialog({ task, onClose }: { task: { id: string; title: string; status: string }; onClose: () => void }) {
  const cfg = useAgentConfig().data;
  const meta = useMeta().data;
  const toast = useToast();
  const navigate = useNavigate();
  const early = ["inbox", "draft", "refining"].includes(task.status);
  const templates = cfg ? templatesFor(cfg.teams, task.status) : [];
  const [mode, setMode] = useState<"template" | "roles">("template");
  const [picked, setPicked] = useState<string>();
  const [models, setModels] = useState<Record<string, string>>({});
  const [members, setMembers] = useState<{ role: string; model: string }[]>([]);
  const [note, setNote] = useState("");
  const spawn = useInvalidating(
    (body: unknown) => request<{ id: string }>("POST", "/api/teams", body),
    // Spawning a team appends `team.spawned`; it also puts the team on the task's page.
    keysFor({ type: "team.spawned" }),
  );
  const template = templates.find((t) => t.id === (picked ?? templates[0]?.id));
  const roles = (cfg?.roles ?? []).filter((r) => r.class !== "orchestrator" && r.stages.includes(early ? "refinement" : "delivery"));
  const role = (id: string) => cfg?.roles.find((r) => r.id === id);
  const defaultModel = (id: string) => meta?.roleModels?.[id]?.model ?? role(id)?.model;

  const submit = () => {
    const body =
      mode === "template"
        ? { task: task.id, template: template?.id, models: Object.fromEntries(Object.entries(models).filter(([, m]) => m.trim())), note: note.trim() || undefined }
        : { task: task.id, members: members.map((m) => ({ role: m.role, model: m.model.trim() || undefined })), note: note.trim() || undefined };
    spawn.mutate(body, {
      onSuccess: (team) => {
        toast(`Команда ${team.id} собрана и получила задачу`);
        onClose();
        navigate(`/team/${encodeURIComponent(team.id)}`);
      },
      onError: (e) => toast(`Не собрана: ${e.message}`, "error"),
    });
  };
  const ready = mode === "template" ? !!template : members.length > 0;
  const dot = (id: string) => (
    <span className={`ag-dot c-${role(id)?.class ?? "executor"}`} aria-hidden="true">
      {(role(id)?.title ?? id).slice(0, 1).toUpperCase()}
    </span>
  );
  const noteField = (
    <Field label="Заметка для команды — попадёт в kickoff">
      <Textarea rows={2} value={note} onChange={(e) => setNote(e.target.value)} placeholder="Например: сначала покажите план" />
    </Field>
  );

  return (
    <Modal label="Собрать команду" onClose={onClose} wide>
      <div className="mh spawn-head">
        <h2>Собрать команду</h2>
        <span className="g-caption">
          <span className="g-mono">{task.id}</span> · {(STATUS_NAME[task.status as Status] ?? task.status).toLowerCase()}
        </span>
        <span className="grow" />
        <Segmented
          label="Как собрать"
          value={mode}
          onChange={setMode}
          items={[
            { id: "template", label: "По шаблону" },
            { id: "roles", label: "Из ролей" },
          ]}
        />
        <IconButton
          label="Закрыть"
          onClick={onClose}
        >
          <Icon.close />
        </IconButton>
      </div>
      {!cfg ? (
        <div className="mb">
          <p className="g-caption">Загрузка…</p>
        </div>
      ) : mode === "template" ? (
        <div className="spawn-two">
          <div className="spawn-templates" role="radiogroup" aria-label="Шаблон">
            <span className="g-caption">{early ? "Шаблоны разбора: задача ещё не готова к работе" : "Шаблоны для задачи, готовой к работе"}</span>
            {templates.map((t) => (
              <button type="button" role="radio" aria-checked={t.id === template?.id} key={t.id} className={`spawn-template${t.id === template?.id ? " on" : ""}`} onClick={() => setPicked(t.id)}>
                <span className="t">
                  <span className="ag-dots">{t.members.slice(0, 5).map((m) => <Fragment key={m.key}>{dot(m.role)}</Fragment>)}</span>
                  {t.title}
                </span>
                <span className="s">{t.members.map((m) => role(m.role)?.title ?? m.role).join(", ")}</span>
              </button>
            ))}
            {!templates.length && <p className="g-caption">Подходящих шаблонов нет — соберите команду из ролей.</p>}
          </div>
          <div className="spawn-main">
            {template && (
              <>
                <div className="spawn-title">
                  <b>{template.title}</b>
                  <span className="g-caption">
                    {STAGE_TITLE[template.stage]} · {WORKSPACE_TITLE[template.workspace]} · почта: {MAIL_TITLE[template.mail]} · {template.members.length}{" "}
                    {plural(template.members.length, "участник", "участника", "участников")}
                  </span>
                  {template.description && <span className="g-caption">{template.description}</span>}
                </div>
                <TemplateGraph members={template.members} relations={template.relations} roles={cfg.roles} />
                <div className="spawn-models">
                  <span className="g-caption">Модели участников</span>
                  {template.members.map((m) => (
                    <label key={m.key} className="spawn-model">
                      {dot(m.role)}
                      <span>{m.name ? `${m.name[0].toUpperCase()}${m.name.slice(1)} — ${(role(m.role)?.title ?? m.role).toLowerCase()}` : (role(m.role)?.title ?? m.role)}</span>
                      <Input
                        value={models[m.key] ?? ""}
                        onChange={(e) => setModels((x) => ({ ...x, [m.key]: e.target.value }))}
                        placeholder={m.model ?? defaultModel(m.role) ?? "как у роли"}
                        aria-label={`Модель: ${m.key}`}
                      />
                    </label>
                  ))}
                </div>
              </>
            )}
            {noteField}
          </div>
        </div>
      ) : (
        <div className="mb spawn-team">
          <p className="g-caption spawn-note">Связи между участниками выводятся из их классов: исполнитель сдаёт ревьюеру, ревьюер возвращает на доработку.</p>
          {members.map((m, i) => (
            <div key={i} className="spawn-member">
              {dot(m.role)}
              <Select value={m.role} onChange={(e) => setMembers((x) => x.map((y, j) => (j === i ? { ...y, role: e.target.value } : y)))} aria-label="Роль">
                {roles.map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.title}
                  </option>
                ))}
              </Select>
              <Input
                value={m.model}
                onChange={(e) => setMembers((x) => x.map((y, j) => (j === i ? { ...y, model: e.target.value } : y)))}
                placeholder={defaultModel(m.role) ?? "модель как у роли"}
                aria-label="Модель"
              />
              <IconButton label="Убрать" onClick={() => setMembers((x) => x.filter((_, j) => j !== i))}>
                <Icon.trash size={13} />
              </IconButton>
            </div>
          ))}
          <Button className="spawn-add" disabled={!roles.length} onClick={() => setMembers((x) => [...x, { role: roles[0].id, model: "" }])}>
            <Icon.plus size={13} />
            Добавить роль
          </Button>
          {noteField}
        </div>
      )}
      <div className="mf">
        Каждый участник получит kickoff, оркестратор — сообщение
        <span className="grow" />
        <Button variant="ghost" onClick={onClose}>
          Отмена
        </Button>
        <Button variant="primary" disabled={!ready} busy={spawn.isPending} onClick={submit}>
          Собрать команду
        </Button>
      </div>
    </Modal>
  );
}
