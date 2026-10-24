// A task's delivery: the repositories it works in, its branches and pull/merge requests,
// the checks, and — for people — the merge.

import { useState } from "react";
import { CI_NAME, CR_NAME, type TaskRepo, useMergeRequest, useTaskRepos } from "@/entities/repo";
import { Banner, Button, ConfirmDialog, Tag, useToast } from "@/shared/ui";
import { canWriteProject, useSession } from "@/entities/session";
import { CollapsibleSection } from "./CollapsibleSection.tsx";

function ciTone(ci: TaskRepo["ciState"]): "accent" | "signal" | "danger" | "neutral" {
  return ci === "passed" ? "accent" : ci === "failed" ? "danger" : ci === "pending" || ci === "stalled" ? "signal" : "neutral";
}

// The checks of a delivery, whenever they mean something: any recorded state (with or without a
// request — a policy may ask for none at all), and "no checks" for an open request, as before.
// The checks of the target branch appear here once the request was merged.
function ciPill(r: TaskRepo) {
  const known = r.ciState ?? (r.crState === "open" ? "none" : null);
  if (!known) return null;
  const of = r.ciRef && r.ciRef !== r.branch ? ` ${r.ciRef}` : "";
  return (
    <Tag tone={ciTone(known)} title={`проверки${of}`}>
      {CI_NAME[known]}
    </Tag>
  );
}

export function TaskDelivery({ task }: { task: string }) {
  const rows = useTaskRepos(task).data?.repos ?? [];
  const merge = useMergeRequest();
  const toast = useToast();
  const [merging, setMerging] = useState<TaskRepo>();
  const writable = canWriteProject(useSession().data);
  if (!rows.length) return null;
  return (
    <CollapsibleSection title="Репозитории" badge={<span className="n">{rows.length}</span>}>
      <div className="td-delivery">
        {rows.map((r) => (
          <div key={r.repo} className={`td-repo${r.ciState === "failed" ? " failed" : ""}`}>
            {r.ciState === "failed" && (
              <Banner tone="danger" title="Проверки упали">
                В {r.repo}{r.crNumber ? `, запрос #${r.crNumber}` : r.branch ? `, ветка ${r.branch}` : ""}: слияние не пройдёт, пока проверки красные.
              </Banner>
            )}
            <div className="line">
            <b>{r.repo}</b>
            <Tag tone="outline">{r.access === "write" ? "запись" : "чтение"}</Tag>
            {r.branch && <span className="mono">{r.branch}</span>}
            {r.crNumber ? (
              <>
                {r.crUrl ? (
                  <a href={r.crUrl} target="_blank" rel="noreferrer">
                    запрос #{r.crNumber}
                  </a>
                ) : (
                  <span>запрос #{r.crNumber}</span>
                )}
                <Tag tone={r.crState === "merged" ? "accent" : r.crState === "closed" ? "signal" : "outline"}>{CR_NAME[r.crState ?? "open"]}</Tag>
                {ciPill(r)}
                {r.crState === "open" && (
                  writable && (
                    <Button variant="secondary" sm onClick={() => setMerging(r)}>
                      Слить
                    </Button>
                  )
                )}
              </>
            ) : r.state === "published" ? (
              <>
                <span className="muted">ветка отправлена, запроса ещё нет</span>
                {ciPill(r)}
              </>
            ) : r.access === "write" ? (
              <span className="muted">пока без изменений на хостинге</span>
            ) : null}
            </div>
          </div>
        ))}
      </div>
      {merging && (
        <ConfirmDialog
          title={`Слить запрос #${merging.crNumber}?`}
          confirmLabel="Слить"
          busy={merge.isPending}
          onClose={() => setMerging(undefined)}
          onConfirm={() =>
            merge.mutate(
              { task, repo: merging.repo },
              {
                onSuccess: () => {
                  toast("Запрос слит");
                  setMerging(undefined);
                },
                onError: (e) => toast(e.message, "error"),
              },
            )
          }
        >
          Запрос слит на хостинге от имени бота genie. Как человек вы не связаны условиями «нужны проверки и одобрения» — но правила защиты веток на хостинге по-прежнему действуют.
        </ConfirmDialog>
      )}
    </CollapsibleSection>
  );
}
