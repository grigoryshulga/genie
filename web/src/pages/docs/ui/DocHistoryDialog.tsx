// Page history: the versions of a page on main, one version rendered at its
// commit, a unified diff of any two, and a restore that publishes a new
// changeset. The merge agent's commits carry their badge from the trailer.

import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router";
import type { DocReadResult, CommitInfo } from "@/entities/doc";
import {
  commitDate,
  commitStamp,
  commitTask,
  docActionError,
  isAgentMergedCommit,
  useDocDiff,
  useDocHistory,
  useDocPage,
  useRestoreDocVersion,
} from "@/entities/doc";
import { Button, ConfirmDialog, Dialog, EmptyState, ErrorState, LoadingState, Markdown, Segmented, Select, Tag, useToast } from "@/shared/ui";
import { DocDiff } from "./DocDiff.tsx";

const failure = (error: unknown) =>
  docActionError({ status: (error as { status?: number } | undefined)?.status, message: error instanceof Error ? error.message : String(error) });

/** Frontmatter shown apart from the body, so a version reads like the page. */
function VersionText({ text }: { text: string }) {
  const match = /^---\n([\s\S]*?)\n---\n?/.exec(text);
  return (
    <>
      {match && <pre className="doc-version-frontmatter">{match[1]}</pre>}
      <Markdown text={match ? text.slice(match[0].length) : text} />
    </>
  );
}

function older(history: CommitInfo[], sha: string): CommitInfo | undefined {
  return history[history.findIndex((commit) => commit.sha === sha) + 1];
}

export function DocHistoryDialog({ page, onClose }: { page: DocReadResult; onClose: () => void }) {
  const history = useDocHistory(page.path);
  const restore = useRestoreDocVersion();
  const toast = useToast();
  const [mode, setMode] = useState<"version" | "compare">("version");
  const [to, setTo] = useState<string>();
  const [from, setFrom] = useState<string>();
  const [confirming, setConfirming] = useState(false);

  const commits = useMemo(() => history.data ?? [], [history.data]);
  useEffect(() => {
    if (!commits.length || to) return;
    setTo(commits[0].sha);
    if (commits[1]) setFrom(commits[1].sha);
  }, [commits, to]);

  const version = useDocPage(page.path, to ? { at: to } : undefined);
  const diff = useDocDiff(page.path, from, to);
  const selected = commits.find((commit) => commit.sha === to);

  const doRestore = () => {
    if (!to) return;
    restore.mutate({ path: page.path, at: to }, {
      onSuccess: () => {
        setConfirming(false);
        toast("Версия восстановлена новой публикацией");
        onClose();
      },
      onError: (error) => {
        setConfirming(false);
        toast(failure(error), "error");
      },
    });
  };

  return (
    <Dialog
      title={`История версий: ${page.title}`}
      wide
      onClose={onClose}
      footer={
        <>
          <span className="doc-history-foot g-muted">
            {commits.length ? `${commits.length} версий на основной ветке` : "История читается из git"}
          </span>
          <span className="grow" />
          {to && (
            <Button variant="primary" onClick={() => setConfirming(true)} busy={restore.isPending}>
              Восстановить эту версию
            </Button>
          )}
        </>
      }
    >
      {history.isPending ? (
        <LoadingState label="Загружаем историю" lines={4} />
      ) : history.isError ? (
        <ErrorState message={`Не удалось прочитать историю: ${failure(history.error)}`} onRetry={() => void history.refetch()} />
      ) : !commits.length ? (
        <EmptyState title="Версий пока нет">История появится после первой публикации страницы.</EmptyState>
      ) : (
        <div className="doc-history">
          <ol className="doc-history-list" aria-label="Версии страницы">
            {commits.map((commit) => (
              <li key={commit.sha}>
                <button type="button" className="doc-history-row" aria-selected={commit.sha === to} onClick={() => { setTo(commit.sha); setFrom(older(commits, commit.sha)?.sha); setMode("version"); }}>
                  <span className="doc-history-when g-mono">{commitStamp(commit.date)}</span>
                  <span className="doc-history-subject">{commit.subject}</span>
                  <span className="doc-history-meta">
                    <span>{commit.authorName}</span>
                    {commitTask(commit) && <span className="g-mono">{commitTask(commit)}</span>}
                    {isAgentMergedCommit(commit) && <Tag tone="signal">свёл агент</Tag>}
                  </span>
                  <span className="doc-history-sha g-mono">{commit.sha.slice(0, 8)}</span>
                </button>
              </li>
            ))}
          </ol>
          <section className="doc-history-pane" aria-label="Версия">
            <div className="doc-history-tools">
              <Segmented
                label="Как показать версию"
                value={mode}
                onChange={setMode}
                items={[
                  { id: "version", label: "Версия" },
                  { id: "compare", label: "Сравнить" },
                ]}
              />
              {mode === "compare" && (
                <div className="doc-history-compare">
                  <Select sm aria-label="Версия «от»" value={from ?? ""} onChange={(event) => setFrom(event.target.value)}>
                    {commits.map((commit) => (
                      <option key={commit.sha} value={commit.sha}>
                        {commitDate(commit.date)} · {commit.subject}
                      </option>
                    ))}
                  </Select>
                  <span className="g-muted">→</span>
                  <Select sm aria-label="Версия «до»" value={to ?? ""} onChange={(event) => setTo(event.target.value)}>
                    {commits.map((commit) => (
                      <option key={commit.sha} value={commit.sha}>
                        {commitDate(commit.date)} · {commit.subject}
                      </option>
                    ))}
                  </Select>
                </div>
              )}
            </div>
            {mode === "version" ? (
              version.isPending ? (
                <p className="doc-hint">Загружаем версию…</p>
              ) : version.isError ? (
                <p className="doc-hint">Не удалось открыть версию: {failure(version.error)}</p>
              ) : (
                <div className="doc-history-version">
                  {selected && (
                    <p className="doc-history-version-head">
                      <span className="g-mono">{selected.sha.slice(0, 8)}</span>
                      <span>{selected.authorName}</span>
                      <span>{commitStamp(selected.date)}</span>
                      {isAgentMergedCommit(selected) && <Tag tone="signal">свёл агент</Tag>}
                    </p>
                  )}
                  <VersionText text={version.data?.content ?? ""} />
                </div>
              )
            ) : diff.isPending ? (
              <p className="doc-hint">Считаем разницу…</p>
            ) : diff.isError ? (
              <p className="doc-hint">Не удалось сравнить версии: {failure(diff.error)}</p>
            ) : from === to ? (
              <p className="doc-hint">Выберите две разные версии.</p>
            ) : (
              <DocDiff diff={diff.data} empty="Между этими версиями нет текстовых изменений" />
            )}
          </section>
        </div>
      )}

      {confirming && (
        <ConfirmDialog
          title="Восстановить эту версию?"
          confirmLabel="Восстановить"
          busy={restore.isPending}
          onClose={() => setConfirming(false)}
          onConfirm={doRestore}
        >
          Содержимое страницы на выбранном коммите будет опубликовано новой правкой. Её можно будет откатить так же, как любую публикацию.{" "}
          {to && <Link to={`/docs/changes?path=${encodeURIComponent(page.path)}`}>Посмотреть изменения страницы</Link>}
        </ConfirmDialog>
      )}
    </Dialog>
  );
}
