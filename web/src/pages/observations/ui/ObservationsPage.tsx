// «Наблюдения» — the observation cases of the project (docs/platform/observation-plane.md §17):
// a filtered table, each row linking to the case's page. Creating a case is in the task card.

import { Link, useSearchParams } from "react-router";
import {
  isActive,
  OBSERVATION_TEXT,
  POLICY_LABEL,
  RESULT_LABEL,
  STATUS_LABEL,
  type CaseStatus,
  type ObservationCase,
  useObservationCases,
  useObservationIncidents,
} from "@/entities/observation";
import { clock, dayLabel } from "@/shared/lib";

/** The filters of the list. «Завершённые» is every case that is no longer active, cancelled ones included. */
const FILTERS = [
  { id: "active", label: "Активные" },
  { id: "concluded", label: "Завершённые" },
  { id: "all", label: "Все" },
] as const;
type Filter = (typeof FILTERS)[number]["id"];

/** The dot before a status: the same colours the other lists use for the same meaning. */
export const STATUS_DOT: Record<CaseStatus, string> = {
  proposed: "",
  waiting: "waiting",
  observing: "running",
  evaluating: "running",
  concluded: "ok",
  cancelled: "off",
};

/** A timestamp as «сегодня, 14:05» or «18 октября, 14:05»; an em dash when there is none. */
export function when(iso?: string): string {
  return iso ? `${dayLabel(iso)}, ${clock(iso)}` : "—";
}

/** Links to the tasks of a case or an incident; an em dash for none. */
export function TaskLinks({ ids }: { ids: string[] }) {
  if (!ids.length) return <>—</>;
  return (
    <span className="obs-tasks">
      {ids.map((id) => (
        <Link key={id} className="mono" to={`/task/${encodeURIComponent(id)}`}>
          {id}
        </Link>
      ))}
    </span>
  );
}

function inFilter(c: ObservationCase, filter: Filter): boolean {
  if (filter === "all") return true;
  return isActive(c.status) === (filter === "active");
}

export function ObservationsPage() {
  const [sp, setSp] = useSearchParams();
  const filter: Filter = FILTERS.find((f) => f.id === sp.get("state"))?.id ?? "active";
  // The list endpoint takes at most 200 cases; the filter is applied here, so every case is one query.
  const cases = useObservationCases({ limit: 200 });
  const incidents = useObservationIncidents("open");
  const openByCase = new Map<number, number>();
  for (const i of incidents.data ?? []) openByCase.set(i.caseId, (openByCase.get(i.caseId) ?? 0) + 1);
  const shown = cases.data?.filter((c) => inFilter(c, filter));
  const choose = (id: Filter) => setSp(id === "active" ? {} : { state: id });

  return (
    <main className="main">
      <header className="topbar">
        <h1>{OBSERVATION_TEXT.nav}</h1>
        <span className="sub d-only">что дали поставленные задачи и как они работают после выпуска</span>
        <span className="grow" />
        <div className="seg" role="group" aria-label="Какие наблюдения показать">
          {FILTERS.map((f) => (
            <button key={f.id} type="button" className={filter === f.id ? "on" : undefined} aria-pressed={filter === f.id} onClick={() => choose(f.id)}>
              {f.label}
            </button>
          ))}
        </div>
      </header>
      <div className="scroll obs-body">
        {incidents.isError && (
          <p className="obs-error" role="alert">
            Не удалось загрузить открытые инциденты: {incidents.error.message}
          </p>
        )}
        {cases.isPending && <div className="empty">Загрузка…</div>}
        {cases.isError && (
          <p className="obs-error" role="alert">
            Не удалось загрузить наблюдения: {cases.error.message}
          </p>
        )}
        {cases.isSuccess && !cases.data.length && <div className="empty">Наблюдений пока нет. Их создают из карточки задачи.</div>}
        {cases.isSuccess && cases.data.length > 0 && !shown?.length && <div className="empty">Ничего не найдено</div>}
        {shown && shown.length > 0 && (
          <div className="obs-table-wrap">
            <table className="obs-table" aria-label="Наблюдения проекта">
              <thead>
                <tr>
                  <th scope="col">Ключ</th>
                  <th scope="col">Название</th>
                  <th scope="col">Политика</th>
                  <th scope="col">Статус</th>
                  <th scope="col">Задачи</th>
                  <th scope="col">Следующая оценка</th>
                  <th scope="col">Открытые инциденты</th>
                </tr>
              </thead>
              <tbody>
                {shown.map((c) => {
                  const open = openByCase.get(c.id) ?? 0;
                  return (
                    <tr key={c.id}>
                      <td className="mono">{c.key}</td>
                      <td>
                        <Link to={`/observation/${c.id}`}>{c.title}</Link>
                      </td>
                      <td>{POLICY_LABEL[c.policy]}</td>
                      <td>
                        <span className="obs-status">
                          <span className={`state-dot ${STATUS_DOT[c.status]}`} aria-hidden="true" />
                          {STATUS_LABEL[c.status]}
                          {c.result && ` · ${RESULT_LABEL[c.result]}`}
                        </span>
                      </td>
                      <td>
                        <TaskLinks ids={c.tasks} />
                      </td>
                      <td>{when(c.nextEvaluationAt)}</td>
                      <td>{open ? <span className="obs-open">{open}</span> : "—"}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </main>
  );
}
