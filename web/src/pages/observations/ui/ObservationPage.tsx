// «Наблюдение» — one observation case (docs/platform/observation-plane.md §14, §17): its facts,
// the actions the caller may take (caseActions, the rules the server enforces too), the contract,
// deployments, evaluations, evidence, incidents, and the follow-up tasks.

import { type FormEvent, type ReactNode, useState } from "react";
import { Link, useParams } from "react-router";
import {
  type CaseAction,
  type CaseRights,
  type Contract,
  type ObservationCase,
  type ObservationPolicy,
  caseActions,
  contractTemplate,
  criterionChange,
  formatDelta,
  INCIDENT_LABEL,
  incidentActions,
  POLICY_LABEL,
  RESULT_LABEL,
  STATUS_LABEL,
  useArmCase,
  useCancelCase,
  useDismissIncident,
  useEvaluateCase,
  useFileFollowup,
  useObservationCase,
  useRecordDeployment,
  useReportEvidence,
  useResolveIncident,
  useSetContract,
  useStartCase,
  useUpdateCase,
} from "@/entities/observation";
import { useSession } from "@/entities/session";
import type { CaseView } from "@/shared/api/generated/CaseView.ts";
import type { Confidence } from "@/shared/api/generated/Confidence.ts";
import type { CriterionState } from "@/shared/api/generated/CriterionState.ts";
import type { EvalTrigger } from "@/shared/api/generated/EvalTrigger.ts";
import type { EvidencePhase } from "@/shared/api/generated/EvidencePhase.ts";
import type { Incident } from "@/shared/api/generated/Incident.ts";
import type { TrustLevel } from "@/shared/api/generated/TrustLevel.ts";
import type { UpdateCaseBody } from "@/shared/api/generated/UpdateCaseBody.ts";
import type { Verdict } from "@/shared/api/generated/Verdict.ts";
import { ConfirmDialog, Icon, Modal, useToast } from "@/shared/ui";
import { STATUS_DOT, TaskLinks, when } from "./ObservationsPage.tsx";
import "../observations.css";

// Labels the glossary does not name yet: the same Russian register as the glossary's rows.
const VERDICT_LABEL: Record<Verdict, string> = {
  confirmed: RESULT_LABEL.confirmed,
  negative: RESULT_LABEL.negative,
  inconclusive: RESULT_LABEL.inconclusive,
  insufficient: "Недостаточно данных",
};
const TRIGGER_LABEL: Record<EvalTrigger, string> = { window: "по расписанию", manual: "по запросу", final: "итоговая" };
const CONFIDENCE_LABEL: Record<Confidence, string> = { low: "низкая", medium: "средняя", high: "высокая" };
const CRITERION_LABEL: Record<CriterionState, string> = {
  met: "достигнуто",
  not_met: "не достигнуто",
  ok: "в норме",
  violated: "нарушено",
  insufficient: "недостаточно данных",
};
const PHASE_LABEL: Record<EvidencePhase, string> = { baseline: "база", observed: "наблюдение" };
const TRUST_LABEL: Record<TrustLevel, string> = { measured: "измерено", reported: "сообщено", agent: "агентом" };
const POLICIES: ObservationPolicy[] = ["guardrails", "verify"];
const PHASES: EvidencePhase[] = ["observed", "baseline"];

/** The case number of `O-7` or `7`; undefined for anything else. */
function parseCaseId(raw: string | undefined): number | undefined {
  const match = /^(?:O-)?(\d+)$/i.exec(raw ?? "");
  return match ? Number(match[1]) : undefined;
}

/**
 * What the caller may do here. The web talks for a person: a signed-in user, or the operator of a
 * local server. Viewers may read; everyone else with a project role may write.
 */
function useCaseRights(): CaseRights {
  const session = useSession().data;
  const role = session?.projects.find((p) => p.slug === session.project)?.role;
  return {
    writable: session?.mode === "local" || (role !== undefined && role !== "viewer"),
    human: true,
  };
}

export function ObservationPage() {
  const id = parseCaseId(useParams().caseId);
  const rights = useCaseRights();
  const view = useObservationCase(id);
  if (id === undefined) return <Frame><div className="empty">Такого наблюдения нет</div></Frame>;
  if (view.isSuccess) return <CaseBody key={id} view={view.data} rights={rights} />;
  if (view.isError) {
    return (
      <Frame>
        <p className="obs-error" role="alert">Не удалось открыть наблюдение: {view.error.message}</p>
      </Frame>
    );
  }
  return <Frame><div className="empty">Загрузка…</div></Frame>;
}

