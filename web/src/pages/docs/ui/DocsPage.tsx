// DocsPage — `/docs` and `/docs/edit`. Owns the URL state (page, q, new), the
// column layout (tree | page | aside, search, editor, empty state), the mobile
// drawer and the docs change signal.

import { lazy, Suspense, useMemo, useState } from "react";
import { Navigate, useLocation, useNavigate, useSearchParams } from "react-router";
import {
  buildDocNavigation,
  docSections,
  sectionTrail,
  DocsTree,
  fieldsFromPage,
  serializeDoc,
  todayIso,
  useDebounced,
  useDocPage,
  useDocSearch,
  useDocsTree,
  useDocsVersion,
  useSaveDoc,
} from "@/entities/doc";
import { useSession } from "@/entities/session";
import { ContextSearch } from "@/widgets/sidebar";
import { useToast, Icon } from "@/shared/ui";
import { DocsOverview } from "./DocsOverview.tsx";
import { DocSearchList } from "./DocSearchList.tsx";
import { DocView } from "./DocView.tsx";
import { NewDocDialog } from "./NewDocDialog.tsx";

const DocEditor = lazy(() => import("./DocEditor.tsx").then((module) => ({ default: module.DocEditor })));

export function DocsPage() {
  const [sp, setSp] = useSearchParams();
  const location = useLocation();
  const navigate = useNavigate();
  const toast = useToast();
  const mode = /\/edit\/?$/.test(location.pathname) ? "edit" : "view";

  const tree = useDocsTree();
  const session = useSession();
  useDocsVersion();
  const pages = useMemo(() => tree.data?.pages ?? [], [tree.data]);

  const roots = tree.data?.roots ?? [];
  const navigation = buildDocNavigation(pages, roots);
  const sectionPath = sp.get("section") ?? navigation.path;
  const section = docSections(navigation).find((s) => s.path === sectionPath);
  const projectName = session.data?.projects.find((p) => p.slug === session.data?.project)?.name || session.data?.project || "";
  const pagePath = sp.get("page") ?? undefined;
  const q = sp.get("q") ?? "";
  const newKind = sp.get("new");
  const searchMode = q.trim().length > 0;
  const debouncedQ = useDebounced(q, 220);
  const search = useDocSearch(debouncedQ, searchMode);

  const activePath = pagePath;
  const pageQ = useDocPage(mode === "view" ? activePath : undefined);
  const save = useSaveDoc();

  const [drawer, setDrawer] = useState(false);
  const [drawerFilter, setDrawerFilter] = useState("");

  const setParam = (patch: Record<string, string | null>, replace = false) => {
    const next = new URLSearchParams(sp);
    for (const [key, value] of Object.entries(patch)) {
      if (value === null || value === "") next.delete(key);
      else next.set(key, value);
    }
    setSp(next, { replace });
  };

  const openPage = (path: string) => setParam({ page: path, section: null, new: null, title: null });
  const openSection = (path: string) => setParam({ section: path === navigation.path ? null : path, page: null, q: null, new: null, title: null });
  const createInSection = (kind: "page" | "section") => setParam({ new: kind });
  const targetSection = pagePath ? sectionTrail(navigation, pagePath).at(-1)?.path : section?.path;
  const openEditor = (path: string) => navigate(`/docs/edit?page=${encodeURIComponent(path)}`);
  const closeEditor = (path: string) => navigate(`/docs?page=${encodeURIComponent(path)}`);

  const markVerified = async () => {
    const page = pageQ.data;
    if (!page) return;
    try {
      await save.mutateAsync({ path: page.path, content: serializeDoc({ ...fieldsFromPage(page), verified: todayIso() }, page.content), mode: "update" });
      toast(`${page.path}: проверена ${todayIso()}`);
    } catch (error) {
      toast(error instanceof Error ? error.message : String(error), "error");
    }
  };

  if (mode === "edit") {
    if (!pagePath) return <Navigate to="/docs" replace />;
    return (
      <main className="main docs-main">
        <Suspense fallback={<div className="docs-view muted">Загрузка редактора…</div>}>
        <DocEditor
          path={pagePath}
          pages={pages}
          onClose={() => closeEditor(pagePath)}
          onSaved={(path) => {
            toast("Страница сохранена");
            closeEditor(path);
          }}
        />
        </Suspense>
      </main>
    );
  }

  const loading = tree.isPending;

  const results = search.data?.results ?? [];
  const grid = searchMode ? "docs-search-mode" : pagePath ? "view" : "overview";
  const showPage = !!pageQ.data;

  return (
    <main className="main docs-main">
      <ContextSearch className="mobile-context-search mobile-search-bar" />
      <div className={`docs-grid ${grid}`}>
        <div className="docs-tree-col">
          <header className="docs-col-head">
            <button type="button" className="icon-btn m-only" onClick={() => setDrawer(true)} aria-label="Открыть навигацию по документации">
              <Icon.list size={15} />
            </button>
            <h1>Документация</h1>
            {searchMode ? <span className="doc-col-tag">поиск</span> : <span className="count">{loading ? "" : pages.length}</span>}
            <span className="grow" />
            <button type="button" className="btn ghost" onClick={() => setParam({ new: "page" })}>
              <Icon.plus size={12} />
              Статья
            </button>
          </header>


          {loading ? (
            <div className="doc-tree-empty muted">Загрузка…</div>
          ) : tree.isError ? (
            <div className="doc-tree-empty muted">Не удалось загрузить документацию: {tree.error.message}</div>
          ) : searchMode ? (
            <DocSearchList
              navigation={navigation}
              query={q}
              results={results}
              pending={search.isPending || debouncedQ !== q}
              error={search.error}
              selected={activePath}
              onOpen={openPage}
              onCreate={(title) => setParam({ new: "page", title, q: null })}
            />
          ) : (
            <DocsTree pages={pages} roots={roots} section={sectionPath} selected={activePath} onSelect={openPage} onSection={openSection} />
          )}
        </div>

        {tree.isError ? (
          <div className="docs-content"><div className="docs-view muted">Не удалось загрузить пространство документации.</div></div>
        ) : loading ? (
          <div className="docs-content"><div className="docs-view muted">Загрузка пространства…</div></div>
        ) : !pagePath && !searchMode ? (
          roots.length === 0 ? (
            <div className="docs-content"><div className="docs-view muted">Для проекта не настроен корень документации. Администратор может привязать пространство в настройках хранилища знаний.</div></div>
          ) : section ? (
            <DocsOverview root={navigation} section={section} projectName={projectName} onSection={openSection} onOpen={openPage} onCreate={createInSection} onOpenTree={() => setDrawer(true)} />
          ) : (
            <div className="docs-content"><div className="docs-view">
              <p>Раздел не найден в этом проекте.</p>
              <button type="button" className="btn" onClick={() => openSection(navigation.path)}>К обзору пространства</button>
            </div></div>
          )
        ) : searchMode && !results.length ? (
          <div className="docs-content" />
        ) : activePath && pageQ.isPending && !showPage ? (
          <div className="docs-content">
            <div className="docs-view muted">Загрузка страницы…</div>
          </div>
        ) : pageQ.isError ? (
          <div className="docs-content">
            <div className="docs-view muted">Не удалось открыть страницу: {pageQ.error.message}</div>
          </div>
        ) : pageQ.data ? (
          <DocView
            page={pageQ.data}
            navigation={navigation}
            onSection={openSection}
            pages={pages}
            onOpen={openPage}
            onEdit={() => openEditor(pageQ.data!.path)}
            onMarkVerified={() => void markVerified()}
            onOpenTree={() => setDrawer(true)}
            hideAside={searchMode}
          />
        ) : (
          <div className="docs-content">
            <div className="docs-view muted">Выберите статью в результатах поиска.</div>
          </div>
        )}
      </div>

      {drawer && (
        <div className="docs-drawer-overlay" onMouseDown={(event) => event.target === event.currentTarget && setDrawer(false)}>
          <div className="docs-drawer" role="dialog" aria-modal="true" aria-label="Навигация по документации">
            <div className="docs-drawer-grab" />
            <header className="docs-col-head">
              <h1>Документация</h1>
              <span className="count">{pages.length}</span>
              <span className="grow" />
              <button
                type="button"
                className="btn ghost"
                onClick={() => {
                  setDrawer(false);
                  setParam({ new: "page" });
                }}
              >
                <Icon.plus size={12} />
                Статья
              </button>
            </header>
            <label className="docs-filter">
              <Icon.search size={13} />
              <input value={drawerFilter} aria-label="Фильтр статей" placeholder="Фильтр и поиск" onChange={(event) => setDrawerFilter(event.target.value)} autoFocus />
            </label>
            <div className="docs-drawer-tree scroll">
              <DocsTree
                pages={pages}
                roots={roots}
                section={sectionPath}
                onSection={(path) => { openSection(path); setDrawer(false); }}
                selected={activePath}
                onSelect={(path) => {
                  openPage(path);
                  setDrawer(false);
                }}
                filter={drawerFilter}
              />
            </div>
          </div>
        </div>
      )}

      {newKind === "page" || newKind === "note" || newKind === "section" ? (
        <NewDocDialog
          key={`${newKind}:${targetSection}`}
          navigation={navigation}
          initialSection={targetSection}
          initialKind={newKind}
          initialTitle={sp.get("title") ?? ""}
          onClose={() => setParam({ new: null, title: null })}
          onCreated={(path) => {
            setParam({ new: null }, true);
            openEditor(path);
          }}
        />
      ) : null}

    </main>
  );
}
