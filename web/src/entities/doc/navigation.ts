import type { DocPage } from "./model.ts";

export interface DocSection {
  path: string;
  title: string;
  landing?: DocPage;
  sections: DocSection[];
  pages: DocPage[];
  count: number;
}

/** Existing repositories need no migration to acquire readable section names. */
export function readableDocName(value: string): string {
  const text = value.replace(/\.md$/i, "").replace(/[-_]+/g, " ").trim();
  return text ? text[0].toLocaleUpperCase("ru") + text.slice(1) : "Обзор";
}

export function articleTitle(page: Pick<DocPage, "title" | "path">): string {
  const stem = page.path.split("/").at(-1)?.replace(/\.md$/i, "") ?? "";
  if (page.title && page.title !== stem) return page.title;
  const name = /^(index|readme)$/i.test(stem) ? page.path.split("/").at(-2) ?? "Обзор" : stem;
  return readableDocName(name);
}

export function buildDocNavigation(pages: DocPage[], roots: string[]): DocSection {
  const root: DocSection = { path: roots.length === 1 ? `${roots[0]}/` : "", title: "Документация", sections: [], pages: [], count: 0 };
  if (roots.length > 1) {
    root.sections = roots.map((name) => ({ path: `${name}/`, title: readableDocName(name), sections: [], pages: [], count: 0 }));
  }
  for (const page of pages) {
    const parts = page.path.split("/");
    const file = parts.pop() ?? "";
    const ownedRoot = parts.shift();
    if (!ownedRoot || !roots.includes(ownedRoot)) continue;
    const segments = roots.length === 1 ? parts : [ownedRoot, ...parts];
    let node = root;
    let path = roots.length === 1 ? `${ownedRoot}/` : "";
    for (const segment of segments) {
      path += `${segment}/`;
      let section = node.sections.find((s) => s.path === path);
      if (!section) {
        section = { path, title: readableDocName(segment), sections: [], pages: [], count: 0 };
        node.sections.push(section);
      }
      node = section;
    }
    if (/^(index|readme)\.md$/i.test(file) && (!node.landing || /^index\.md$/i.test(file))) {
      // Prefer index when both conventional landing pages exist; keep the other article.
      if (node.landing) node.pages.push(node.landing);
      node.landing = page;
      node.title = articleTitle(page);
    } else node.pages.push(page);
  }
  const order = (node: DocSection): number => {
    node.sections.sort((a, b) => a.title.localeCompare(b.title, "ru"));
    node.pages.sort((a, b) => articleTitle(a).localeCompare(articleTitle(b), "ru"));
    node.count = node.pages.length + node.sections.reduce((n, s) => n + order(s), 0);
    return node.count;
  };
  order(root);
  return root;
}

/** Index pages give a section its name and description; they are the section itself, not articles to list. */
export function sectionLandingPaths(root: DocSection): Set<string> {
  return new Set(docSections(root).flatMap((section) => (section.landing ? [section.landing.path] : [])));
}

export function docSections(root: DocSection): DocSection[] {
  return [root, ...root.sections.flatMap(docSections)];
}

export function sectionTrail(root: DocSection, path: string): DocSection[] {
  for (const section of root.sections) {
    if (path.startsWith(section.path)) return [root, ...sectionTrail(section, path)];
  }
  return [root];
}

export function articlePath(section: string, slug: string, kind: "page" | "note" | "section", date: string): string {
  if (kind === "section") return `${section}${slug}/index.md`;
  return `${section}${kind === "note" ? `inbox/${date}-` : ""}${slug}.md`;
}
