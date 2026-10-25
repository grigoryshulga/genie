// DocView — the page view (mockups 1–3, 11): breadcrumbs, title with type/status/
// stale/diagnostics badges, the git-derived `updated` next to frontmatter
// `verified`, related/tag chips, banners, Markdown body and the aside with
// backlinks, `paths`, diagnostics and the on-page table of contents.

import { DocBreadcrumbs } from "./DocBreadcrumbs.tsx";
import { useRef, useState } from "react";
import { Link } from "react-router";
import type { DocPage, DocReadResult, DocSection } from "@/entities/doc";
import {
  fieldsFromPage,
  sectionTrail,
  serializeDoc,
  slugify,
  useRemoveDocPage,
  useRenameDoc,
  useSaveDoc,
  bodyUnderTitle,
  diagnosticText,
  docAgo,
  articleTitle,
  DocBody,
  DocDiagBadge,
  DocMarks,
  DocStaleBadge,
  DocStatusBadge,
  DocTypeBadge,
  firstParagraph,
  staleReasonText,
} from "@/entities/doc";
import { BacklinkIcon, DeprecatedIcon, DiagIcon, OkIcon, PathsIcon, StaleIcon } from "@/entities/doc";
import { canWriteProject, useSession } from "@/entities/session";
import { DocViewSwitch } from "@/widgets/docs-view-switch";
import { Button, ConfirmDialog, DisclosureCard, Icon, IconButton, RenameButton, RenameField, Tag, useToast } from "@/shared/ui";

