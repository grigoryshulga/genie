// The Changes feed: open drafts, unreviewed publications, agent-resolved merges and
// pending merges in one list, with the selected changeset's diff and actions.

import { useMemo, useState } from "react";
import { Link, useSearchParams } from "react-router";
import type { Changeset } from "@/entities/doc";
import {
  canPublishChangeset,
  canReviewChangeset,
  changesFilterFromParams,
  changesFilterOptions,
  changesFilterToParams,
  changesetAuthor,
  changesetPathsLabel,
  changesetStatusName,
  CHANGES_VIEWS,
  commitStamp,
  diffStats,
  filterChangesets,
  isAgentMergedChangeset,
  isAgentMergedCommit,
  isUnreviewedChangeset,
  MERGE_STATUS_NAME,
  normalizeChangesetDetail,
  parseUnifiedDiff,
  useChangeset,
  useChangesets,
  useDiscardChangeset,
  usePublishChangeset,
  useReviewChangeset,
  useRevertChangeset,
  type ChangesView,
} from "@/entities/doc";
import { useSession } from "@/entities/session";
import { DocViewSwitch } from "@/widgets/docs-view-switch";
import { Banner, Button, ConfirmDialog, EmptyState, ErrorState, Icon, PageHeader, Segmented, Select, Skeleton, Tag, useToast } from "@/shared/ui";
import { DocDiff } from "./DocDiff.tsx";
import { changesetTitle, failure } from "./changes-shared.ts";
import "./changes.css";