/** The page frame: a way back to the list, the actions of the case, and the scrolling body. */
function Frame({ actions, children }: { actions?: ReactNode; children?: ReactNode }) {
  return (
    <main className="main">
      <header className="topbar obs-topbar">
        <Link className="btn ghost sm" to="/observations">
          ← Наблюдения
        </Link>
        <span className="grow" />
        {actions}
      </header>
      <div className="scroll obs-body">{children}</div>
    </main>
  );
}

type IncidentNote = { id: number; action: "resolve" | "dismiss" };
type CaseDialog = "edit" | "cancel" | "final";

function CaseBody({ view, rights }: { view: CaseView; rights: CaseRights }) {
  const c = view.case;
  const actions = caseActions(c, rights);
  const can = (action: CaseAction) => actions.includes(action);
  const toast = useToast();
  const [dialog, setDialog] = useState<CaseDialog>();
  const [followup, setFollowup] = useState<{ incident?: number }>();
  const [incidentNote, setIncidentNote] = useState<IncidentNote>();
  const arm = useArmCase();
  const start = useStartCase();
  const evaluate = useEvaluateCase();
  const fail = (e: Error) => toast(e.message, "error");
  const armBlocked = view.armProblems.length > 0;
  const openIncidents = view.incidents.filter((i) => i.status === "open").length;

  const headerActions = (
    <>
      {(can("edit") || can("rename")) && (
        <button type="button" className="btn" onClick={() => setDialog("edit")}>
          Изменить
        </button>
      )}
      {can("arm") && (
        <button
          type="button"
          className="btn"
          disabled={arm.isPending || armBlocked}
          title={armBlocked ? "Сначала устраните причины ниже" : undefined}
          onClick={() => arm.mutate(c.id, { onError: fail, onSuccess: () => toast("Наблюдение ждёт поставки") })}
        >
          Ждать поставки
        </button>
      )}
      {can("start") && (
        <button type="button" className="btn primary" disabled={start.isPending} onClick={() => start.mutate(c.id, { onError: fail, onSuccess: () => toast("Наблюдение начато") })}>
          Начать наблюдение
        </button>
      )}
      {can("evaluate") && (
        <button
          type="button"
          className="btn"
          disabled={evaluate.isPending}
          onClick={() => evaluate.mutate({ id: c.id, final: false }, { onError: fail, onSuccess: () => toast("Оценка запрошена") })}
        >
          Запросить оценку
        </button>
      )}
      {can("evaluateFinal") && (
        <button type="button" className="btn" onClick={() => setDialog("final")}>
          Итоговая оценка
        </button>
      )}
      {can("cancel") && (
        <button type="button" className="btn ghost" onClick={() => setDialog("cancel")}>
          Отменить наблюдение
        </button>
      )}
    </>
  );

  return (
    <Frame actions={headerActions}>
      <section className="obs-head">
        <div className="obs-title">
          <span className="mono muted">{c.key}</span>
          <h1>{c.title}</h1>
        </div>
        <dl className="obs-facts">
          <div>
            <dt>Статус</dt>
            <dd>
              <span className="obs-status">
                <span className={`state-dot ${STATUS_DOT[c.status]}`} aria-hidden="true" />
                {STATUS_LABEL[c.status]}
                {c.result && ` · ${RESULT_LABEL[c.result]}`}
              </span>
            </dd>
          </div>
          <div>
            <dt>Политика</dt>
            <dd>{POLICY_LABEL[c.policy]}</dd>
          </div>
          <div>
            <dt>Задачи</dt>
            <dd>
              <TaskLinks ids={c.tasks} />
            </dd>
          </div>
          <div>
            <dt>Начало наблюдения</dt>
            <dd>{when(c.observationStartedAt)}</dd>
          </div>
          <div>
            <dt>Следующая оценка</dt>
            <dd>{when(c.nextEvaluationAt)}</dd>
          </div>
          {c.nextGuardrailAt && (
            <div>
              <dt>Следующая проверка ограничителей</dt>
              <dd>{when(c.nextGuardrailAt)}</dd>
            </div>
          )}
          {c.evaluationRequested && (
            <div>
              <dt>Оценка запрошена</dt>
              <dd>{TRIGGER_LABEL[c.evaluationRequested]}</dd>
            </div>
          )}
          {c.confidence && (
            <div>
              <dt>Уверенность</dt>
              <dd>{CONFIDENCE_LABEL[c.confidence]}</dd>
            </div>
          )}
          {c.conclusion && (
            <div className="wide">
              <dt>{c.status === "cancelled" ? "Причина отмены" : "Вывод"}</dt>
              <dd>{c.conclusion}</dd>
            </div>
          )}
        </dl>
      </section>

      {c.lastError && (
        <section className="obs-warn" role="alert">
          <b>Сбор данных не удаётся: подряд ошибок {c.failures}</b>
          <span className="mono">{c.lastError}</span>
        </section>
      )}
      {can("arm") && armBlocked && (
        <section className="obs-warn" aria-label="Почему наблюдение нельзя ждать поставки">
          <b>Пока наблюдение нельзя перевести в ожидание поставки:</b>
          <ul>
            {view.armProblems.map((problem, i) => (
              <li key={`${i}-${problem}`}>{problem}</li>
            ))}
          </ul>
        </section>
      )}

      <ContractSection c={c} view={view} actions={actions} />
      <DeploymentsSection caseId={c.id} view={view} canRecord={can("deploy")} />
      <EvaluationsSection view={view} />
      <EvidenceSection caseId={c.id} view={view} canReport={can("evidence")} />
      <CaseSection title="Инциденты" note={openIncidents ? `открытых: ${openIncidents}` : undefined}>
        {view.incidents.length ? (
          <ul className="obs-list">
            {view.incidents.map((i) => (
              <IncidentItem
                key={i.id}
                incident={i}
                rights={rights}
                note={incidentNote}
                onNote={setIncidentNote}
                onFollowup={(incident) => setFollowup({ incident })}
              />
            ))}
          </ul>
        ) : (
          <p className="muted">Инцидентов нет.</p>
        )}
      </CaseSection>
      <CaseSection
        title="Задачи по итогам"
        aside={can("followup") && <button type="button" className="btn sm" onClick={() => setFollowup({})}>Создать задачу</button>}
      >
        {view.followups.length ? (
          <ul className="obs-list">
            {view.followups.map((f) => (
              <li key={f.id}>
                <TaskLinks ids={[f.task]} /> <span className="muted">· {f.incident ? `по инциденту I-${f.incident}` : "по итогам наблюдения"} · {when(f.created)}</span>
              </li>
            ))}
          </ul>
        ) : (
          <p className="muted">Задач по итогам пока нет.</p>
        )}
      </CaseSection>

      {dialog === "edit" && <EditDialog c={c} canRename={can("rename")} canEdit={can("edit")} onClose={() => setDialog(undefined)} />}
      {dialog === "cancel" && <CancelDialog caseId={c.id} onClose={() => setDialog(undefined)} />}
      {dialog === "final" && (
        <ConfirmDialog
          title="Итоговая оценка"
          confirmLabel="Оценить и завершить"
          busy={evaluate.isPending}
          onConfirm={() =>
            evaluate.mutate(
              { id: c.id, final: true },
              {
                onError: fail,
                onSuccess: () => {
                  setDialog(undefined);
                  toast("Итоговая оценка запрошена");
                },
              },
            )
          }
          onClose={() => setDialog(undefined)}
        >
          Итоговая оценка пройдёт по имеющимся измерениям и завершит наблюдение итогом. Если данных мало, итог будет «Нет вывода».
        </ConfirmDialog>
      )}
      {followup && <FollowupDialog caseId={c.id} incident={followup.incident} onClose={() => setFollowup(undefined)} />}
    </Frame>
  );
}

