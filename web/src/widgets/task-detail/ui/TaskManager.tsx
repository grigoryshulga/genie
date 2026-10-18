import { useState } from "react";
import { useManageTask, useTaskManagement, type Task } from "@/entities/task";
import { canWriteProject, useSession } from "@/entities/session";
import { Markdown, useToast } from "@/shared/ui";
import { CollapsibleSection } from "./CollapsibleSection.tsx";
import "./task-manager.css";

const STATES: Record<string, string> = {
  active: "Ведёт выполнение",
  retrospective: "Результат закрыт · ретро ожидается",
  completed: "Ретро опубликовано",
};

/** The manager outlives the result: unresolved questions and retrospective stay visible. */
export function TaskManager({ task }: { task: Task }) {
  const query = useTaskManagement(task.id);
  const action = useManageTask();
  const toast = useToast();
  const [answers, setAnswers] = useState<Record<number, string>>({});
  const writable = canWriteProject(useSession().data);
  if (query.isPending) return null;
  if (query.isError) return <section className="sec"><h3>Менеджер задачи</h3><p role="alert">{query.error.message}</p></section>;
  const manager = query.data.manager;
  const act = (name: "dispatch" | "retry" | "answer" | "escalate", extra: { id?: number; text?: string; source?: string } = {}) => {
    action.mutate({ task: task.id, action: name, ...extra }, {
      onError: (e) => toast(e.message, "error"),
      onSuccess: () => toast(name === "dispatch" ? "Задача передана менеджеру" : name === "answer" ? "Ответ передан менеджеру задачи" : name === "escalate" ? "Запрошено решение человека" : "Менеджер продолжит работу"),
    });
  };
  if (!manager) {
    if (task.status !== "ready" || !writable) return null;
    return <CollapsibleSection title="Менеджер задачи"><button type="button" className="btn" disabled={action.isPending} onClick={() => act("dispatch")}>Отправить в выполнение</button></CollapsibleSection>;
  }
  const questions = query.data.clarifications.filter((q) => q.state === "project" || q.state === "owner");
  const report = manager.report ? task.artifacts.find((a) => a.id === manager.report) : undefined;
  return (
    <CollapsibleSection title="Менеджер задачи">
      <p>{STATES[manager.state] ?? manager.state} <span className="muted">· постановка v{manager.briefVersion}</span></p>
      {report && <p>Отчёт: {report.name} · #{report.id} — доступен в артефактах задачи.</p>}
      {manager.state !== "completed" && writable && <button type="button" className="btn ghost" disabled={action.isPending} onClick={() => act("retry")}>Продолжить работу менеджера</button>}
      {questions.map((q) => (
        <div key={q.id} className="manager-question">
          <p><b>Уточнение #{q.id}</b> · {q.state === "owner" ? "ожидает человека" : "у менеджера проекта"}{q.blocking ? " · блокирует приёмку" : " · работа может продолжаться"}</p>
          <Markdown text={q.question} />
          {writable && <form onSubmit={(e) => { e.preventDefault(); act("answer", { id: q.id, text: answers[q.id] }); }}>
            <label>Ответ<textarea className="in" required value={answers[q.id] ?? ""} onChange={(e) => setAnswers((a) => ({ ...a, [q.id]: e.target.value }))} /></label>
            <button type="submit" className="btn" disabled={action.isPending || !answers[q.id]?.trim()}>Ответить</button>
            {q.state === "project" && <button type="button" className="btn ghost" disabled={action.isPending} onClick={() => act("escalate", { id: q.id })}>Запросить решение человека</button>}
          </form>}
        </div>
      ))}
    </CollapsibleSection>
  );
}