export function DocsChangesPage() {
  const [sp, setSp] = useSearchParams();
  const toast = useToast();
  const session = useSession();
  const filter = changesFilterFromParams(sp);
  const all = useChangesets({ view: "all" });
  const changesets = useMemo(() => all.data ?? [], [all.data]);
  // Merging changesets have their own block at the top, so they are not listed twice.
  const pending = changesets.filter((changeset) => changeset.status === "merging");
  const rows = filterChangesets(changesets, filter).filter((changeset) => changeset.status !== "merging");
  const selectedId = Number(sp.get("changeset") ?? "") || rows[0]?.id || pending[0]?.id;
  const detail = useChangeset(selectedId);
  const changeset = detail.data?.changeset;
  const role = session.data?.projects.find((project) => project.slug === session.data?.project)?.role;
  const login = session.data?.user.login;

  const publish = usePublishChangeset();
  const discard = useDiscardChangeset();
  const review = useReviewChangeset();
  const revert = useRevertChangeset();
  const [confirm, setConfirm] = useState<"discard" | "revert" | null>(null);
  const [merging, setMerging] = useState<{ id: number } | null>(null);

  const select = (id: number) => {
    const next = new URLSearchParams(sp);
    next.set("changeset", String(id));
    setSp(next, { replace: true });
  };

  const updateFilter = (patch: Partial<typeof filter>) => {
    const next = changesFilterToParams({ ...filter, ...patch });
    next.delete("changeset");
    setSp(next);
  };

  const options = changesFilterOptions(changesets);
  const counts: Record<ChangesView, number> = {
    open: changesets.filter((item) => item.status === "open").length,
    unreviewed: changesets.filter(isUnreviewedChangeset).length,
    agent: changesets.filter(isAgentMergedChangeset).length,
    all: changesets.length,
  };

  const publishSelected = () => {
    if (!selectedId) return;
    setMerging(null);
    publish.mutate(selectedId, {
      onSuccess: (result) => {
        if (result.status === "merging") {
          setMerging({ id: result.merge.id });
          toast("Правки пересеклись с более новыми — их сводит агент");
        } else {
          toast("Изменения опубликованы");
        }
      },
      onError: (error) => toast(failure(error), "error"),
    });
  };
  const discardSelected = () => {
    if (!selectedId) return;
    discard.mutate(selectedId, {
      onSuccess: () => {
        setConfirm(null);
        toast("Черновик отменён");
      },
      onError: (error) => {
        setConfirm(null);
        toast(failure(error), "error");
      },
    });
  };
  const reviewSelected = () => {
    if (!selectedId) return;
    review.mutate(selectedId, {
      onSuccess: () => toast("Изменение отмечено проверенным"),
      onError: (error) => toast(failure(error), "error"),
    });
  };
  const revertSelected = () => {
    if (!selectedId) return;
    revert.mutate(selectedId, {
      onSuccess: () => {
        setConfirm(null);
        toast("Изменение откачено новой публикацией");
      },
      onError: (error) => {
        setConfirm(null);
        toast(failure(error), "error");
      },
    });
  };

  return (
    <main className="main">
      <PageHeader title="Изменения" toolbar={<DocViewSwitch />} />
      <div className="changes-filters" role="group" aria-label="Фильтры изменений">
        <Segmented
          label="Какие изменения показать"
          value={filter.view}
          onChange={(view) => updateFilter({ view })}
          items={CHANGES_VIEWS.map((view) => ({ id: view.id, label: `${view.label}${counts[view.id] ? ` ${counts[view.id]}` : ""}` }))}
        />
        <div className="changes-filter-selects">
          <Select sm aria-label="Задача" value={filter.task ?? ""} onChange={(event) => updateFilter({ task: event.target.value || undefined })}>
            <option value="">Все задачи</option>
            {options.tasks.map((task) => (
              <option key={task} value={task}>
                {task}
              </option>
            ))}
          </Select>
          <Select sm aria-label="Автор" value={filter.author ?? ""} onChange={(event) => updateFilter({ author: event.target.value || undefined })}>
            <option value="">Все авторы</option>
            {options.authors.map((author) => (
              <option key={author} value={author}>
                {author}
              </option>
            ))}
          </Select>
          {(filter.task || filter.author || filter.path) && (
            <Button sm onClick={() => updateFilter({ task: undefined, author: undefined, path: undefined })}>
              Сбросить
            </Button>
          )}
        </div>
      </div>

      <div className="split narrow changes-split">
        <section className="split-list" aria-label="Список изменений">
          {all.isPending && (
            <div className="split-skeletons" aria-hidden="true">
              {[0, 1, 2].map((index) => (
                <Skeleton key={index} height={44} />
              ))}
            </div>
          )}
          {all.isError && <Banner tone="danger" title="Не получилось прочитать изменения">{failure(all.error)}</Banner>}
          {pending.length > 0 && (
            <div className="changes-pending">
              <p className="changes-pending-head">Ждут слияния</p>
              {pending.map((changeset) => (
                <button key={changeset.id} type="button" className="g-row split-row" aria-selected={changeset.id === selectedId} onClick={() => select(changeset.id)}>
                  <Icon.merge size={13} />
                  <span className="split-row-text">
                    <span className="split-row-title">{changesetTitle(changeset)}</span>
                    <span className="split-row-sub">Правки пересеклись · {changesetAuthor(changeset)}</span>
                  </span>
                </button>
              ))}
            </div>
          )}
          {!all.isPending && !rows.length && !pending.length && (
            <p className="split-hint">
              {filter.view === "open" ? "Открытых черновиков нет." : filter.view === "unreviewed" ? "Непроверенных публикаций нет." : filter.view === "agent" ? "Сведённых агентом изменений нет." : "Изменений пока нет."}
            </p>
          )}
          {!all.isPending && !rows.length && pending.length > 0 && <p className="split-hint">Других изменений в этом виде нет — ожидающие слияния перечислены выше.</p>}
          {rows.map((changeset) => (
            <button key={changeset.id} type="button" className="g-row split-row" aria-selected={changeset.id === selectedId} onClick={() => select(changeset.id)}>
              <Icon.branch size={13} />
              <span className="split-row-text">
                <span className="split-row-title">
                  {changesetTitle(changeset)}
                  {isUnreviewedChangeset(changeset) && <Tag tone="accent">не проверено</Tag>}
                  {isAgentMergedChangeset(changeset) && <Tag tone="signal">агент</Tag>}
                </span>
                <span className="split-row-sub">
                  {changesetAuthor(changeset)}
                  {changeset.task ? ` · ${changeset.task}` : ""} · {changesetPathsLabel(changeset.paths.length)} · {commitStamp(changeset.updated)}
                </span>
              </span>
            </button>
          ))}
        </section>

        <section className="split-main" aria-label="Изменение">
          {!selectedId ? (
            <div className="pane-empty">
              <Icon.branch size={22} />
              <span>Здесь появятся черновики, публикации и слияния документации</span>
            </div>
          ) : detail.isPending ? (
            <div className="changes-detail" aria-busy="true">
              <Skeleton width="40%" height={28} />
              <Skeleton width="70%" height={16} />
              <Skeleton width="90%" height={16} />
            </div>
          ) : detail.isError ? (
            <div className="changes-detail">
              <ErrorState message={`Не удалось открыть изменение: ${failure(detail.error)}`} onRetry={() => void detail.refetch()} />
            </div>
          ) : changeset && detail.data ? (
            <ChangesetDetail
              changeset={changeset}
              diff={detail.data.diff}
              merge={detail.data.merge}
              canPublish={canPublishChangeset(changeset, login)}
              canReview={canReviewChangeset(role)}
              busy={publish.isPending || discard.isPending || review.isPending || revert.isPending}
              merging={merging}
              onPublish={publishSelected}
              onDiscard={() => setConfirm("discard")}
              onReview={reviewSelected}
              onRevert={() => setConfirm("revert")}
              commits={detail.data.commits}
            />
          ) : null}
        </section>
      </div>

      {confirm && changeset && (
        <ConfirmDialog
          title={confirm === "discard" ? `Отменить черновик #${changeset.id}?` : `Откатить изменение #${changeset.id}?`}
          confirmLabel={confirm === "discard" ? "Отменить изменения" : "Откатить"}
          danger
          busy={discard.isPending || revert.isPending}
          onClose={() => setConfirm(null)}
          onConfirm={confirm === "discard" ? discardSelected : revertSelected}
        >
          {confirm === "discard"
            ? "Ветка закроется без слияния: правки исчезнут из пространства знаний, но останутся в истории git."
            : "Будет создана и опубликована обратная правка. Пересечения с более новыми изменениями сведёт агент."}
        </ConfirmDialog>
      )}
    </main>
  );
}