// --- sections ---------------------------------------------------------------------------------------

function CaseSection({ title, note, aside, children }: { title: string; note?: ReactNode; aside?: ReactNode; children: ReactNode }) {
  return (
    <section className="obs-section">
      <div className="obs-section-head">
        <h2>{title}</h2>
        {note && <span className="n">{note}</span>}
        {aside}
      </div>
      {children}
    </section>
  );
}

function ContractSection({ c, view, actions }: { c: ObservationCase; view: CaseView; actions: CaseAction[] }) {
  const [editing, setEditing] = useState(false);
  const contract = view.contract;
  const canEdit = actions.includes("contract") || actions.includes("reviseContract");
  // Once observation has started, a revision is a person's call and needs a reason (§6.2).
  const started = c.status === "observing" || c.status === "evaluating";
  return (
    <CaseSection
      title="Контракт"
      note={contract ? `версия ${contract.version} · ${contract.author}` : "не задан"}
      aside={
        canEdit &&
        !editing && (
          <button type="button" className="btn sm" onClick={() => setEditing(true)}>
            {contract ? "Изменить контракт" : "Задать контракт"}
          </button>
        )
      }
    >
      {editing ? (
        <ContractEditor c={c} contract={contract?.contract} started={started} onClose={() => setEditing(false)} />
      ) : contract ? (
        <details>
          <summary>JSON контракта</summary>
          <pre className="obs-code">{JSON.stringify(contract.contract, null, 2)}</pre>
        </details>
      ) : (
        <p className="muted">Контракта пока нет.</p>
      )}
      {view.contracts.length > 0 && (
        <details>
          <summary>История версий: {view.contracts.length}</summary>
          <ol className="obs-list">
            {view.contracts.map((v) => (
              <li key={v.version}>
                версия {v.version} · {v.author} · {when(v.created)}
                {v.reason && ` · ${v.reason}`}
              </li>
            ))}
          </ol>
        </details>
      )}
    </CaseSection>
  );
}

