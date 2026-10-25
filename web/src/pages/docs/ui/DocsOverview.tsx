import { useState, type ReactNode } from "react";
import { articleTitle, BookIcon, DocSaveError, DocTypeBadge, docAgo, fetchDocPage, fieldsFromPage,  sectionTrail, serializeDoc, useRemoveDocSection, useRenameDoc, useSaveDoc, type DocPage, type DocSection } from "@/entities/doc";
import { canAdminProject, useSession } from "@/entities/session";
import { ConfirmDialog, Icon, IconButton, RenameButton, RenameField, SearchField, Select, Tag, Textarea, useToast } from "@/shared/ui";

type ArticleOrder = "title" | "newest" | "oldest";

const ORDERS: { value: ArticleOrder; label: string }[] = [
  { value: "title", label: "По названию" },
  { value: "newest", label: "Сначала новые" },
  { value: "oldest", label: "Сначала старые" },
];

/** The `updated` date as a timestamp; missing or unreadable dates sort after dated articles. */
function updatedAt(page: DocPage): number | undefined {
  const stamp = Date.parse(page.updated ?? "");
  return Number.isFinite(stamp) ? stamp : undefined;
}

function byUpdated(direction: 1 | -1) {
  return (a: DocPage, b: DocPage) => {
    const x = updatedAt(a);
    const y = updatedAt(b);
    if (x === undefined || y === undefined) return x === y ? 0 : x === undefined ? 1 : -1;
    return direction * (x - y);
  };
}

