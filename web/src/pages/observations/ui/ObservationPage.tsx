// «Наблюдение» — one observation case (docs/platform/observation-plane.md §14, §17): its facts,
// the actions the caller may take (caseActions, the rules the server enforces too), the contract,
// deployments, evaluations, evidence, incidents, and the follow-up tasks.

import { type FormEvent, type ReactNode, useState } from "react";
import { Link, useParams } from "react-router";
import {
  ACTION_LABEL,
  type CaseAction,
  type CaseRights,
  CONFIDENCE_LABEL,
  type Contract,
  CRITERION_STATE_LABEL,
  FIELD_LABEL,
  HEADING_LABEL,
  INCIDENT_ACTION_LABEL,
  INCIDENT_LABEL,
  OBSERVATION_TEXT,
  caseActions,
  contractTemplate,
  criterionChange,
  formatDelta,
  incidentActions,
  parseContract,
  PHASE_LABEL,
  POLICY_LABEL,
  RESULT_LABEL,
  START_LABEL,
  caseDot,
  STATUS_LABEL,
  TRIGGER_LABEL,
  TRUST_LABEL,
  VERDICT_LABEL,
  type ObservationCase,
  type ObservationPolicy,
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
import { canWriteProject, useSession } from "@/entities/session";
import type { CaseView } from "@/shared/api/generated/CaseView.ts";
import type { EvidencePhase } from "@/shared/api/generated/EvidencePhase.ts";
import type { Incident } from "@/shared/api/generated/Incident.ts";
import type { UpdateCaseBody } from "@/shared/api/generated/UpdateCaseBody.ts";
import "@/shared/ui/agents.css";
import { ConfirmDialog, Icon, Modal, useToast } from "@/shared/ui";
import { TaskLinks, when } from "./parts.tsx";
import "./observations.css";

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
  return { writable: canWriteProject(useSession().data), human: true };
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
          ← {OBSERVATION_TEXT.nav}
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
          {ACTION_LABEL.edit}
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
          {ACTION_LABEL.arm}
        </button>
      )}
      {can("start") && (
        <button type="button" className="btn primary" disabled={start.isPending} onClick={() => start.mutate(c.id, { onError: fail, onSuccess: () => toast("Наблюдение начато") })}>
          {ACTION_LABEL.start}
        </button>
      )}
      {can("evaluate") && (
        <button
          type="button"
          className="btn"
          disabled={evaluate.isPending}
          onClick={() => evaluate.mutate({ id: c.id, final: false }, { onError: fail, onSuccess: () => toast("Оценка запрошена") })}
        >
          {ACTION_LABEL.evaluate}
        </button>
      )}
      {can("evaluateFinal") && (
        <button type="button" className="btn" onClick={() => setDialog("final")}>
          {ACTION_LABEL.evaluateFinal}
        </button>
      )}
      {can("cancel") && (
        <button type="button" className="btn ghost" onClick={() => setDialog("cancel")}>
          {ACTION_LABEL.cancel}
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
            <dt>{FIELD_LABEL.status}</dt>
            <dd>
              <span className="obs-status">
                <span className={`state-dot ${caseDot(c)}`} aria-hidden="true" />
                {STATUS_LABEL[c.status]}
                {c.result && ` · ${RESULT_LABEL[c.result]}`}
              </span>
            </dd>
          </div>
          <div>
            <dt>{FIELD_LABEL.policy}</dt>
            <dd>{POLICY_LABEL[c.policy]}</dd>
          </div>
          <div>
            <dt>{FIELD_LABEL.tasks}</dt>
            <dd>
              <TaskLinks ids={c.tasks} />
            </dd>
          </div>
          {view.contract && (
            <div>
              <dt>{FIELD_LABEL.startOn}</dt>
              <dd>{START_LABEL[view.contract.contract.observation.start]}</dd>
            </div>
          )}
          <div>
            <dt>{FIELD_LABEL.observationStart}</dt>
            <dd>{when(c.observationStartedAt)}</dd>
          </div>
          <div>
            <dt>{FIELD_LABEL.nextEvaluation}</dt>
            <dd>{when(c.nextEvaluationAt)}</dd>
          </div>
          {c.nextGuardrailAt && (
            <div>
              <dt>{FIELD_LABEL.nextGuardrail}</dt>
              <dd>{when(c.nextGuardrailAt)}</dd>
            </div>
          )}
          {c.evaluationRequested && (
            <div>
              <dt>{FIELD_LABEL.evaluationRequested}</dt>
              <dd>{TRIGGER_LABEL[c.evaluationRequested]}</dd>
            </div>
          )}
          {c.confidence && (
            <div>
              <dt>{FIELD_LABEL.confidence}</dt>
              <dd>{CONFIDENCE_LABEL[c.confidence]}</dd>
            </div>
          )}
          {c.conclusion && (
            <div className="wide">
              <dt>{c.status === "cancelled" ? FIELD_LABEL.cancelReason : FIELD_LABEL.conclusion}</dt>
              <dd>{c.conclusion}</dd>
            </div>
          )}
        </dl>
      </section>

      {c.lastError && (
        <section className="obs-warn" role="status">
          <b>
            Не удаётся собрать данные: ошибок подряд — {c.failures}
          </b>
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
      <CaseSection title={HEADING_LABEL.incidents} note={openIncidents ? `открытых: ${openIncidents}` : undefined}>
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
        title={HEADING_LABEL.followups}
        aside={
          can("followup") && (
            <button type="button" className="btn sm" onClick={() => setFollowup({})}>
              {ACTION_LABEL.followup}
            </button>
          )
        }
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
          title={ACTION_LABEL.evaluateFinal}
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
          Итоговая оценка пройдёт по имеющимся измерениям и завершит наблюдение итогом. Если данных мало, итог будет «{VERDICT_LABEL.inconclusive}».
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
      title={HEADING_LABEL.contract}
      note={contract ? `версия ${contract.version} · ${contract.author}` : "не задан"}
      aside={
        canEdit &&
        !editing && (
          <button type="button" className="btn sm" onClick={() => setEditing(true)}>
            {contract ? ACTION_LABEL.reviseContract : ACTION_LABEL.contract}
          </button>
        )
      }
    >
      {editing ? (
        <ContractEditor c={c} contract={contract?.contract} started={started} onClose={() => setEditing(false)} />
      ) : contract ? (
        <details>
          <summary>{FIELD_LABEL.contractJson}</summary>
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
  const [initial] = useState(() => (contract ? JSON.stringify(contract, null, 2) : contractTemplate(c.policy)));
  const [text, setText] = useState(initial);
  const [reason, setReason] = useState("");
  const [syntax, setSyntax] = useState<string>();
  const [confirmTemplate, setConfirmTemplate] = useState(false);
  const save = useSetContract();
  const applyTemplate = () => {
    setText(contractTemplate(c.policy));
    setSyntax(undefined);
  };
  const submit = (e: FormEvent) => {
    e.preventDefault();
    const parsed = parseContract(text);
    if ("error" in parsed) {
      setSyntax(parsed.error);
      return;
    }
    setSyntax(undefined);
    save.mutate({ id: c.id, contract: parsed.contract, reason: reason.trim() || undefined }, { onSuccess: onClose });
  };
  const message = syntax ?? save.error?.message;
  return (
    <>
      <form className="obs-form" aria-label={HEADING_LABEL.contract} onSubmit={submit}>
        <label className="field wide">
          <span>{FIELD_LABEL.contractJson}</span>
          <textarea className="obs-mono" rows={16} spellCheck={false} required autoFocus value={text} onChange={(e) => setText(e.target.value)} />
        </label>
        {started && (
          <label className="field wide">
            <span>{FIELD_LABEL.reason}</span>
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
            onClick={() => (text === initial ? applyTemplate() : setConfirmTemplate(true))}
          >
            Шаблон для «{POLICY_LABEL[c.policy]}»
          </button>
          <span className="grow" />
          <button type="button" className="btn ghost" onClick={onClose}>
            Отмена
          </button>
          <button type="submit" className="btn primary" disabled={save.isPending || !text.trim() || (started && !reason.trim())}>
            Сохранить контракт
          </button>
        </div>
      </form>
      {confirmTemplate && (
        <ConfirmDialog
          title="Заменить текст шаблоном?"
          confirmLabel="Заменить"
          danger
          onConfirm={() => {
            applyTemplate();
            setConfirmTemplate(false);
          }}
          onClose={() => setConfirmTemplate(false)}
        >
          Изменения в тексте контракта будут потеряны.
        </ConfirmDialog>
      )}
    </>
  );
}

function DeploymentsSection({ caseId, view, canRecord }: { caseId: number; view: CaseView; canRecord: boolean }) {
  return (
    <CaseSection title={HEADING_LABEL.deployments} note={view.deployments.length ? `записано: ${view.deployments.length}` : undefined}>
      {view.deployments.length ? (
        <div className="ag-table-wrap">
          <table className="ag-table" aria-label={HEADING_LABEL.deployments}>
            <thead>
              <tr>
                <th scope="col">{FIELD_LABEL.ref}</th>
                <th scope="col">{FIELD_LABEL.environment}</th>
                <th scope="col">{FIELD_LABEL.at}</th>
                <th scope="col">{FIELD_LABEL.recordedBy}</th>
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
      aria-label={ACTION_LABEL.deploy}
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
        <span>{FIELD_LABEL.ref}</span>
        <input required maxLength={200} value={ref} onChange={(e) => setRef(e.target.value)} />
      </label>
      <label className="field">
        <span>{FIELD_LABEL.environment}</span>
        <input value={environment} onChange={(e) => setEnvironment(e.target.value)} />
      </label>
      <label className="field">
        <span>{FIELD_LABEL.at} (по умолчанию сейчас)</span>
        <input type="datetime-local" value={at} onChange={(e) => setAt(e.target.value)} />
      </label>
      <div className="obs-form-actions">
        <span className="grow" />
        {record.error && (
          <p className="obs-error" role="alert">
            {record.error.message}
          </p>
        )}
        <button type="submit" className="btn" disabled={record.isPending || !ref.trim()}>
          {ACTION_LABEL.deploy}
        </button>
      </div>
    </form>
  );
}

function EvaluationsSection({ view }: { view: CaseView }) {
  return (
    <CaseSection title={HEADING_LABEL.evaluations}>
      {view.evaluations.length ? (
        <ol className="obs-list">
          {view.evaluations.map((e) => (
            <li key={e.id} className="obs-eval">
              <div className="obs-eval-head">
                <b>{VERDICT_LABEL[e.verdict]}</b>
                <span className="muted">{TRIGGER_LABEL[e.trigger]}</span>
                {e.confidence && <span className="muted">{FIELD_LABEL.confidence}: {CONFIDENCE_LABEL[e.confidence]}</span>}
                <span className="muted">{when(e.at)}</span>
              </div>
              {e.summary && <p>{e.summary}</p>}
              {e.details.length > 0 && (
                <ul className="obs-criteria">
                  {e.details.map((d, i) => (
                    <li key={`${d.criterion}-${i}`}>
                      <span className="mono">{d.criterion}</span>: {CRITERION_STATE_LABEL[d.state]}
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
  const contract = view.contract;
  // The criteria of the current contract: the form picks from them, and a row's change threshold
  // comes from it only when the row was measured under that same version (§7, §9).
  const criteria = contract ? [...contract.contract.hypotheses.map((h) => h.id), ...contract.contract.guardrails.map((g) => g.id)] : [];
  return (
    <CaseSection title={HEADING_LABEL.evidence} note={view.evidence.length >= 100 ? "последние 100" : undefined}>
      {view.evidence.length ? (
        <div className="ag-table-wrap">
          <table className="ag-table" aria-label={HEADING_LABEL.evidence}>
            <thead>
              <tr>
                <th scope="col">{FIELD_LABEL.at}</th>
                <th scope="col">{FIELD_LABEL.criterion}</th>
                <th scope="col">{FIELD_LABEL.phase}</th>
                <th scope="col">{FIELD_LABEL.value}</th>
                <th scope="col">{FIELD_LABEL.baseline}</th>
                <th scope="col">{FIELD_LABEL.change}</th>
                <th scope="col">{FIELD_LABEL.samples}</th>
                <th scope="col">{FIELD_LABEL.source}</th>
                <th scope="col">{FIELD_LABEL.trust}</th>
                <th scope="col">{FIELD_LABEL.note}</th>
              </tr>
            </thead>
            <tbody>
              {view.evidence.map((e) => {
                const threshold = contract && e.contractVersion === contract.version ? criterionChange(contract.contract, e.criterion) : undefined;
                return (
                  <tr key={e.id}>
                    <td>{when(e.at)}</td>
                    <td className="mono">{e.criterion}</td>
                    <td>{PHASE_LABEL[e.phase]}</td>
                    <td>{e.value ?? "—"}</td>
                    <td>{e.baseline ?? "—"}</td>
                    <td>{formatDelta(e, threshold)}</td>
                    <td>{e.samples ?? "—"}</td>
                    <td className="mono">{e.source}</td>
                    <td>{TRUST_LABEL[e.trust]}</td>
                    <td>{e.error ? <span className="obs-error">{e.error}</span> : e.note || "—"}</td>
                  </tr>
                );
              })}
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
      aria-label={ACTION_LABEL.evidence}
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
        <span>{FIELD_LABEL.criterion}</span>
        <select value={chosen} onChange={(e) => setCriterion(e.target.value)}>
          {criteria.map((id) => (
            <option key={id} value={id}>
              {id}
            </option>
          ))}
        </select>
      </label>
      <label className="field">
        <span>{FIELD_LABEL.phase}</span>
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
        <span>{FIELD_LABEL.value}</span>
        <input type="number" step="any" required value={value} onChange={(e) => setValue(e.target.value)} />
      </label>
      <label className="field">
        <span>{FIELD_LABEL.baseline} (необязательно)</span>
        <input type="number" step="any" value={baseline} onChange={(e) => setBaseline(e.target.value)} />
      </label>
      <label className="field wide">
        <span>{FIELD_LABEL.note}</span>
        <input value={note} onChange={(e) => setNote(e.target.value)} />
      </label>
      <div className="obs-form-actions">
        <span className="grow" />
        {report.error && (
          <p className="obs-error" role="alert">
            {report.error.message}
          </p>
        )}
        <button type="submit" className="btn" disabled={report.isPending || !value.trim()}>
          {ACTION_LABEL.evidence}
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
      {i.note && <p className="muted">{FIELD_LABEL.note}: {i.note}</p>}
      {i.followups.length > 0 && (
        <p className="muted">
          {HEADING_LABEL.followups}: <TaskLinks ids={i.followups} />
        </p>
      )}
      {actions.length > 0 && (
        <div className="obs-actions">
          {actions.includes("resolve") && (
            <button type="button" className="btn sm" onClick={() => onNote({ id: i.id, action: "resolve" })}>
              {INCIDENT_ACTION_LABEL.resolve}
            </button>
          )}
          {actions.includes("dismiss") && (
            <button type="button" className="btn sm" onClick={() => onNote({ id: i.id, action: "dismiss" })}>
              {INCIDENT_ACTION_LABEL.dismiss}
            </button>
          )}
          {actions.includes("followup") && (
            <button type="button" className="btn sm" onClick={() => onFollowup(i.id)}>
              {INCIDENT_ACTION_LABEL.followup}
            </button>
          )}
        </div>
      )}
      {note?.id === i.id && (
        <IncidentNoteForm key={`${i.id}-${note.action}`} incident={i} action={note.action} onDone={() => onNote(undefined)} />
      )}
    </li>
  );
}

/** Resolving takes an optional note; dismissing takes a required one, which says why it was no incident (§10.1). */
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
      aria-label={`${INCIDENT_ACTION_LABEL[action]} ${incident.key}`}
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
    >
      <label className="field wide">
        <span>{dismissing ? "Почему это не инцидент" : `${FIELD_LABEL.note} (необязательно)`}</span>
        <textarea autoFocus rows={2} required={dismissing} value={note} onChange={(e) => setNote(e.target.value)} />
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
        <button type="submit" className={`btn ${dismissing ? "danger" : "primary"}`} disabled={pending || (dismissing && !note.trim())}>
          {INCIDENT_ACTION_LABEL[action]}
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
  canSubmit = true,
  error,
  onSubmit,
  onClose,
  children,
}: {
  title: string;
  submitLabel: string;
  danger?: boolean;
  busy?: boolean;
  /** False while a required field is blank: the submit button stays disabled. */
  canSubmit?: boolean;
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
          <button type="submit" className={`btn ${danger ? "danger" : "primary"}`} disabled={busy || !canSubmit}>
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
    <FormDialog
      title={ACTION_LABEL.edit}
      submitLabel="Сохранить"
      busy={update.isPending}
      canSubmit={!canRename || title.trim() !== ""}
      error={update.error?.message}
      onSubmit={submit}
      onClose={onClose}
    >
      {canRename && (
        <label className="field">
          <span>{FIELD_LABEL.title}</span>
          <input required autoFocus value={title} onChange={(e) => setTitle(e.target.value)} />
        </label>
      )}
      {canEdit && (
        <>
          <label className="field">
            <span>{FIELD_LABEL.policy}</span>
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
            <legend>{FIELD_LABEL.tasks}</legend>
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
      title={ACTION_LABEL.cancel}
      submitLabel={ACTION_LABEL.cancel}
      danger
      busy={cancel.isPending}
      canSubmit={reason.trim() !== ""}
      error={cancel.error?.message}
      onSubmit={() => cancel.mutate({ id: caseId, reason: reason.trim() }, { onSuccess: onClose })}
      onClose={onClose}
    >
      <label className="field">
        <span>{FIELD_LABEL.reason}</span>
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
      title={incident ? `${HEADING_LABEL.followups}: инцидент I-${incident}` : HEADING_LABEL.followups}
      submitLabel={ACTION_LABEL.followup}
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
        <span>{FIELD_LABEL.title}</span>
        <input autoFocus value={title} onChange={(e) => setTitle(e.target.value)} />
      </label>
      <label className="field">
        <span>{FIELD_LABEL.description}</span>
        <textarea rows={4} value={description} onChange={(e) => setDescription(e.target.value)} />
      </label>
    </FormDialog>
  );
}