/** The contract as JSON, with a template for the case's policy. The server checks it and its answer is shown here. */
function ContractEditor({ c, contract, started, onClose }: { c: ObservationCase; contract?: Contract; started: boolean; onClose: () => void }) {
  const [text, setText] = useState(() => (contract ? JSON.stringify(contract, null, 2) : contractTemplate(c.policy)));
  const [reason, setReason] = useState("");
  const [syntax, setSyntax] = useState<string>();
  const save = useSetContract();
  const submit = (e: FormEvent) => {
    e.preventDefault();
    // JSON.parse yields `any`; the server validates the shape against the policy (§5.3) and answers with the first problem.
    let body: Contract;
    try {
      body = JSON.parse(text);
    } catch (err) {
      setSyntax(`Контракт должен быть корректным JSON: ${err instanceof Error ? err.message : String(err)}`);
      return;
    }
    setSyntax(undefined);
    save.mutate({ id: c.id, contract: body, reason: reason.trim() || undefined }, { onSuccess: onClose });
  };
  const message = syntax ?? save.error?.message;
  return (
    <form className="obs-form" aria-label="Контракт наблюдения" onSubmit={submit}>
      <label className="field wide">
        <span>Контракт (JSON)</span>
        <textarea className="obs-mono" rows={16} spellCheck={false} required value={text} onChange={(e) => setText(e.target.value)} />
      </label>
      {started && (
        <label className="field wide">
          <span>Причина изменения</span>
          <textarea rows={2} required value={reason} onChange={(e) => setReason(e.target.value)} />
        </label>
      )}
      {message && (
        <p className="obs-error wide" role="alert">
          {message}
        </p>
      )}
      <div className="obs-form-actions">
        <button
          type="button"
          className="btn"
          onClick={() => {
            setText(contractTemplate(c.policy));
            setSyntax(undefined);
          }}
        >
          Шаблон для «{POLICY_LABEL[c.policy]}»
        </button>
        <span className="grow" />
        <button type="button" className="btn ghost" onClick={onClose}>
          Отмена
        </button>
        <button type="submit" className="btn primary" disabled={save.isPending}>
          Сохранить контракт
        </button>
      </div>
    </form>
  );
}

