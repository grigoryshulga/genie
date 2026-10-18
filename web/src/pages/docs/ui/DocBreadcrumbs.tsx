import { sectionTrail, type DocSection } from "@/entities/doc";

export function DocBreadcrumbs({ root, path, title, onSection }: { root: DocSection; path: string; title?: string; onSection: (path: string) => void }) {
  const trail = sectionTrail(root, path);
  return <nav className="doc-crumbs" aria-label="Навигация по документации">
    {trail.map((section, i) => <span key={section.path}>
      {i > 0 && <span className="sep">›</span>}
      <button type="button" onClick={() => onSection(section.path)}>{i === 0 ? "Документация" : section.title}</button>
    </span>)}
    {title && <span><span className="sep">›</span><span aria-current="page">{title}</span></span>}
  </nav>;
}