function ChangesetDetail({
  changeset,
  diff,
  merge,
  canPublish,
  canReview,
  busy,
  merging,
  onPublish,
  onDiscard,
  onReview,
  onRevert,
  commits,
}: {
  changeset: Changeset;
  diff: string;
  merge?: ReturnType<typeof normalizeChangesetDetail>["merge"];
  canPublish: boolean;
  canReview: boolean;
  busy: boolean;
  merging: { id: number } | null;
  onPublish: () => void;
  onDiscard: () => void;
  onReview: () => void;
  onRevert: () => void;
  commits: ReturnType<typeof normalizeChangesetDetail>["commits"];
}) {
  const lines = parseUnifiedDiff(diff);
  const stats = diffStats(lines);
  return (
    <div className="changes-detail">
      <header className="changes-detail-head">
        <div className="changes-detail-title">
          <p className="g-mono g-muted">#{changeset.id} · {changeset.branch}</p>
          <h2>{changesetTitle(changeset)}</h2>
          <p className="changes-detail-meta">
            <Tag tone={changeset.status === "published" ? "signal" : changeset.status === "merging" ? "accent" : "outline"}>{changesetStatusName(changeset.status)}</Tag>
            {isUnreviewedChangeset(changeset) && <Tag tone="accent">Не проверено</Tag>}
            {changeset.origin === "merge" || changeset.origin === "revert" || changeset.origin === "restore" ? <Tag tone="outline">{originName(changeset.origin)}</Tag> : null}
            <span>{changesetAuthor(changeset)}</span>
            {changeset.task && <Link className="epic-chip" to={`/tasks?task=${encodeURIComponent(changeset.task)}`}>{changeset.task}</Link>}
            <span className="g-muted">{commitStamp(changeset.created)}</span>
          </p>
        </div>
        <div className="changes-detail-actions">
          {canPublish && changeset.status === "open" && (
            <>
              <Button onClick={onDiscard} busy={busy}>
                Отменить
              </Button>
              <Button variant="primary" onClick={onPublish} busy={busy}>
                Опубликовать
              </Button>
            </>
          )}
          {canReview && isUnreviewedChangeset(changeset) && (
            <Button variant="primary" onClick={onReview} busy={busy}>
              <Icon.check size={13} />
              Отметить проверенным
            </Button>
          )}
          {canReview && changeset.status === "published" && (
            <Button variant="danger" onClick={onRevert} busy={busy}>
              Откатить
            </Button>
          )}
        </div>
      </header>

      {merging && <Banner tone="accent" title="Правки пересеклись с более новыми — их сводит агент">Следите за слиянием: <Link to={`/docs/changes/merge/${merging.id}`}>открыть слияние</Link>.</Banner>}
      {changeset.status === "merging" && !merging && (
        <Banner tone="accent" title="Правки пересеклись с более новыми">
          Слияние ведёт агент.{" "}
          {merge ? <Link to={`/docs/changes/merge/${merge.id}`}>Открыть слияние</Link> : "Результат появится в этом списке."}
        </Banner>
      )}
      {merge && merge.status !== "resolved" && (
        <p className="changes-merge-line">
          Слияние #{merge.id}: <b>{MERGE_STATUS_NAME[merge.status]}</b>
          {merge.error ? ` · ${merge.error}` : ""} <Link to={`/docs/changes/merge/${merge.id}`}>Открыть</Link>
        </p>
      )}

      <section className="changes-paths" aria-label="Страницы изменения">
        {changeset.paths.map((path) => (
          <Link key={path} className="g-mono" to={`/docs?page=${encodeURIComponent(path)}`}>
            {path}
          </Link>
        ))}
      </section>

      <section className="changes-diff" aria-label="Разница с основной версией">
        <h3>
          Разница <span className="g-muted">+{stats.added} −{stats.removed}</span>
        </h3>
        <DocDiff diff={diff} empty="Текстовых изменений нет" />
      </section>

      {commits.length > 0 && (
        <section className="changes-commits" aria-label="Коммиты изменения">
          <h3>Коммиты</h3>
          <ol>
            {commits.map((commit) => (
              <li key={commit.sha}>
                <span className="g-mono g-muted">{commit.sha.slice(0, 8)}</span>
                <span>{commit.subject}</span>
                <span className="g-muted">{commit.authorName}</span>
                {isAgentMergedCommit(commit) && <Tag tone="signal">свёл агент</Tag>}
              </li>
            ))}
          </ol>
        </section>
      )}
    </div>
  );
}

function originName(origin: Changeset["origin"]): string {
  switch (origin) {
    case "merge":
      return "сведено агентом";
    case "revert":
      return "откат";
    case "restore":
      return "восстановление";
    case "migration":
      return "перенос";
    case "remote":
      return "из удалённого репозитория";
    default:
      return "из genie";
  }
}