function DeploymentsSection({ caseId, view, canRecord }: { caseId: number; view: CaseView; canRecord: boolean }) {
  return (
    <CaseSection title="Развёртывания" note={view.deployments.length ? `записано: ${view.deployments.length}` : undefined}>
      {view.deployments.length ? (
        <div className="obs-table-wrap">
          <table className="obs-table" aria-label="Развёртывания">
            <thead>
              <tr>
                <th scope="col">Ссылка</th>
                <th scope="col">Окружение</th>
                <th scope="col">Время</th>
                <th scope="col">Записал</th>
              </tr>
            </thead>
            <tbody>
              {view.deployments.map((d) => (
                <tr key={d.id}>
                  <td className="mono">{d.ref}</td>
                  <td>{d.environment || "—"}</td>
                  <td>{when(d.at)}</td>
                  <td>{d.recordedBy}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <p className="muted">Развёртываний не записано.</p>
      )}
      {canRecord && <DeploymentForm caseId={caseId} />}
    </CaseSection>
  );
}

function DeploymentForm({ caseId }: { caseId: number }) {
  const [ref, setRef] = useState("");
  const [environment, setEnvironment] = useState("");
  const [at, setAt] = useState("");
  const record = useRecordDeployment();
  const toast = useToast();
  return (
    <form
      className="obs-form"
      aria-label="Записать развёртывание"
      onSubmit={(e) => {
        e.preventDefault();
        record.mutate(
          { id: caseId, ref: ref.trim(), environment: environment.trim() || undefined, at: at ? new Date(at).toISOString() : undefined },
          {
            onSuccess: () => {
              setRef("");
              setEnvironment("");
              setAt("");
              toast("Развёртывание записано");
            },
          },
        );
      }}
    >
      <label className="field">
        <span>Коммит, тег или релиз</span>
        <input required maxLength={200} value={ref} onChange={(e) => setRef(e.target.value)} />
      </label>
      <label className="field">
        <span>Окружение</span>
        <input value={environment} onChange={(e) => setEnvironment(e.target.value)} />
      </label>
      <label className="field">
        <span>Время (по умолчанию сейчас)</span>
        <input type="datetime-local" value={at} onChange={(e) => setAt(e.target.value)} />
      </label>
      <div className="obs-form-actions">
        <span className="grow" />
        {record.error && (
          <p className="obs-error" role="alert">
            {record.error.message}
          </p>
        )}
        <button type="submit" className="btn" disabled={record.isPending}>
          Записать развёртывание
        </button>
      </div>
    </form>
  );
}

function EvaluationsSection({ view }: { view: CaseView }) {
  return (
    <CaseSection title="Оценки">
      {view.evaluations.length ? (
        <ol className="obs-list">
          {view.evaluations.map((e) => (
            <li key={e.id} className="obs-eval">
              <div className="obs-eval-head">
                <b>{VERDICT_LABEL[e.verdict]}</b>
                <span className="muted">{TRIGGER_LABEL[e.trigger]}</span>
                {e.confidence && <span className="muted">уверенность: {CONFIDENCE_LABEL[e.confidence]}</span>}
                <span className="muted">{when(e.at)}</span>
              </div>
              {e.summary && <p>{e.summary}</p>}
              {e.details.length > 0 && (
                <ul className="obs-criteria">
                  {e.details.map((d, i) => (
                    <li key={`${d.criterion}-${i}`}>
                      <span className="mono">{d.criterion}</span>: {CRITERION_LABEL[d.state]}
                      {d.reason && <span className="muted"> — {d.reason}</span>}
                    </li>
                  ))}
                </ul>
              )}
            </li>
          ))}
        </ol>
      ) : (
        <p className="muted">Оценок пока нет.</p>
      )}
    </CaseSection>
  );
}

function EvidenceSection({ caseId, view, canReport }: { caseId: number; view: CaseView; canReport: boolean }) {
  const contract = view.contract?.contract;
  // The change thresholds come from the current contract; the evidence rows do not carry theirs.
  const criteria = contract ? [...contract.hypotheses.map((h) => h.id), ...contract.guardrails.map((g) => g.id)] : [];
  return (
    <CaseSection title="Измерения" note={view.evidence.length >= 100 ? "последние 100" : undefined}>
      {view.evidence.length ? (
        <div className="obs-table-wrap">
          <table className="obs-table" aria-label="Измерения">
            <thead>
              <tr>
                <th scope="col">Время</th>
                <th scope="col">Критерий</th>
                <th scope="col">Фаза</th>
                <th scope="col">Значение</th>
                <th scope="col">База</th>
                <th scope="col">Изменение</th>
                <th scope="col">Выборка</th>
                <th scope="col">Источник</th>
                <th scope="col">Доверие</th>
                <th scope="col">Примечание</th>
              </tr>
            </thead>
            <tbody>
              {view.evidence.map((e) => (
                <tr key={e.id}>
                  <td>{when(e.at)}</td>
                  <td className="mono">{e.criterion}</td>
                  <td>{PHASE_LABEL[e.phase]}</td>
                  <td>{e.value ?? "—"}</td>
                  <td>{e.baseline ?? "—"}</td>
                  <td>{formatDelta(e, contract ? criterionChange(contract, e.criterion) : undefined)}</td>
                  <td>{e.samples ?? "—"}</td>
                  <td className="mono">{e.source}</td>
                  <td>{TRUST_LABEL[e.trust]}</td>
                  <td>{e.error ? <span className="obs-error">{e.error}</span> : e.note || "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <p className="muted">Измерений пока нет.</p>
      )}
      {canReport && criteria.length > 0 && <EvidenceForm caseId={caseId} criteria={criteria} />}
    </CaseSection>
  );
}

function EvidenceForm({ caseId, criteria }: { caseId: number; criteria: string[] }) {
  const [criterion, setCriterion] = useState("");
  const [phase, setPhase] = useState<EvidencePhase>("observed");
  const [value, setValue] = useState("");
  const [baseline, setBaseline] = useState("");
  const [note, setNote] = useState("");
  const report = useReportEvidence();
  const toast = useToast();
  const chosen = criteria.includes(criterion) ? criterion : (criteria[0] ?? "");
  return (
    <form
      className="obs-form"
      aria-label="Добавить измерение"
      onSubmit={(e) => {
        e.preventDefault();
        report.mutate(
          {
            id: caseId,
            criterion: chosen,
            phase,
            value: Number(value),
            baseline: baseline.trim() === "" ? undefined : Number(baseline),
            note: note.trim() || undefined,
          },
          {
            onSuccess: () => {
              setValue("");
              setBaseline("");
              setNote("");
              toast("Измерение записано");
            },
          },
        );
      }}
    >
      <label className="field">
        <span>Критерий</span>
        <select value={chosen} onChange={(e) => setCriterion(e.target.value)}>
          {criteria.map((id) => (
            <option key={id} value={id}>
              {id}
            </option>
          ))}
        </select>
      </label>
      <label className="field">
        <span>Фаза</span>
        <select
          value={phase}
          onChange={(e) => {
            const next = PHASES.find((p) => p === e.target.value);
            if (next) setPhase(next);
          }}
        >
          {PHASES.map((p) => (
            <option key={p} value={p}>
              {PHASE_LABEL[p]}
            </option>
          ))}
        </select>
      </label>
      <label className="field">
        <span>Значение</span>
        <input type="number" step="any" required value={value} onChange={(e) => setValue(e.target.value)} />
      </label>
      <label className="field">
        <span>База (необязательно)</span>
        <input type="number" step="any" value={baseline} onChange={(e) => setBaseline(e.target.value)} />
      </label>
      <label className="field wide">
        <span>Примечание</span>
        <input value={note} onChange={(e) => setNote(e.target.value)} />
      </label>
      <div className="obs-form-actions">
        <span className="grow" />
        {report.error && (
          <p className="obs-error" role="alert">
            {report.error.message}
          </p>
        )}
        <button type="submit" className="btn" disabled={report.isPending}>
          Записать измерение
        </button>
      </div>
    </form>
  );
}

function IncidentItem({
  incident: i,
  rights,
  note,
  onNote,
  onFollowup,
}: {
  incident: Incident;
  rights: CaseRights;
  note?: IncidentNote;
  onNote: (next: IncidentNote | undefined) => void;
  onFollowup: (incident: number) => void;
}) {
  const actions = incidentActions(i, rights);
  return (
    <li className={`obs-incident${i.status === "open" ? " open" : ""}`}>
      <div className="obs-incident-head">
        <span className="mono">{i.key}</span>
        <span className={`obs-badge ${i.status}`}>{INCIDENT_LABEL[i.status]}</span>
        <b className="mono">{i.guardrail}</b>
        <span className="muted">{when(i.created)}</span>
      </div>
      {i.summary && <p>{i.summary}</p>}
      {i.note && <p className="muted">Примечание: {i.note}</p>}
      {i.followups.length > 0 && (
        <p className="muted">
          Задачи: <TaskLinks ids={i.followups} />
        </p>
      )}
      {actions.length > 0 && (
        <div className="obs-actions">
          {actions.includes("resolve") && (
            <button type="button" className="btn sm" onClick={() => onNote({ id: i.id, action: "resolve" })}>
              Разрешить
            </button>
          )}
          {actions.includes("dismiss") && (
            <button type="button" className="btn sm" onClick={() => onNote({ id: i.id, action: "dismiss" })}>
              Отклонить
            </button>
          )}
          {actions.includes("followup") && (
            <button type="button" className="btn sm" onClick={() => onFollowup(i.id)}>
              Создать задачу
            </button>
          )}
        </div>
      )}
      {note?.id === i.id && <IncidentNoteForm incident={i} action={note.action} onDone={() => onNote(undefined)} />}
    </li>
  );
}

/** Resolving takes an optional note; dismissing a person's note, which says why it was no incident (§10.1). */
function IncidentNoteForm({ incident, action, onDone }: { incident: Incident; action: "resolve" | "dismiss"; onDone: () => void }) {
  const [note, setNote] = useState("");
  const resolve = useResolveIncident();
  const dismiss = useDismissIncident();
  const dismissing = action === "dismiss";
  const pending = dismissing ? dismiss.isPending : resolve.isPending;
  const error = dismissing ? dismiss.error?.message : resolve.error?.message;
  const submit = () => {
    const body = { id: incident.id, note: note.trim() || undefined };
    if (dismissing) dismiss.mutate(body, { onSuccess: onDone });
    else resolve.mutate(body, { onSuccess: onDone });
  };
  return (
    <form
      className="obs-form obs-incident-form"
      aria-label={dismissing ? `Отклонить ${incident.key}` : `Разрешить ${incident.key}`}
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
    >
      <label className="field wide">
        <span>{dismissing ? "Почему это не инцидент" : "Примечание (необязательно)"}</span>
        <textarea rows={2} required={dismissing} value={note} onChange={(e) => setNote(e.target.value)} />
      </label>
      {error && (
        <p className="obs-error wide" role="alert">
          {error}
        </p>
      )}
      <div className="obs-form-actions">
        <span className="grow" />
        <button type="button" className="btn ghost" onClick={onDone}>
          Отмена
        </button>
        <button type="submit" className={`btn ${dismissing ? "danger" : "primary"}`} disabled={pending}>
          {dismissing ? "Отклонить инцидент" : "Разрешить инцидент"}
        </button>
      </div>
    </form>
  );
}

// --- dialogs ----------------------------------------------------------------------------------------

function FormDialog({
  title,
  submitLabel,
  danger,
  busy,
  error,
  onSubmit,
  onClose,
  children,
}: {
  title: string;
  submitLabel: string;
  danger?: boolean;
  busy?: boolean;
  error?: string;
  onSubmit: () => void;
  onClose: () => void;
  children: ReactNode;
}) {
  return (
    <Modal label={title} onClose={onClose}>
      <form
        className="obs-dialog"
        onSubmit={(e) => {
          e.preventDefault();
          onSubmit();
        }}
      >
        <div className="mh">{title}</div>
        <div className="mb">
          {children}
          {error && (
            <p className="obs-error" role="alert">
              {error}
            </p>
          )}
        </div>
        <div className="mf">
          <span className="grow" />
          <button type="button" className="btn ghost" onClick={onClose}>
            Отмена
          </button>
          <button type="submit" className={`btn ${danger ? "danger" : "primary"}`} disabled={busy}>
            {submitLabel}
          </button>
        </div>
      </form>
    </Modal>
  );
}

/** Title, policy and tasks. Only the fields the caller may change are shown; only the changes are sent (§6.2). */
function EditDialog({ c, canRename, canEdit, onClose }: { c: ObservationCase; canRename: boolean; canEdit: boolean; onClose: () => void }) {
  const [title, setTitle] = useState(c.title);
  const [policy, setPolicy] = useState<ObservationPolicy>(c.policy);
  const [tasks, setTasks] = useState<string[]>(c.tasks);
  const [adding, setAdding] = useState("");
  const update = useUpdateCase();
  const addTask = () => {
    const id = adding.trim();
    if (id && !tasks.includes(id)) setTasks([...tasks, id]);
    setAdding("");
  };
  const submit = () => {
    const body: UpdateCaseBody = {};
    if (canRename && title.trim() !== c.title) body.title = title.trim();
    if (canEdit) {
      if (policy !== c.policy) body.policy = policy;
      const addTasks = tasks.filter((t) => !c.tasks.includes(t));
      const removeTasks = c.tasks.filter((t) => !tasks.includes(t));
      if (addTasks.length) body.addTasks = addTasks;
      if (removeTasks.length) body.removeTasks = removeTasks;
    }
    if (Object.keys(body).length === 0) {
      onClose();
      return;
    }
    update.mutate({ id: c.id, ...body }, { onSuccess: onClose });
  };
  return (
    <FormDialog title="Изменить наблюдение" submitLabel="Сохранить" busy={update.isPending} error={update.error?.message} onSubmit={submit} onClose={onClose}>
      {canRename && (
        <label className="field">
          <span>Название</span>
          <input required autoFocus value={title} onChange={(e) => setTitle(e.target.value)} />
        </label>
      )}
      {canEdit && (
        <>
          <label className="field">
            <span>Политика</span>
            <select
              value={policy}
              onChange={(e) => {
                const next = POLICIES.find((p) => p === e.target.value);
                if (next) setPolicy(next);
              }}
            >
              {POLICIES.map((p) => (
                <option key={p} value={p}>
                  {POLICY_LABEL[p]}
                </option>
              ))}
            </select>
          </label>
          <fieldset className="obs-tasks-field">
            <legend>Задачи</legend>
            {tasks.length ? (
              <ul className="obs-chips">
                {tasks.map((t) => (
                  <li key={t}>
                    <span className="mono">{t}</span>
                    <button type="button" className="icon-btn" aria-label={`Убрать задачу ${t}`} title="Убрать" onClick={() => setTasks(tasks.filter((x) => x !== t))}>
                      <Icon.close size={12} />
                    </button>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="muted">Задач нет.</p>
            )}
            <div className="obs-add">
              <input
                aria-label="Номер задачи"
                placeholder="Номер задачи"
                value={adding}
                onChange={(e) => setAdding(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    addTask();
                  }
                }}
              />
              <button type="button" className="btn" disabled={!adding.trim()} onClick={addTask}>
                Добавить
              </button>
            </div>
          </fieldset>
        </>
      )}
    </FormDialog>
  );
}

function CancelDialog({ caseId, onClose }: { caseId: number; onClose: () => void }) {
  const [reason, setReason] = useState("");
  const cancel = useCancelCase();
  return (
    <FormDialog
      title="Отменить наблюдение"
      submitLabel="Отменить наблюдение"
      danger
      busy={cancel.isPending}
      error={cancel.error?.message}
      onSubmit={() => cancel.mutate({ id: caseId, reason: reason.trim() }, { onSuccess: onClose })}
      onClose={onClose}
    >
      <label className="field">
        <span>Причина</span>
        <textarea required autoFocus rows={3} value={reason} onChange={(e) => setReason(e.target.value)} />
      </label>
    </FormDialog>
  );
}

function FollowupDialog({ caseId, incident, onClose }: { caseId: number; incident?: number; onClose: () => void }) {
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const followup = useFileFollowup();
  const toast = useToast();
  return (
    <FormDialog
      title={incident ? `Задача по инциденту I-${incident}` : "Задача по итогам"}
      submitLabel="Создать задачу"
      busy={followup.isPending}
      error={followup.error?.message}
      onSubmit={() =>
        followup.mutate(
          { id: caseId, incident, title: title.trim() || undefined, description: description.trim() || undefined },
          {
            onSuccess: () => {
              toast("Задача создана");
              onClose();
            },
          },
        )
      }
      onClose={onClose}
    >
      <p className="muted">Название и описание можно не заполнять: сервер подберёт их по инциденту или итогам.</p>
      <label className="field">
        <span>Название</span>
        <input autoFocus value={title} onChange={(e) => setTitle(e.target.value)} />
      </label>
      <label className="field">
        <span>Описание</span>
        <textarea rows={4} value={description} onChange={(e) => setDescription(e.target.value)} />
      </label>
    </FormDialog>
  );
}
