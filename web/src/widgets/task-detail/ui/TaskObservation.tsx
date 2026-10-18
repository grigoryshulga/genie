import { type FormEvent, useState } from "react";
import { Link, useNavigate } from "react-router";
import {
  newCaseTitle,
  type ObservationCase,
  type ObservationPolicy,
  OBSERVATION_TEXT,
  POLICY_LABEL,
  RESULT_LABEL,
  STATUS_LABEL,
  useCreateCase,
  useTaskObservations,
} from "@/entities/observation";
import { useSession } from "@/entities/session";
import type { Task } from "@/entities/task";
import { useToast } from "@/shared/ui";

const POLICIES = Object.keys(POLICY_LABEL) as ObservationPolicy[];
/** The check a delivered task is usually observed with: whether its result held (§5.1). */
const DEFAULT_POLICY: ObservationPolicy = "verify";

/** A case's status, with its result once it has one. */
function caseState(c: ObservationCase): string {
  return c.result ? `${STATUS_LABEL[c.status]} · ${RESULT_LABEL[c.result]}` : STATUS_LABEL[c.status];
}

/**
 * The outcome cases of a task (§17), shown on its card. Everyone who can see the task sees its
 * cases; a writer can start a proposed case linked to this task, which then opens its own page.
 * Not shown for epics: the caller renders it only for tasks.
 */
export function TaskObservation({ task }: { task: Task }) {
  const navigate = useNavigate();
  const toast = useToast();
  const query = useTaskObservations(task.id);
  const create = useCreateCase();
  // The same writer rule as TaskManager: a member, an admin or an owner, or a local session.
  const session = useSession().data;
  const role = session?.projects.find((p) => p.slug === session.project)?.role;
  const writable = session?.mode === "local" || (role !== undefined && role !== "viewer");
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [policy, setPolicy] = useState<ObservationPolicy>(DEFAULT_POLICY);

  const startForm = () => {
    setTitle(newCaseTitle(task.title));
    setPolicy(DEFAULT_POLICY);
    setOpen(true);
  };

  const submit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const name = title.trim();
    if (!name) return;
    create.mutate(
      { title: name, policy, tasks: [task.id] },
      {
        onSuccess: (view) => {
          setOpen(false);
          navigate(`/observation/${view.case.id}`);
        },
        onError: (err) => toast(`Не удалось: ${err.message}`, "error"),
      },
    );
  };

  const cases = query.data ?? [];

  return (
    <section className="sec" aria-label={OBSERVATION_TEXT.section}>
      <h3>
        {OBSERVATION_TEXT.section}
        {query.isSuccess && cases.length > 0 && <span className="n">{cases.length}</span>}
        {writable && !open && (
          <button type="button" className="btn ghost" style={{ height: 24 }} onClick={startForm}>
            {OBSERVATION_TEXT.create}
          </button>
        )}
      </h3>
      {query.isPending && <div className="muted">Загрузка…</div>}
      {query.isError && <p role="alert">{query.error.message}</p>}
      {open && (
        <form aria-label={OBSERVATION_TEXT.create} onSubmit={submit} style={{ display: "flex", flexDirection: "column", gap: 10, marginBottom: 12 }}>
          <label className="field">
            Название
            <input required autoFocus value={title} onChange={(e) => setTitle(e.target.value)} />
          </label>
          <label className="field">
            Политика
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
          <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
            <button type="button" className="btn ghost" disabled={create.isPending} onClick={() => setOpen(false)}>
              Отмена
            </button>
            <button type="submit" className="btn primary" disabled={create.isPending || !title.trim()}>
              {OBSERVATION_TEXT.create}
            </button>
          </div>
        </form>
      )}
      {query.isSuccess && cases.length === 0 && !open && <div className="muted">{OBSERVATION_TEXT.empty}</div>}
      {cases.length > 0 && (
        <ul style={{ display: "flex", flexDirection: "column", gap: 8, listStyle: "none", margin: 0, padding: 0 }}>
          {cases.map((c) => (
            <li key={c.id}>
              <Link className="artifact" to={`/observation/${c.id}`}>
                <span className="mono">{c.key}</span>
                <span className="nm">{c.title}</span>
                <span className="who">{caseState(c)}</span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