export function DocsOverview({ root, section, toolbar, onSection, onOpen, onCreate, onOpenTree }: {
  root: DocSection; section: DocSection; toolbar: ReactNode; onSection: (path: string) => void; onOpen: (path: string) => void; onCreate: (kind: "page" | "section") => void; onOpenTree: () => void;
}) {
  const toast = useToast();
  const save = useSaveDoc();
  const remove = useRemoveDocSection();
  const session = useSession();
  const [editing, setEditing] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [renamingPath, setRenamingPath] = useState<string>();
  const rename = useRenameDoc();
  const [draft, setDraft] = useState("");
  const [query, setQuery] = useState("");
  const [order, setOrder] = useState<ArticleOrder>("title");

  const summary = section.landing?.summary;
  const articles = section.pages;
  // The section's own page disappears with it, so the view moves to the section above.
  const parent = sectionTrail(root, section.path).at(-2)?.path ?? root.path;
  // Spaces are bindings of the project (several roots): only their sections can be removed.
  const removable = section !== root && !(root.path === "" && root.sections.includes(section));
  const confirmRemove = () => remove.mutate(section.path, {
    onSuccess: () => {
      toast(`Раздел «${section.title}» удалён`);
      setRemoving(false);
      onSection(parent);
    },
    onError: (error) => {
      setRemoving(false);
      toast(`Не удалось удалить раздел: ${error.message}`, "error");
    },
  });
  const needle = query.trim().toLowerCase();
  const matching = articles.filter((page) => !needle || [articleTitle(page), page.summary ?? "", page.path].some((text) => text.toLowerCase().includes(needle)));
  const shown = order === "title" ? matching : [...matching].sort(byUpdated(order === "newest" ? -1 : 1));

  // The description lives in the section's index page: its frontmatter `summary`. A section without one gets an index page.
  const openEditor = () => {
    setDraft(summary ?? "");
    setEditing(true);
  };
  const saveDescription = async () => {
    const text = draft.trim() || null;
    try {
      if (section.landing) {
        const page = await fetchDocPage(section.landing.path);
        await save.mutateAsync({ path: page.path, content: serializeDoc({ ...fieldsFromPage(page), summary: text }, page.content), mode: "update" });
      } else {
        const content = serializeDoc({ title: section.title, type: "", status: "", verified: "", tags: [], paths: [], related: [], summary: text, aliases: [] }, "");
        await save.mutateAsync({ path: `${section.path}index.md`, content, mode: "create" });
      }
      setEditing(false);
    } catch (error) {
      const approval = error instanceof DocSaveError && error.status === 202;
      toast(error instanceof Error ? error.message : String(error), approval ? "ok" : "error");
    }
  };

  return <div className="docs-content">
    <header className="docs-topbar page-head">
      <button type="button" className="g-icon-btn m-only" onClick={onOpenTree} aria-label="Открыть навигацию"><Icon.list size={15} /></button>
      {section !== root && (renaming ? (
        <RenameField className="g-page-title" value={section.title} label="Название раздела" onCommit={(title) => { setRenaming(false); rename.mutate({ path: section.landing?.path ?? `${section.path}index.md`, create: !section.landing, title }); }} onCancel={() => setRenaming(false)} />
      ) : (
        <h1 className="g-page-title g-rename-host">{section.title}<RenameButton label="Переименовать раздел" onClick={() => setRenaming(true)} /></h1>
      ))}
      <span className="grow" />
      <div className="page-toolbar" role="toolbar" aria-label="Документация и предложения">
        <IconButton label="Новая статья" onClick={() => onCreate("page")}><Icon.plus size={14} /></IconButton>
        <IconButton label="Создать раздел" onClick={() => onCreate("section")}><Icon.folder size={14} /></IconButton>
        {removable && canAdminProject(session.data) && <IconButton label="Удалить раздел" onClick={() => setRemoving(true)}><Icon.trash size={14} /></IconButton>}
        {toolbar}
      </div>
    </header>
    <div className="docs-view doc-overview scroll">
      {editing ? (
        <form className="doc-space-edit" onSubmit={(event) => { event.preventDefault(); void saveDescription(); }}>
          <Textarea aria-label="Описание раздела" autoFocus rows={3} value={draft} placeholder="Коротко, что лежит в этом разделе" onChange={(event) => setDraft(event.target.value)} />
          <div className="doc-space-actions">
            <button type="submit" className="g-btn primary" disabled={save.isPending}>Сохранить</button>
            <button type="button" className="g-btn ghost" onClick={() => setEditing(false)}>Отмена</button>
          </div>
        </form>
      ) : summary ? (
        <>
          <p className="doc-space-description">{summary}</p>
          <div className="doc-space-tools"><button type="button" className="g-btn ghost" onClick={openEditor}>Изменить описание</button></div>
        </>
      ) : (
        <div className="doc-space-tools"><button type="button" className="g-btn ghost" onClick={openEditor}>Добавить описание</button></div>
      )}
      {removing && <ConfirmDialog title={`Удалить раздел «${section.title}»?`} confirmLabel="Удалить" danger busy={remove.isPending} onClose={() => setRemoving(false)} onConfirm={() => confirmRemove()}>
        В разделе и вложенных разделах страниц: {section.count}. Все они будут удалены вместе с папкой. Из интерфейса это не отменить.
      </ConfirmDialog>}
      {section.sections.length > 0 && <section aria-label="Разделы" className="doc-overview-section"><h3>Разделы</h3><div className="doc-section-cards">
        {section.sections.map((child) => <button key={child.path} type="button" className="doc-section-card" onClick={() => onSection(child.path)}>
          <BookIcon size={18} /><strong>{child.title}</strong>
          {child.landing?.summary && <p className="doc-section-card-summary">{child.landing.summary}</p>}
          <span className="doc-section-card-foot"><span className="g-muted">Статей: {child.count}</span><span className="doc-section-card-open">Открыть раздел →</span></span>
        </button>)}
      </div></section>}
      <section className="doc-overview-section">
        <div className="doc-list-head">
          <h3>{section === root ? "Статьи пространства" : "Статьи раздела"}</h3>
          {articles.length > 0 && <div className="doc-list-tools">
            <SearchField label="Поиск статей раздела" placeholder="Найти статью…" value={query} onChange={(event) => setQuery(event.target.value)} />
            <Select sm aria-label="Сортировка статей" value={order} onChange={(event) => setOrder(event.target.value as ArticleOrder)}>
              {ORDERS.map((item) => <option key={item.value} value={item.value}>{item.label}</option>)}
            </Select>
          </div>}
        </div>
        {shown.length ? <div className="doc-article-list">{shown.map((page) => renamingPath === page.path ? (
          <div key={page.path} className="doc-article-item doc-article-editing">
            <RenameField value={articleTitle(page)} label="Название статьи" onCommit={(title) => { setRenamingPath(undefined); rename.mutate({ path: page.path, title }); }} onCancel={() => setRenamingPath(undefined)} />
          </div>
        ) : (
          <div key={page.path} className="doc-article-item g-rename-host">
            <button type="button" className="doc-article-open" onClick={() => onOpen(page.path)}>
              <span><strong>{articleTitle(page)}</strong>{page.summary && <span className="doc-article-summary">{page.summary}</span>}</span>
              <span className="doc-article-meta">
                <DocTypeBadge type={page.type} />
                {page.tags.slice(0, 3).map((tag) => <Tag key={tag} className="doc-tag">{tag}</Tag>)}
                {page.tags.length > 3 && <span className="g-muted">+{page.tags.length - 3}</span>}
              </span>
              <span className="g-muted">{docAgo(page.updated)} →</span>
            </button>
            <RenameButton label="Переименовать статью" onClick={() => setRenamingPath(page.path)} />
          </div>
        ))}</div> : articles.length ? <p className="doc-hint">Ничего не найдено по запросу «{query.trim()}».</p> : <p className="doc-hint">Здесь пока нет статей. Создайте первую или выберите вложенный раздел.</p>}
      </section>
    </div>
  </div>;
}
