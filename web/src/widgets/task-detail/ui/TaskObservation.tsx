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
import { canWriteProject, useSession } from "@/entities/session";
import type { Task } from "@/entities/task";
import { Button, Skeleton, useToast } from "@/shared/ui";
import { CollapsibleSection } from "./CollapsibleSection.tsx";

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
  // The same writer rule as TaskManager (canWriteProject): a member, an admin or an owner, or a local session.
  const writable = canWriteProject(useSession().data);
  const [formOpen, setFormOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [policy, setPolicy] = useState<ObservationPolicy>(DEFAULT_POLICY);

  const startForm = () => {
    setTitle(newCaseTitle(task.title));
    setPolicy(DEFAULT_POLICY);
    setFormOpen(true);
  };

  const submit = (e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const name = title.trim();
    if (!name) return;
    create.mutate(
      { title: name, policy, tasks: [task.id] },
      {
        onSuccess: (view) => {
          setFormOpen(false);
          navigate(`/observation/${view.case.id}`);
        },
        onError: (err) => toast(`Не удалось: ${err.message}`, "error"),
      },
    );
  };

  const cases = query.data ?? [];

  return (
    <CollapsibleSection
      title={OBSERVATION_TEXT.section}
      badge={query.isSuccess && cases.length > 0 ? <span className="n">{cases.length}</span> : undefined}
      action={writable && !formOpen ? (
          <Button variant="ghost" sm onClick={startForm}>
            {OBSERVATION_TEXT.create}
          </Button>
        ) : undefined}
    >
      {query.isPending && <Skeleton height={32} />}
      {query.isError && <p role="alert">{query.error.message}</p>}
      {formOpen && (
        <form aria-label={OBSERVATION_TEXT.create} onSubmit={submit} className="td-form">
          <label className="td-field">
            Название
            <input className="g-input" required autoFocus value={title} onChange={(e) => setTitle(e.target.value)} />
          </label>
          <label className="td-field">
            Политика
            <select
              className="g-select"
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
          <div className="td-form-actions">
            <Button variant="ghost" sm disabled={create.isPending} onClick={() => setFormOpen(false)}>
              Отмена
            </Button>
            <Button type="submit" variant="secondary" sm disabled={create.isPending || !title.trim()}>
              {OBSERVATION_TEXT.create}
            </Button>
          </div>
        </form>
      )}
      {query.isSuccess && cases.length === 0 && !formOpen && <div className="muted">{OBSERVATION_TEXT.empty}</div>}
      {cases.length > 0 && (
        <ul className="td-cases">
          {cases.map((c) => (
            <li key={c.id}>
              <Link className="artifact line" to={`/observation/${c.id}`}>
                <span className="mono">{c.key}</span>
                <span className="nm">{c.title}</span>
                <span className="who">{caseState(c)}</span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </CollapsibleSection>
  );
}
