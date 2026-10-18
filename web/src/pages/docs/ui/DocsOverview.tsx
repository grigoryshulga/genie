import { articleTitle, BookIcon, docAgo, type DocSection } from "@/entities/doc";
import { Icon } from "@/shared/ui";
import { DocBreadcrumbs } from "./DocBreadcrumbs.tsx";

export function DocsOverview({ root, section, projectName, onSection, onOpen, onCreate, onOpenTree }: {
  root: DocSection; section: DocSection; projectName: string; onSection: (path: string) => void; onOpen: (path: string) => void; onCreate: (kind: "page" | "section") => void; onOpenTree: () => void;
}) {
  const articles = section.landing ? [section.landing, ...section.pages] : section.pages;
  return <div className="docs-content">
    <header className="docs-topbar">
      <button type="button" className="icon-btn m-only" onClick={onOpenTree} aria-label="Открыть навигацию"><Icon.list size={15} /></button>
      <DocBreadcrumbs root={root} path={section.path} onSection={onSection} />
    </header>
    <div className="docs-view doc-overview scroll">
      <div className="doc-space-kicker">Пространство проекта · {projectName}</div>
      <h2 className="doc-title">{section === root ? "Документация проекта" : section.title}</h2>
      <p className="doc-space-description">{section.landing?.summary || "Знания проекта, собранные в статьи и разделы. Выберите тему или добавьте то, что поможет команде."}</p>
      <div className="doc-space-actions">
        <button type="button" className="btn primary" onClick={() => onCreate("page")}><Icon.plus size={13} />Новая статья</button>
        <button type="button" className="btn" onClick={() => onCreate("section")}>Создать раздел</button>
      </div>
      {section.sections.length > 0 && <section aria-label="Разделы" className="doc-overview-section"><h3>Разделы</h3><div className="doc-section-cards">
        {section.sections.map((child) => <button key={child.path} type="button" className="doc-section-card" onClick={() => onSection(child.path)}>
          <BookIcon size={18} /><strong>{child.title}</strong><span className="muted">Статей: {child.count} · Открыть раздел →</span>
          {child.landing?.summary && <p>{child.landing.summary}</p>}
        </button>)}
      </div></section>}
      <section className="doc-overview-section"><h3>{section === root ? "Статьи пространства" : "Статьи раздела"}</h3>
        {articles.length ? <div className="doc-article-list">{articles.map((page) => <button type="button" key={page.path} onClick={() => onOpen(page.path)}>
          <span><strong>{articleTitle(page)}</strong>{page.summary && <span className="doc-article-summary">{page.summary}</span>}</span><span className="muted">{docAgo(page.updated)} →</span>
        </button>)}</div> : <p className="doc-hint">Здесь пока нет статей. Создайте первую или выберите вложенный раздел.</p>}
      </section>
    </div>
  </div>;
}
