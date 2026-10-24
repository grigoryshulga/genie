import { useState } from "react";
import type { DocPage } from "../model.ts";
import { articleTitle, buildDocNavigation, docSections, sectionTrail, type DocSection } from "../navigation.ts";
import { DocMarks } from "./DocBadges.tsx";
import { BookIcon, CaretIcon, DocFileIcon } from "./icons.tsx";

function Article({ page, selected, onSelect }: { page: DocPage; selected?: string; onSelect: (path: string) => void }) {
  return <button type="button" className={`doc-row${page.path === selected ? " on" : ""}`} onClick={() => onSelect(page.path)} aria-current={page.path === selected ? "page" : undefined}>
    <DocFileIcon size={13} className="doc-row-ic" />
    <span className="nm">{articleTitle(page)}</span><DocMarks page={page} />
  </button>;
}

function Section({ section, selected, activeSection, onSelect, onSection, filter }: {
  section: DocSection; selected?: string; activeSection?: string; onSelect: (path: string) => void; onSection: (path: string) => void; filter: string;
}) {
  const [open, setOpen] = useState(true);
  const expanded = !!filter || open;
  return <div className="doc-nav-section">
    <div className="doc-section-row">
      <button type="button" className="g-icon-btn" aria-label={`${expanded ? "Свернуть" : "Развернуть"}: ${section.title}`} aria-expanded={expanded} onClick={() => setOpen(!open)}><CaretIcon size={11} className={`doc-caret${expanded ? " open" : ""}`} /></button>
      <button type="button" className={`doc-section-link${activeSection === section.path && !selected ? " on" : ""}`} onClick={() => onSection(section.path)}><span>{section.title}</span><span className="g-muted">{section.count}</span></button>
    </div>
    {expanded && <div className="doc-section-children">
      {section.landing && <Article page={section.landing} selected={selected} onSelect={onSelect} />}
      {section.sections.map((child) => <Section key={child.path} section={child} selected={selected} activeSection={activeSection} onSelect={onSelect} onSection={onSection} filter={filter} />)}
      {section.pages.map((page) => <Article key={page.path} page={page} selected={selected} onSelect={onSelect} />)}
    </div>}
  </div>;
}

export function DocsTree({ pages, roots, selected, section, onSelect, onSection, filter = "" }: {
  pages: DocPage[]; roots: string[]; selected?: string; section?: string; onSelect: (path: string) => void; onSection: (path: string) => void; filter?: string;
}) {
  const query = filter.trim().toLocaleLowerCase("ru");
  const full = buildDocNavigation(pages, roots);
  const shown = query ? pages.filter((p) => `${articleTitle(p)} ${p.path} ${sectionTrail(full, p.path).map((s) => s.title).join(" ")}`.toLocaleLowerCase("ru").includes(query)) : pages;
  const root = query ? buildDocNavigation(shown, roots) : full;
  const titles = new Map(docSections(full).map((s) => [s.path, s.title]));
  for (const section of docSections(root)) section.title = titles.get(section.path) ?? section.title;
  return <nav className="doc-tree" aria-label="Разделы документации">
    <div className="doc-tree-body">
      <button type="button" className={`doc-row doc-space-home${!selected && (!section || section === root.path) ? " on" : ""}`} onClick={() => onSection(root.path)}><BookIcon size={15} /><span className="nm">Обзор пространства</span></button>
      {query && !shown.length && <p className="doc-hint">Ничего не найдено</p>}
      {root.landing && <Article page={root.landing} selected={selected} onSelect={onSelect} />}
      {root.sections.map((child) => <Section key={child.path} section={child} selected={selected} activeSection={section} onSelect={onSelect} onSection={onSection} filter={query} />)}
      {root.pages.map((page) => <Article key={page.path} page={page} selected={selected} onSelect={onSelect} />)}
    </div>
  </nav>;
}