export function DocView({
  page,
  navigation,
  onSection,
  pages,
  onOpen,
  onEdit,
  onMarkVerified,
  onOpenTree,
  hideAside,
}: {
  page: DocReadResult;
  navigation: DocSection;
  onSection: (path: string) => void;
  pages: DocPage[];
  onOpen: (path: string) => void;
  onEdit: () => void;
  onMarkVerified: () => void;
  onOpenTree?: () => void;
  hideAside?: boolean;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const byPath = new Map(pages.map((p) => [p.path, p]));
  const toast = useToast();
  const session = useSession();
  const save = useSaveDoc();
  const remove = useRemoveDocPage();
  const [removing, setRemoving] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const rename = useRenameDoc();
  const writable = canWriteProject(session.data);

  // A copy lives beside the original, as a draft with its own title, slug and no aliases or verification date.
  const clone = async () => {
    const title = `${page.title} (копия)`;
    const dir = page.path.slice(0, page.path.lastIndexOf("/") + 1);
    const content = serializeDoc({ ...fieldsFromPage(page), title, status: "draft", verified: "", aliases: [] }, page.content);
    try {
      const saved = await save.mutateAsync({ path: `${dir}${slugify(title)}.md`, content, mode: "create" });
      toast(`Создана копия: ${saved.page.title}`);
      onOpen(saved.page.path);
    } catch (error) {
      toast(error instanceof Error ? error.message : String(error), "error");
    }
  };

  // Once the page is gone, its section is the place to be.
  const confirmRemove = () => remove.mutate(page.path, {
    onSuccess: () => {
      toast(`Статья «${page.title}» удалена`);
      setRemoving(false);
      onSection(sectionTrail(navigation, page.path).at(-1)?.path ?? navigation.path);
    },
    onError: (error) => {
      setRemoving(false);
      toast(`Не удалось удалить статью: ${error.message}`, "error");
    },
  });

  const scrollToHeading = (heading: string) => {
    const blocks = scrollRef.current?.querySelectorAll(".doc-body h1, .doc-body h2, .doc-body h3, .doc-body h4");
    if (!blocks) return;
    for (const block of blocks) {
      if ((block.textContent ?? "").trim() === heading) {
        block.scrollIntoView({ behavior: "smooth", block: "start" });
        return;
      }
    }
  };

  const staleNote = page.staleReasons.length ? page.staleReasons.map(staleReasonText).join("; ") : "";
  // The title and the summary are shown once: not again as the body's heading and first paragraph.
  const body = bodyUnderTitle(page.content, page.title);
  const summary = page.summary && page.summary !== firstParagraph(page.content) ? page.summary : undefined;
  const headings = page.headings.filter((h) => h !== page.title);
  const badVerified = page.diagnostics.some((d) => d.includes('"verified"'));

  return (
    <>
      {/* The top row spans the reader and the aside, so the aside starts under it (see docs.css). */}
      <header className="docs-topbar">
        {onOpenTree && (
          <button type="button" className="g-icon-btn m-only" onClick={onOpenTree} aria-label="Открыть дерево документации">
            <Icon.list size={15} />
          </button>
        )}
        <DocBreadcrumbs root={navigation} path={page.path} title={articleTitle(page)} onSection={onSection} />
        <span className="grow" />
        <IconButton label="Редактировать" onClick={onEdit}>
          <Icon.pencil size={14} />
        </IconButton>
        {writable && (
          <IconButton label="Клонировать" onClick={() => void clone()} disabled={save.isPending}>
            <Icon.copy size={14} />
          </IconButton>
        )}
        {writable && (
          <IconButton label="Удалить" onClick={() => setRemoving(true)}>
            <Icon.trash size={14} />
          </IconButton>
        )}
        <DocViewSwitch />
      </header>
      <div className="docs-content">
        <div className="docs-view doc-reader scroll" ref={scrollRef}>
          <article className="doc-reading">
          <div className="doc-head">
            <div className="doc-head-top">
            <div className="doc-article-badges" aria-label="Тип и состояние статьи">
              <DocTypeBadge type={page.type} long />
              {page.status && <DocStatusBadge status={page.status} />}
              {page.stale && <DocStaleBadge count={page.staleReasons.length} />}
              {page.diagnostics.length > 0 && <DocDiagBadge count={page.diagnostics.length} />}
            </div>
            {page.tags.length > 0 && <div className="doc-tags" aria-label="Теги статьи">
              {page.tags.map((tag) => <Tag key={tag} className="doc-tag">{tag}</Tag>)}
            </div>}
            </div>
            {renaming ? (
              <RenameField className="doc-title" value={articleTitle(page)} label="Название статьи" onCommit={(title) => { setRenaming(false); rename.mutate({ path: page.path, title }); }} onCancel={() => setRenaming(false)} />
            ) : (
              <h2 className="doc-title g-rename-host">{articleTitle(page)}<RenameButton label="Переименовать статью" onClick={() => setRenaming(true)} /></h2>
            )}
            <div className="doc-meta">
              <Tag tone="outline">Обновлена {docAgo(page.updated)}</Tag>
              {page.verified ? (
                <Tag tone="signal" className="ok">
                  <OkIcon size={12} /> Проверена {page.verified}
                </Tag>
              ) : badVerified ? (
                <Tag tone="accent" className="warn">Дата проверки не распознана</Tag>
              ) : (
                <Tag tone="outline">Не проверялась</Tag>
              )}
              {page.related.length > 0 && (
                <span className="doc-related">
                  Задачи
                  {page.related.map((id) => (
                    <Link key={id} className="epic-chip" to={`/tasks?task=${encodeURIComponent(id)}`}>
                      {id}
                    </Link>
                  ))}
                </span>
              )}
            </div>
          </div>

          {page.diagnostics.length > 0 && (
            <div className="doc-banner diag g-banner danger">
              <DiagIcon size={15} />
              <div className="doc-banner-body">
                <b>Ошибка во frontmatter:</b> использованы значения по умолчанию. Файл прочитан и доступен, но поля ниже нужно исправить.
                <ul className="doc-banner-list">
                  {page.diagnostics.map((raw) => (
                    <li key={raw} title={raw} className="g-mono">
                      {diagnosticText(raw)}
                    </li>
                  ))}
                </ul>
              </div>
              <button type="button" className="g-btn secondary" onClick={onEdit}>
                Исправить в редакторе
              </button>
            </div>
          )}

          {page.status === "deprecated" && (
            <div className="doc-banner outdated g-banner signal">
              <DeprecatedIcon size={15} />
              <div className="doc-banner-body">
                <b>Страница устарела:</b> оставлена для истории — не опирайтесь на неё при работе.
              </div>
            </div>
          )}

          {page.stale && (
            <div className="doc-banner stale g-banner accent">
              <StaleIcon size={15} />
              <div className="doc-banner-body">
                <b>Возможно устарела:</b> {staleNote}
                {page.related.length > 0 && <span className="g-muted"> · задачи {page.related.join(", ")}</span>}
              </div>
              {page.diagnostics.length === 0 && (
                <Button sm onClick={onMarkVerified}>
                  <Icon.check size={13} />
                  Отметить проверенной
                </Button>
              )}
            </div>
          )}

          {summary && <div className="doc-summary">{summary}</div>}

          <DocBody text={body} links={page.links} onOpen={onOpen} empty="Страница пуста" />
          </article>
        </div>
      </div>

      {!hideAside && (
        <aside className="docs-aside">
          <DisclosureCard title="Ссылаются сюда" icon={<BacklinkIcon size={12} />} badge={<span className="g-num">{page.backlinks.length}</span>} storageKey="genie.docs.card.backlinks">
            {page.backlinks.length > 0 ? (
              <ul className="doc-links">
                {page.backlinks.map((path) => (
                  <li key={path}>
                    <button type="button" onClick={() => onOpen(path)}>
                      <span className="nm">{byPath.get(path)?.title ?? path}</span>
                    </button>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="doc-hint">На страницу пока никто не ссылается.</p>
            )}
          </DisclosureCard>

          <DisclosureCard title="Описывает код" icon={<PathsIcon size={12} />} storageKey="genie.docs.card.paths">
            {page.paths && page.paths.length > 0 ? (
              <ul className="doc-paths g-mono">
                {page.paths.map((pattern) => (
                  <li key={pattern}>
                    <Icon.file size={12} />
                    {pattern}
                  </li>
                ))}
              </ul>
            ) : (
              <p className="doc-hint">
                Не привязана к коду: поле <code>paths</code> пусто, поэтому genie не следит, устарела ли страница.
              </p>
            )}
            {page.paths && page.paths.length > 0 && (
              <p className={`doc-note${page.stale ? " stale" : ""}`}>{page.stale ? `Изменено после проверки: ${staleNote}` : "Без изменений после проверки"}</p>
            )}
          </DisclosureCard>

          {page.diagnostics.length > 0 && (
            <DisclosureCard title="Диагностика" icon={<DiagIcon size={12} />} storageKey="genie.docs.card.diagnostics">
              <ul className="doc-diags">
                {page.diagnostics.map((raw) => (
                  <li key={raw} title={raw}>
                    {diagnosticText(raw)}
                  </li>
                ))}
              </ul>
            </DisclosureCard>
          )}

          {headings.length > 0 && (
            <DisclosureCard title="На странице" storageKey="genie.docs.card.contents">
              <ul className="doc-toc">
                {headings.map((heading) => (
                  <li key={heading}>
                    <button type="button" onClick={() => scrollToHeading(heading)}>
                      {heading}
                    </button>
                  </li>
                ))}
              </ul>
            </DisclosureCard>
          )}

          {page.links.length > 0 && (
            <DisclosureCard title="Ссылки со страницы" storageKey="genie.docs.card.links">
              <ul className="doc-links">
                {page.links.map((link) => (
                  <li key={link.target}>
                    {link.resolution === "resolved" && link.targetPath ? (
                      <button type="button" onClick={() => onOpen(link.targetPath!)}>
                        <span className="nm">{link.target}</span>
                        <span className="g-mono sub">{link.targetPath}</span>
                      </button>
                    ) : (
                      <span className={link.resolution === "ambiguous" ? "unresolved ambiguous" : "unresolved"} title={link.resolution === "ambiguous" ? `Несколько совпадений: ${link.matches.join(", ")}` : "Страница не найдена — ссылка не угадывается"}>
                        {link.target}
                        <span className="g-muted"> · {link.resolution === "ambiguous" ? "несколько совпадений" : "нет страницы"}</span>
                      </span>
                    )}
                  </li>
                ))}
              </ul>
            </DisclosureCard>
          )}

          <div className="docs-aside-foot">
            <DocMarks page={page} />
            <span className="g-mono">{page.path}</span>
          </div>
        </aside>
      )}
      {removing && (
        <ConfirmDialog title={`Удалить статью «${page.title}»?`} confirmLabel="Удалить" danger busy={remove.isPending} onClose={() => setRemoving(false)} onConfirm={() => confirmRemove()}>
          Статья будет удалена из пространства. Из интерфейса это не отменить.
        </ConfirmDialog>
      )}
    </>
  );
}
