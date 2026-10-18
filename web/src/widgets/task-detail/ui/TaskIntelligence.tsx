import { useState } from "react";
import { useTaskIntelligence } from "@/entities/task";

/** Lazy task evidence, using project/task-scoped API and generated Rust types. */
export function TaskIntelligence({ task }: { task: string }) {
  const [open, setOpen] = useState(false);
  const history = useTaskIntelligence(task, open);
  return <details className="sec" open={open} onToggle={(e) => setOpen(e.currentTarget.open)}>
    <summary>Оценка задачи и история маршрутизации</summary>
    {history.isError ? <p role="alert">{history.error.message}</p> : !history.data ? <p className="muted">Загрузка…</p> : <>
      {!history.data.snapshots.length && <p className="muted">Для этой задачи сигналы не записаны. Исторические оценки не восстанавливаются предположениями.</p>}
      {history.data.snapshots.map((s) => <details key={s.id}><summary>Brief v{s.profile.briefVersion || "?"} → S-{s.id} · ревизия {s.revision} · {new Date(s.createdAt).toLocaleString("ru-RU")}</summary>
        <p className="muted">Профиль {s.profile.projectProfileVersion}; классификатор {s.profile.extractorVersion}. Уверенность в классификации не является вероятностью успешного выполнения.</p>
        <ul>{s.profile.signals.map((signal) => <li key={signal.name}><b>{signal.name}: {signal.value}</b> · {signal.source} · {signal.assessorVersion}<br /><span className="muted">{signal.evidenceRefs.join(", ") || "Нет подтверждённых сведений"}</span></li>)}</ul>
        {history.data.routing.filter((r) => r.snapshotId === s.id).map((r) => <div key={r.id}>
          <b>→ R-{r.id}: {r.plan.selectedTemplate ?? "custom"} · {r.plan.decisionStatus}</b>
          <p>Попытка {r.attemptId ?? "не начата"}; {r.plan.routerProvider} / {r.plan.routerVersion}</p>
          <p>Проверки: {r.plan.requiredChecks.join(", ") || "Не заданы"}. Причины: {r.plan.reasonCodes.join(", ")}</p>
          {r.plan.overrideReason && <p>Обоснование: {r.plan.overrideReason}</p>}
          <ul>{r.plan.selectedModels.map((m, i) => <li key={i}>{m.role}: {m.model ?? "неизвестно"} / {m.thinking ?? "по умолчанию"}</li>)}</ul>
        </div>)}
      </details>)}
      {history.data.attempts.map((a) => <p key={a.id}>Попытка {a.ordinal} (#{a.id}) · {new Date(a.startedAt).toLocaleString("ru-RU")} → {history.data.assessments.filter((r) => r.attemptId === a.id).map((r) => r.result).join(", ") || "Результат ещё не зафиксирован"}</p>)}
      {history.data.evaluations.map((evaluation) => <details key={evaluation.id}><summary>Shadow evaluation #{evaluation.id} · {new Date(evaluation.createdAt).toLocaleString("ru-RU")}</summary><p>Предложение для проверки человеком. Политика выполнения не меняется.</p><p>{recommendation(evaluation.evidence)}</p></details>)}
    </>}
  </details>;
}

function recommendation(evidence: unknown): string {
  if (typeof evidence === "object" && evidence !== null && "recommendation" in evidence && typeof evidence.recommendation === "string") return evidence.recommendation;
  return "Подробные сведения доступны в истории оценки.";
}
