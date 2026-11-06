// The bar above a page the reader has an open changeset for: publish it, discard
// it, or read its diff. A publication that overlaps newer work returns `merging`
// and points at the merge instead of asking for the edit again.

import { useState } from "react";
import { Link } from "react-router";
import type { DocReadResult, DocMerge } from "@/entities/doc";
import {
  changesetPathsLabel,
  docActionError,
  useChangeset,
  useDiscardChangeset,
  usePublishChangeset,
} from "@/entities/doc";
import { Button, ConfirmDialog, Dialog, Icon, useToast } from "@/shared/ui";
import { DocDiff } from "./DocDiff.tsx";

const failure = (error: unknown) =>
  docActionError({ status: (error as { status?: number } | undefined)?.status, message: error instanceof Error ? error.message : String(error) });

export function DocChangesetBar({ page }: { page: DocReadResult }) {
  const id = page.changeset ?? undefined;
  const detail = useChangeset(id);
  const publish = usePublishChangeset();
  const discard = useDiscardChangeset();
  const toast = useToast();
  const [confirming, setConfirming] = useState(false);
  const [diffOpen, setDiffOpen] = useState(false);
  const [pendingMerge, setPendingMerge] = useState<DocMerge | null>(null);

  if (!id) return null;
  const changeset = detail.data?.changeset;
  const pages = changeset?.paths.length;
  // A reload of a merging changeset has no local result to point at; the detail carries its merge.
  const merge = pendingMerge ?? detail.data?.merge;

  const doPublish = () => {
    setPendingMerge(null);
    publish.mutate(id, {
      onSuccess: (result) => {
        if (result.status === "merging") {
          setPendingMerge(result.merge);
          toast("Правки пересеклись с более новыми — их сводит агент");
        } else {
          toast("Изменения опубликованы");
        }
      },
      onError: (error) => toast(failure(error), "error"),
    });
  };

  const doDiscard = () => {
    discard.mutate(id, {
      onSuccess: () => {
        setConfirming(false);
        toast(`Черновик ${id} отменён`);
      },
      onError: (error) => {
        setConfirming(false);
        toast(failure(error), "error");
      },
    });
  };

  return (
    <div className="doc-changeset-bar" aria-label="Черновик изменений">
      <div className="doc-changeset-text">
        <Icon.branch size={14} />
        <b>Черновик изменений #{id}</b>
        <span className="g-muted">·</span>
        <span>{pages === undefined ? "загружаем…" : changesetPathsLabel(pages)}</span>
      </div>
      {merge && (
        <p className="doc-changeset-merge">
          Правки пересеклись с более новыми — их сводит агент.{" "}
          <Link to={`/docs/changes/merge/${merge.id}`}>Открыть слияние</Link>
        </p>
      )}
      <div className="doc-changeset-actions">
        <Button sm onClick={() => setDiffOpen(true)}>
          <Icon.columns size={13} />
          Показать изменения
        </Button>
        <Button variant="danger" sm onClick={() => setConfirming(true)} busy={discard.isPending}>
          Отменить изменения
        </Button>
        <Button variant="primary" sm onClick={doPublish} busy={publish.isPending} disabled={changeset?.status === "merging"}>
          {changeset?.status === "merging" ? "Сводится" : "Опубликовать"}
        </Button>
      </div>

      {confirming && (
        <ConfirmDialog
          title={`Отменить черновик изменений #${id}?`}
          confirmLabel="Отменить изменения"
          danger
          busy={discard.isPending}
          onClose={() => setConfirming(false)}
          onConfirm={doDiscard}
        >
          Ветка закроется без слияния с основной версией. Текст останется в истории git, но из пространства знаний правки исчезнут.
        </ConfirmDialog>
      )}

      {diffOpen && (
        <Dialog title={`Изменения в черновике #${id}`} wide onClose={() => setDiffOpen(false)}>
          {detail.isPending ? <p className="doc-hint">Загружаем разницу…</p> : detail.isError ? <p className="doc-hint">Не удалось прочитать изменения: {failure(detail.error)}</p> : <DocDiff diff={detail.data?.diff} />}
        </Dialog>
      )}
    </div>
  );
}
