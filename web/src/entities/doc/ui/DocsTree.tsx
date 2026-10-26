import { useState } from "react";
import type { DocPage } from "../model.ts";
import { articleTitle, buildDocNavigation, docSections, sectionLandingPaths, sectionTrail, type DocSection } from "../navigation.ts";
import { useRenameDoc, type RenameDoc } from "../api.ts";
import { DocMarks } from "./DocBadges.tsx";
import { BookIcon, CaretIcon, DocFileIcon } from "./icons.tsx";
import { RenameButton, RenameField } from "@/shared/ui";

function Article({ page, selected, onSelect, onRename }: { page: DocPage; selected?: string; onSelect: (path: string) => void; onRename: (target: RenameDoc) => void }) {
  const [editing, setEditing] = useState(false);
  if (editing) return <div className="doc-row doc-row-rename">
    <DocFileIcon size={13} className="doc-row-ic" />
    <RenameField value={articleTitle(page)} label="Название статьи" onCommit={(title) => { setEditing(false); onRename({ path: page.path, title }); }} onCancel={() => setEditing(false)} />
  </div>;
  return <div className="g-rename-host">
    <button type="button" className={`doc-row${page.path === selected ? " on" : ""}`} onClick={() => onSelect(page.path)} aria-current={page.path === selected ? "page" : undefined}>
      <DocFileIcon size={13} className="doc-row-ic" />
      <span className="nm">{articleTitle(page)}</span><DocMarks page={page} />
    </button>
    <RenameButton label="Переименовать статью" onClick={() => setEditing(true)} />
  </div>;
}

function Section({ section, selected, activeSection, onSelect, onSection, filter, onRename }: {
  section: DocSection; selected?: string; activeSection?: string; onSelect: (path: string) => void; onSection: (path: string) => void; filter: string; onRename: (target: RenameDoc) => void;
}) {
  const [open, setOpen] = useState(true);
  const [editing, setEditing] = useState(false);
  const expanded = !!filter || open;
  // The section's name is its index page's title; a section without an index page gets one when renamed.
  const renameTarget = { path: section.landing?.path ?? `${section.path}index.md`, create: !section.landing };
  return <div className="doc-nav-section">
    <div className={`doc-section-row${editing ? "" : " g-rename-host"}`}>
      <button type="button" className="g-icon-btn" aria-label={`${expanded ? "Свернуть" : "Развернуть"}: ${section.title}`} aria-expanded={expanded} onClick={() => setOpen(!open)}><CaretIcon size={11} className={`doc-caret${expanded ? " open" : ""}`} /></button>
      {editing ? (
        <RenameField value={section.title} label="Название раздела" onCommit={(title) => { setEditing(false); onRename({ ...renameTarget, title }); }} onCancel={() => setEditing(false)} />
      ) : (
        <>
          <button type="button" className={`doc-section-link${activeSection === section.path && !selected ? " on" : ""}`} onClick={() => onSection(section.path)}><span>{section.title}</span><span className="g-muted">{section.count}</span></button>
          <RenameButton label="Переименовать раздел" onClick={() => setEditing(true)} />
        </>
      )}
    </div>
    {expanded && <div className="doc-section-children">
      {section.sections.map((child) => <Section key={child.path} section={child} selected={selected} activeSection={activeSection} onSelect={onSelect} onSection={onSection} filter={filter} onRename={onRename} />)}
      {section.pages.map((page) => <Article key={page.path} page={page} selected={selected} onSelect={onSelect} onRename={onRename} />)}
    </div>}
  </div>;
}

export function DocsTree({ pages, roots, selected, section, onSelect, onSection, filter = "" }: {
  pages: DocPage[]; roots: string[]; selected?: string; section?: string; onSelect: (path: string) => void; onSection: (path: string) => void; filter?: string;
}) {
  const rename = useRenameDoc();
  const onRename = (target: RenameDoc) => rename.mutate(target);
  const query = filter.trim().toLocaleLowerCase("ru");
  const full = buildDocNavigation(pages, roots);
  const landings = sectionLandingPaths(full);
  const articles = pages.filter((p) => !landings.has(p.path));
  const shown = query ? articles.filter((p) => `${articleTitle(p)} ${p.path} ${sectionTrail(full, p.path).map((s) => s.title).join(" ")}`.toLocaleLowerCase("ru").includes(query)) : articles;
  const root = query ? buildDocNavigation(shown, roots) : full;
  const titles = new Map(docSections(full).map((s) => [s.path, s.title]));
  for (const section of docSections(root)) section.title = titles.get(section.path) ?? section.title;
  return <nav className="doc-tree" aria-label="Разделы документации">
    <div className="doc-tree-body">
      <button type="button" className={`doc-row doc-space-home${!selected && (!section || section === root.path) ? " on" : ""}`} onClick={() => onSection(root.path)}><BookIcon size={15} /><span className="nm">Обзор пространства</span></button>
      {query && !shown.length && <p className="doc-hint">Ничего не найдено</p>}
      {root.sections.map((child) => <Section key={child.path} section={child} selected={selected} activeSection={section} onSelect={onSelect} onSection={onSection} filter={query} onRename={onRename} />)}
      {root.pages.map((page) => <Article key={page.path} page={page} selected={selected} onSelect={onSelect} onRename={onRename} />)}
    </div>
  </nav>;
}
