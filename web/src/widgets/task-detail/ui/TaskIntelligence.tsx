import type { Snapshot } from "@/shared/api/generated/Snapshot.ts";
import { useSession } from "@/entities/session";
import { useToast } from "@/shared/ui";
import { useState } from "react";
import { useAssessTaskSignals, useResolveTaskSignals, useTaskIntelligence } from "@/entities/task";

/** Lazy task evidence, using project/task-scoped API and generated Rust types. */
export function TaskIntelligence({ task }: { task: string }) {
  const [open, setOpen] = useState(false);
  const history = useTaskIntelligence(task, open);
  return <details className="sec" open={open} onToggle={(e) => setOpen(e.currentTarget.open)}>
    <summary>Оценка задачи и история маршрутизации</summary>
    {history.isError ? <p role="alert">{history.error.message}</p> : !history.data ? <p className="muted">Загрузка…</p> : <>
      <SignalActions task={task} latest={history.data.snapshots.at(-1)} />
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

function SignalActions({ task, latest }: { task: string; latest?: Snapshot }) {
  const session = useSession().data;
  const role = session?.projects.find((p) => p.slug === session.project)?.role;
  const writable = session?.mode === "local" || (role !== undefined && role !== "viewer");
  const assess = useAssessTaskSignals();
  const resolve = useResolveTaskSignals();
  const toast = useToast();
  const [paths, setPaths] = useState("");
  const [reason, setReason] = useState("");
  const [evidence, setEvidence] = useState("");
  if (!writable) return null;
  return <div>
    <form onSubmit={(e) => { e.preventDefault(); assess.mutate({ id: task, body: { affectedPaths: paths.split(",").map((p) => p.trim()).filter(Boolean) } }, { onError: (e) => toast(e.message, "error"), onSuccess: () => toast("Оценка задачи сохранена") }); }}>
      <label>Подтверждённые пути изменения<input className="st-input" value={paths} maxLength={4000} placeholder="src/auth/login.rs, src/api/orders.rs" onChange={(e) => setPaths(e.target.value)} /></label>
      <p className="muted">Пути от корня репозитория, через запятую. Пустое поле сохраняет неизвестную область и все защитные проверки.</p>
      <button className="btn ghost sm" type="submit" disabled={assess.isPending}>Зафиксировать область и оценить</button>
    </form>
    {latest?.profile.signals.some((s) => s.source === "runtime") && <form onSubmit={(e) => { e.preventDefault(); resolve.mutate({ id: task, body: { snapshotId: latest.id, reason, evidenceRefs: evidence.split(",").map((v) => v.trim()).filter(Boolean) } }, { onError: (e) => toast(e.message, "error"), onSuccess: () => { setReason(""); setEvidence(""); toast("Решение зафиксировано; команду можно возобновить отдельно"); } }); }}>
      <h4>Решение по обнаруженным ограничениям</h4>
      <label>Что проверено и решено<textarea className="st-input" required minLength={12} maxLength={2000} value={reason} onChange={(e) => setReason(e.target.value)} /></label>
      <label>Ссылки на подтверждения, через запятую<input className="st-input" required value={evidence} maxLength={4000} onChange={(e) => setEvidence(e.target.value)} /></label>
      <button className="btn ghost sm" type="submit" disabled={resolve.isPending}>Зафиксировать решение человека</button>
    </form>}
  </div>;
}
