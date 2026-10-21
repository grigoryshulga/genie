// «Наблюдения» — the observation cases of the project (docs/platform/observation-plane.md §17):
// a filtered table, each row linking to the case's page. Creating a case is in the task card.

import { Link, useSearchParams } from "react-router";
import {
  FILTER_LABEL,
  isActive,
  type ListFilter,
  type ObservationCase,
  OBSERVATION_TEXT,
  POLICY_LABEL,
  RESULT_LABEL,
  caseDot,
  STATUS_LABEL,
  useObservationCases,
  useObservationIncidents,
} from "@/entities/observation";
import "@/shared/ui/agents.css";
import { LoadingState } from "@/shared/ui";
import { TaskLinks, when } from "./parts.tsx";
import "./observations.css";

/** The most cases the list endpoint returns (`limit` tops out at 200, §15). */
const LIST_LIMIT = 200;

const FILTERS: ListFilter[] = ["active", "concluded", "all"];

function inFilter(c: ObservationCase, filter: ListFilter): boolean {
  if (filter === "all") return true;
  return isActive(c.status) === (filter === "active");
}

export function ObservationsPage() {
  const [sp, setSp] = useSearchParams();
  const filter: ListFilter = FILTERS.find((f) => f === sp.get("state")) ?? "active";
  // The filter is applied here: the endpoint takes one status at a time, and «active» is several.
  const cases = useObservationCases({ limit: LIST_LIMIT });
  const incidents = useObservationIncidents("open");
  const openByCase = new Map<number, number>();
  for (const i of incidents.data ?? []) openByCase.set(i.caseId, (openByCase.get(i.caseId) ?? 0) + 1);
  const shown = cases.data?.filter((c) => inFilter(c, filter));
  const choose = (id: ListFilter) => setSp(id === "active" ? {} : { state: id });

  return (
    <main className="main">
      <header className="topbar obs-topbar">
        <h1>{OBSERVATION_TEXT.nav}</h1>
        <span className="sub d-only">что дали выполненные задачи и как они работают после выпуска</span>
        <span className="grow" />
        <div className="seg" role="group" aria-label="Какие наблюдения показать">
          {FILTERS.map((f) => (
            <button key={f} type="button" className={filter === f ? "on" : undefined} aria-pressed={filter === f} onClick={() => choose(f)}>
              {FILTER_LABEL[f]}
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
        {cases.isPending && <LoadingState label="Загрузка наблюдений" />}
        {cases.isError && (
          <p className="obs-error" role="alert">
            Не удалось загрузить наблюдения: {cases.error.message}
          </p>
        )}
        {cases.isSuccess && !cases.data.length && <div className="empty">Наблюдений пока нет. Их создают из карточки задачи.</div>}
        {cases.isSuccess && cases.data.length > 0 && !shown?.length && <div className="empty">Ничего не найдено</div>}
        {shown && shown.length > 0 && (
          <>
            <div className="ag-table-wrap">
              <table className="ag-table" aria-label={OBSERVATION_TEXT.nav}>
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
                          <Link className="obs-table-cell-link" to={`/observation/${c.id}`}>
                            {c.title}
                          </Link>
                        </td>
                        <td>{POLICY_LABEL[c.policy]}</td>
                        <td>
                          <span className="obs-status">
                            <span className={`state-dot ${caseDot(c)}`} aria-hidden="true" />
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
            {(cases.data?.length ?? 0) >= LIST_LIMIT && <p className="obs-note">Показаны последние {LIST_LIMIT}</p>}
          </>
        )}
      </div>
    </main>
  );
}
