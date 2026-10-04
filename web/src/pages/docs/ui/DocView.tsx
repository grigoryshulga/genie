// DocView — the page view (mockups 1–3, 11): breadcrumbs, title with type/status/
// stale/diagnostics badges, the git-derived `updated` next to frontmatter
// `verified`, related/tag chips, banners, Markdown body and the aside with
// backlinks, `paths`, diagnostics and the on-page table of contents.

import { useRef } from "react";
import { Link } from "react-router";
import type { DocPage, DocReadResult } from "@/entities/doc";
import {
  bodyUnderTitle,
  diagnosticText,
  docAgo,
  docCrumbs,
  DocBody,
  DocDiagBadge,
  DocMarks,
  DocStaleBadge,
  DocStatusBadge,
  DocTypeBadge,
  firstParagraph,
  staleReasonText,
} from "@/entities/doc";
import { BacklinkIcon, DiagIcon, OkIcon, PathsIcon, StaleIcon } from "@/entities/doc";
import { Icon } from "@/shared/ui";

export function DocView({
  page,
  pages,
  onOpen,
  onEdit,
  onMarkVerified,
  onOpenTree,
  hideAside,
}: {
  page: DocReadResult;
  pages: DocPage[];
  onOpen: (path: string) => void;
  onEdit: () => void;
  onMarkVerified: () => void;
  onOpenTree?: () => void;
  hideAside?: boolean;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const crumbs = docCrumbs(page.path);
  const byPath = new Map(pages.map((p) => [p.path, p]));

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
      <div className="docs-content">
        <header className="docs-topbar">
          {onOpenTree && (
            <button type="button" className="icon-btn m-only" onClick={onOpenTree} aria-label="Открыть дерево документации">
              <Icon.list size={15} />
            </button>
          )}
          <nav className="doc-crumbs mono" aria-label="Путь страницы">
            {crumbs.map((part, i) => (
              <span key={`${part}-${i}`}>
                {i > 0 && <span className="sep">/</span>}
                {part}
              </span>
            ))}
          </nav>
          <span className="grow" />
          <button type="button" className="btn" onClick={onEdit}>
            <Icon.file size={13} />
            Редактировать
          </button>
        </header>

        <div className="docs-view scroll" ref={scrollRef}>
          <div className="doc-head">
            <h2 className="doc-title">
              {page.title}
              <DocTypeBadge type={page.type} long />
              {page.status && <DocStatusBadge status={page.status} />}
              {page.stale && <DocStaleBadge count={page.staleReasons.length} />}
              {page.diagnostics.length > 0 && <DocDiagBadge count={page.diagnostics.length} />}
            </h2>
            <div className="doc-meta">
              <span>Обновлена {docAgo(page.updated)}</span>
              {page.verified ? (
                <span className="ok">
                  <OkIcon size={12} /> Проверена {page.verified}
                </span>
              ) : badVerified ? (
                <span className="warn">Дата проверки не распознана</span>
              ) : (
                <span>Не проверялась</span>
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
              {page.tags.map((tag) => (
                <span key={tag} className="doc-tag">
                  {tag}
                </span>
              ))}
            </div>
          </div>

          {page.diagnostics.length > 0 && (
            <div className="doc-banner diag">
              <DiagIcon size={15} />
              <div className="doc-banner-body">
                <b>Ошибка во frontmatter:</b> использованы значения по умолчанию. Файл прочитан и доступен, но поля ниже нужно исправить.
                <ul className="doc-banner-list">
                  {page.diagnostics.map((raw) => (
                    <li key={raw} title={raw} className="mono">
                      {diagnosticText(raw)}
                    </li>
                  ))}
                </ul>
              </div>
              <button type="button" className="btn" onClick={onEdit}>
                Исправить в редакторе
              </button>
            </div>
          )}

          {page.stale && (
            <div className="doc-banner stale">
              <StaleIcon size={15} />
              <div className="doc-banner-body">
                <b>Возможно устарела:</b> {staleNote}
                {page.related.length > 0 && <span className="muted"> · задачи {page.related.join(", ")}</span>}
              </div>
              {page.diagnostics.length === 0 && (
                <button type="button" className="btn amber" onClick={onMarkVerified}>
                  <Icon.check size={13} />
                  Отметить проверенной
                </button>
              )}
            </div>
          )}

          {summary && <div className="doc-summary">{summary}</div>}

          <DocBody text={body} links={page.links} onOpen={onOpen} empty="Страница пуста" />
        </div>
      </div>

      {!hideAside && (
        <aside className="docs-aside">
          <section className="docs-sec">
            <h3>
              <BacklinkIcon size={12} /> Ссылаются сюда <span className="n">{page.backlinks.length}</span>
            </h3>
            {page.backlinks.length > 0 ? (
              <ul className="doc-links">
                {page.backlinks.map((path) => (
                  <li key={path}>
                    <button type="button" onClick={() => onOpen(path)}>
                      <span className="nm">{byPath.get(path)?.title ?? path}</span>
                      <span className="mono sub">{path}</span>
                    </button>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="doc-hint">На страницу пока никто не ссылается.</p>
            )}
          </section>

          <section className="docs-sec">
            <h3>
              <PathsIcon size={12} /> Описывает код
            </h3>
            {page.paths && page.paths.length > 0 ? (
              <ul className="doc-paths mono">
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
          </section>

          {page.diagnostics.length > 0 && (
            <section className="docs-sec">
              <h3>
                <DiagIcon size={12} /> Диагностика
              </h3>
              <ul className="doc-diags">
                {page.diagnostics.map((raw) => (
                  <li key={raw} title={raw}>
                    {diagnosticText(raw)}
                  </li>
                ))}
              </ul>
            </section>
          )}

          {headings.length > 0 && (
            <section className="docs-sec">
              <h3>На странице</h3>
              <ul className="doc-toc">
                {headings.map((heading) => (
                  <li key={heading}>
                    <button type="button" onClick={() => scrollToHeading(heading)}>
                      {heading}
                    </button>
                  </li>
                ))}
              </ul>
            </section>
          )}

          {page.links.length > 0 && (
            <section className="docs-sec">
              <h3>Ссылки со страницы</h3>
              <ul className="doc-links">
                {page.links.map((link) => (
                  <li key={link.target}>
                    {link.resolution === "resolved" && link.targetPath ? (
                      <button type="button" onClick={() => onOpen(link.targetPath!)}>
                        <span className="nm">{link.target}</span>
                        <span className="mono sub">{link.targetPath}</span>
                      </button>
                    ) : (
                      <span className={link.resolution === "ambiguous" ? "unresolved ambiguous" : "unresolved"} title={link.resolution === "ambiguous" ? `Несколько совпадений: ${link.matches.join(", ")}` : "Страница не найдена — ссылка не угадывается"}>
                        {link.target}
                        <span className="muted"> · {link.resolution === "ambiguous" ? "несколько совпадений" : "нет страницы"}</span>
                      </span>
                    )}
                  </li>
                ))}
              </ul>
            </section>
          )}

          <div className="docs-aside-foot">
            <DocMarks page={page} />
            <span className="mono">{page.path}</span>
          </div>
        </aside>
      )}
    </>
  );
}
