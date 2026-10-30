// The fallback for a pending merge: ours, theirs and the result editor, one
// conflicted page at a time. Saving stages every page and publishes the merge.

import { useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import {
  hasConflictMarkers,
  MERGE_STATUS_NAME,
  useDocMerge,
  useResolveDocMerge,
  type MergePageView,
} from "@/entities/doc";
import { Button, Chip, EmptyState, ErrorState, Icon, LoadingState, Tag, Textarea, useToast } from "@/shared/ui";
import { failure } from "./changes-shared.ts";
import "./changes.css";

export function DocsMergePage() {
  const { mergeId } = useParams();
  const id = Number(mergeId);
  const navigate = useNavigate();
  const toast = useToast();
  const query = useDocMerge(Number.isFinite(id) ? id : undefined);
  const resolve = useResolveDocMerge();
  const merge = query.data?.merge;
  const pages = useMemo(() => query.data?.pages ?? [], [query.data]);
  const [active, setActive] = useState<string>();
  const [edits, setEdits] = useState<Record<string, string>>({});

  useEffect(() => {
    if (!pages.length || active) return;
    setActive(pages[0].path);
  }, [pages, active]);

  const page = pages.find((item) => item.path === active) ?? pages[0];
  const text = page ? edits[page.path] ?? page.marked : "";
  const marker = hasConflictMarkers(text);

  const save = () => {
    if (!merge) return;
    resolve.mutate(
      { id: merge.id, done: true, pages: pages.map((item: MergePageView) => ({ path: item.path, content: edits[item.path] ?? item.marked })) },
      {
        onSuccess: () => {
          toast("Слияние завершено и опубликовано");
          navigate(`/docs/changes?view=all&changeset=${merge.changeset}`);
        },
        onError: (error) => toast(failure(error), "error"),
      },
    );
  };

  return (
    <main className="main">
      <header className="changes-merge-head">
        <div>
          <p className="g-mono g-muted">Слияние #{mergeId}</p>
          <h1>Свести правки</h1>
        </div>
        <span className="grow" />
        <Link className="g-btn secondary" to="/docs/changes">
          <Icon.back size={13} />
          К изменениям
        </Link>
        {merge && merge.status !== "resolved" && (
          <Button variant="primary" onClick={save} busy={resolve.isPending} disabled={marker}>
            Сохранить и опубликовать
          </Button>
        )}
      </header>

      <div className="scroll changes-merge-body">
        {query.isPending ? (
          <LoadingState label="Загружаем слияние" lines={5} />
        ) : query.isError ? (
          <ErrorState message={`Не удалось открыть слияние: ${failure(query.error)}`} onRetry={() => void query.refetch()} />
        ) : !merge ? (
          <EmptyState title="Слияние не найдено">Возможно, его уже завершили.</EmptyState>
        ) : merge.status === "resolved" ? (
          <EmptyState title="Слияние уже завершено">
            {merge.resolution ? <span className="g-mono">{merge.resolution.slice(0, 12)}</span> : "Правки опубликованы."}{" "}
            <Link to={`/docs/changes?view=all&changeset=${merge.changeset}`}>Открыть изменение</Link>
          </EmptyState>
        ) : (
          <>
            <section className="changes-merge-meta" aria-label="Состояние слияния">
              <Tag tone={merge.status === "failed" ? "danger" : "accent"}>{MERGE_STATUS_NAME[merge.status]}</Tag>
              <span>Ветка <span className="g-mono">{merge.tip.slice(0, 8)}</span> и основная версия <span className="g-mono">{merge.main.slice(0, 8)}</span>.</span>
              <span>Попыток: {merge.attempts}</span>
              {merge.error && <span className="changes-merge-error">{merge.error}</span>}
              <Link to={`/docs/changes?view=all&changeset=${merge.changeset}`}>Изменение #{merge.changeset}</Link>
            </section>

            {pages.length > 1 && (
              <div className="changes-merge-pages" role="group" aria-label="Страницы со слиянием">
                {pages.map((item) => (
                  <Chip key={item.path} pressed={item.path === page?.path} onClick={() => setActive(item.path)}>
                    <span className="g-mono">{item.path}</span>
                  </Chip>
                ))}
              </div>
            )}

            {page ? (
              <>
                <div className="changes-merge-side">
                  {query.data?.ours && <span>{query.data.ours.authorName ?? query.data.ours.author}{query.data.ours.task ? ` · ${query.data.ours.task}` : ""}{query.data.ours.note ? ` · ${query.data.ours.note}` : ""}</span>}
                  {query.data?.theirs && <span>{query.data.theirs.authorName ?? query.data.theirs.author}{query.data.theirs.task ? ` · ${query.data.theirs.task}` : ""}{query.data.theirs.note ? ` · ${query.data.theirs.note}` : ""}</span>}
                </div>
                <div className="changes-merge-panes">
                  <section className="changes-merge-pane" aria-label="Основная версия">
                    <h2>На основной ветке</h2>
                    <pre>{page.ours || "Пусто"}</pre>
                  </section>
                  <section className="changes-merge-pane" aria-label="Версия в ветке">
                    <h2>В черновике</h2>
                    <pre>{page.theirs || "Пусто"}</pre>
                  </section>
                  <section className="changes-merge-pane result" aria-label="Результат слияния">
                    <h2>Результат</h2>
                    <Textarea
                      aria-label={`Результат слияния страницы ${page.path}`}
                      className="changes-merge-editor g-mono"
                      rows={28}
                      value={text}
                      onChange={(event) => setEdits({ ...edits, [page.path]: event.target.value })}
                    />
                    {marker && <p className="changes-merge-warning">В тексте остались маркеры конфликта: сервер не примет такую публикацию.</p>}
                  </section>
                </div>
                {page.base && (
                  <details className="changes-merge-base">
                    <summary>Исходная версия до правок</summary>
                    <pre>{page.base}</pre>
                  </details>
                )}
              </>
            ) : (
              <EmptyState title="Нет конфликтующих страниц">Слияние можно завершить.</EmptyState>
            )}
          </>
        )}
      </div>
    </main>
  );
}
