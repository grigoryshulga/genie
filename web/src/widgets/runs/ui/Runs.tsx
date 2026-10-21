// The runs of an automation (or of every automation): the journal of what the
// rules did, step by step, with a way to cancel a live run. Used by the work
// automations page and by the settings one.

import { useState } from "react";
import { RUN_STATUS, useRun, useRuns } from "@/entities/platform";
import { request } from "@/shared/api";
import { timeAgo, useTick } from "@/shared/lib";
import { Button, LoadingState, useRefreshingAct } from "@/shared/ui";

export function Runs({ automation }: { automation?: number }) {
  useTick();
  const runs = useRuns(automation);
  const [open, setOpen] = useState<number>();
  const run = useRun(open);
  const act = useRefreshingAct();
  return (
    <section>
      <h3>{automation ? "Последние запуски" : "Запуски всех правил"}</h3>
      {!runs.data?.length ? (
        runs.isPending ? (
          <LoadingState label="Загрузка запусков" lines={2} />
        ) : (
          <p className="muted" style={{ margin: 0 }}>
            Запусков пока не было.
          </p>
        )
      ) : (
        <div className="runs">
          {runs.data.map((r) => (
            <div key={r.id} className="run">
              <button type="button" className="run-row" aria-expanded={open === r.id} onClick={() => setOpen(open === r.id ? undefined : r.id)}>
                <span className={`st ${r.status}`}>
                  {r.status === "running" ? <span className="spin" /> : <span className={`state-dot ${r.status}`} />}
                  {RUN_STATUS[r.status] ?? r.status}
                </span>
                <span className="id">#{r.id}</span>
                <span className="nt">{r.error ?? triggerNote(r.triggerKey)}</span>
                <span className="when">{timeAgo(r.started)}</span>
              </button>
              {open === r.id && run.data && (
                <ol className="run-steps">
                  {run.data.steps.map((s) => (
                    <li key={s.id}>
                      <details>
                        <summary>
                          <span className="sid">{s.stepId}</span>
                          <span>
                            {s.kind} · {RUN_STATUS[s.status] ?? s.status}
                            {s.attempt > 1 ? ` · попытка ${s.attempt}` : ""}
                            {s.error && <span className="err"> · {s.error}</span>}
                          </span>
                        </summary>
                        <pre className="view">{JSON.stringify({ input: s.input, output: s.output, wait: s.wait }, null, 2)}</pre>
                      </details>
                    </li>
                  ))}
                  {["queued", "running", "waiting"].includes(run.data.status) && (
                    <li>
                      <Button onClick={() => void act(() => request("POST", `/api/runs/${r.id}/cancel`), "Отменено")}>
                        Отменить запуск
                      </Button>
                    </li>
                  )}
                </ol>
              )}
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

function triggerNote(key: string): string {
  const kind = key.split(":")[0];
  if (kind === "event") return `по событию #${key.split(":").at(-1)}`;
  return ({ schedule: "по расписанию", manual: "запущено вручную", webhook: "по webhook" } as Record<string, string>)[kind] ?? key;
}
