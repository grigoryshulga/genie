// DocsPage — `/docs` and `/docs/edit`. Owns the URL state (page, q, new), the
// column layout (tree | page | aside, search, editor, empty state), the mobile
// drawer and the docs change signal.

import { lazy, Suspense, useMemo, useRef, useState } from "react";
import { Navigate, useLocation, useNavigate, useSearchParams } from "react-router";
import {
  buildDocNavigation,
  docSections,
  sectionLandingPaths,
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
import { DocViewSwitch } from "@/widgets/docs-view-switch";
import { useToast, Banner, EmptyState, ErrorState, Icon, LoadingState, Skeleton, useModalFocus } from "@/shared/ui";
import { DocsOverview } from "./DocsOverview.tsx";
import { DocSearchList } from "./DocSearchList.tsx";
import { DocView } from "./DocView.tsx";
import { NewDocDialog } from "./NewDocDialog.tsx";
// The docs workspace: the reader/editor layout, the overview cards and the mobile drawer, then the
// responsive and centring layers. The changes bar and history share `changes.css`.
import "./docs-sheets.css";
import "./changes.css";

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
  // The mobile drawer is the tree on a phone: it covers the page, so it takes focus, keeps Tab inside,
  // closes on Escape and hands focus back to the button that opened it.
  const drawerRef = useRef<HTMLDivElement>(null);
  useModalFocus(drawerRef, drawer, () => setDrawer(false));

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
        <Suspense fallback={<div className="docs-view"><LoadingState label="Загрузка редактора" lines={4} /></div>}>
        <DocEditor
          path={pagePath}
          pages={pages}
          onSaved={(path, changeset) => {
            toast(changeset ? `Сохранено в черновике изменений #${changeset}` : "Страница сохранена");
            closeEditor(path);
          }}
        />
        </Suspense>
      </main>
    );
  }

  const loading = tree.isPending;

  const landings = sectionLandingPaths(navigation);
  const results = (search.data?.results ?? []).filter((result) => !landings.has(result.path));
  const grid = searchMode ? "docs-search-mode" : pagePath ? "view" : "overview";
  const showPage = !!pageQ.data;
  // The Страницы/Изменения switch lives in the top row of the page views, not in the navigation column.
  const overview = !tree.isError && !loading && !pagePath && !searchMode && roots.length > 0 && !!section;

  return (
    <main className="main docs-main">

      <div className={`docs-grid ${grid}`}>
        <div className="docs-tree-col">
          <header className="docs-col-head side-head">
            <button type="button" className="g-icon-btn m-only" onClick={() => setDrawer(true)} aria-label="Открыть навигацию по документации">
              <Icon.list size={15} />
            </button>
            <h1 className="g-page-title">Знания</h1>
            {searchMode && <span className="doc-col-tag">поиск</span>}
          </header>

          {loading ? (
            <div className="doc-tree-empty" aria-busy="true">
              <Skeleton width="82%" height={20} />
              <Skeleton width="64%" height={20} />
              <Skeleton width="74%" height={20} />
              <Skeleton width="58%" height={20} />
              <Skeleton width="70%" height={20} />
            </div>
          ) : tree.isError ? (
            <div className="doc-tree-empty">
              <Banner tone="danger" title="Не удалось загрузить документацию">
                {tree.error.message}
              </Banner>
            </div>
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
          <div className="docs-content"><div className="docs-view"><ErrorState message={tree.error.message} onRetry={() => void tree.refetch()} /></div></div>
        ) : loading ? (
          <div className="docs-content"><div className="docs-view" aria-busy="true">
            <div className="doc-head">
              <Skeleton width="12ch" height={20} />
              <Skeleton width="70%" height={36} />
              <Skeleton width="40%" height={20} />
            </div>
            <Skeleton width="100%" height={16} />
            <Skeleton width="92%" height={16} />
            <Skeleton width="96%" height={16} />
            <Skeleton width="60%" height={16} />
          </div></div>
        ) : !pagePath && !searchMode ? (
          roots.length === 0 ? (
            <div className="docs-content"><div className="docs-view">
              <EmptyState title="Пространство документации не настроено">
                Администратор может привязать корень документации в настройках хранилища знаний. Пока корня нет, страницы читать нечего.
              </EmptyState>
            </div></div>
          ) : section ? (
            <DocsOverview key={section.path} root={navigation} section={section} toolbar={<DocViewSwitch />} onSection={openSection} onOpen={openPage} onCreate={createInSection} onOpenTree={() => setDrawer(true)} />
          ) : (
            <div className="docs-content"><div className="docs-view">
              <EmptyState
                title="Раздел не найден в этом проекте"
                action={<button type="button" className="g-btn secondary" onClick={() => openSection(navigation.path)}>К обзору пространства</button>}
              >
                Возможно, раздел переименовали или удалили. Обзор пространства покажет, что есть сейчас.
              </EmptyState>
            </div></div>
          )
        ) : searchMode && !results.length ? (
          <div className="docs-content" />
        ) : activePath && pageQ.isPending && !showPage ? (
          <div className="docs-content">
            <div className="docs-view" aria-busy="true">
              <div className="doc-head">
                <Skeleton width="10ch" height={20} />
                <Skeleton width="62%" height={36} />
                <Skeleton width="34%" height={20} />
              </div>
              <Skeleton width="100%" height={16} />
              <Skeleton width="88%" height={16} />
              <Skeleton width="94%" height={16} />
              <Skeleton width="52%" height={16} />
            </div>
          </div>
        ) : pageQ.isError ? (
          <div className="docs-content">
            <div className="docs-view">
              <ErrorState message={`Не удалось открыть страницу: ${pageQ.error.message}`} onRetry={() => void pageQ.refetch()} />
            </div>
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
            <div className="docs-view">
              <EmptyState title="Выберите статью в результатах поиска">Результаты слева: стрелки выбирают, Enter открывает.</EmptyState>
            </div>
          </div>
        )}
      </div>

      {drawer && (
        <div className="docs-drawer-overlay" onMouseDown={(event) => event.target === event.currentTarget && setDrawer(false)}>
          <div ref={drawerRef} className="docs-drawer" role="dialog" aria-modal="true" aria-label="Навигация по документации">
            <div className="docs-drawer-grab" />
            <header className="docs-col-head side-head">
              <span className="side-title">Знания</span>
              <span className="count">{navigation.count}</span>
              <span className="grow" />
              <button
                type="button"
                className="g-btn ghost"
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
              {/* No autoFocus: the drawer's own focus trap puts the keyboard inside, and a field that
                  grabs focus during mount would leave the trap without an opener to restore to. */}
              <input value={drawerFilter} aria-label="Фильтр статей" placeholder="Фильтр и поиск" onChange={(event) => setDrawerFilter(event.target.value)} />
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
