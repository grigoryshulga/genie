// DocSearchList — the docs search results body (mockups 4–5). The query input
// lives in the column header (DocsPage) so it is not remounted when the column
// switches between the tree and the results.

import { useEffect, useRef } from "react";
import type { DocSearchResult, DocSection } from "@/entities/doc";
import { articleTitle, sectionTrail, DocDiagBadge, DocStaleBadge, DocStatusBadge, DocTypeBadge, snippetParts } from "@/entities/doc";
import { plural } from "@/shared/lib";
import { Icon } from "@/shared/ui";

function Hit({ result, navigation, selected, onOpen }: { result: DocSearchResult; navigation: DocSection; selected: boolean; onOpen: (path: string) => void }) {
  return (
    <button type="button" className={`doc-hit${selected ? " on" : ""}`} onClick={() => onOpen(result.path)} title={result.path}>
      <span className="hit-top">
        <span className="nm">{articleTitle(result)}</span>
        <DocTypeBadge type={result.type} />
        <DocStatusBadge status={result.status} />
        {result.stale && <DocStaleBadge count={result.staleReasons.length} />}
        {result.diagnostics.length > 0 && <DocDiagBadge count={result.diagnostics.length} />}
      </span>
      <span className="hit-path">{sectionTrail(navigation, result.path).slice(1).map((section) => section.title).join(" › ") || "Статьи пространства"}</span>
      <span className="hit-snippet">
        {snippetParts(result.snippet).map((part, i) => (part.hit ? <mark key={i}>{part.text}</mark> : <span key={i}>{part.text}</span>))}
      </span>
    </button>
  );
}

export function DocSearchList({
  navigation,
  query,
  results,
  pending,
  error,
  selected,
  onOpen,
  onCreate,
}: {
  navigation: DocSection;
  query: string;
  results: DocSearchResult[];
  pending: boolean;
  error: Error | null;
  selected: string | undefined;
  onOpen: (path: string) => void;
  onCreate: (title: string) => void;
}) {
  const listRef = useRef<HTMLDivElement>(null);

  // Keep the highlighted hit visible while moving with ↑/↓.
  useEffect(() => {
    listRef.current?.querySelector(".doc-hit.on")?.scrollIntoView({ block: "nearest" });
  }, [selected]);

  return (
    <div className="doc-search" ref={listRef}>
      <div className="doc-search-head">
        <span className="muted">
          {query.trim() ? `${results.length} ${plural(results.length, "страница", "страницы", "страниц")} · по релевантности` : "Введите запрос"}
        </span>
        <span className="grow" />
        <span className="doc-keys muted">
          <kbd>↑↓</kbd> выбрать <kbd>↵</kbd> открыть <kbd>Esc</kbd> к разделам
        </span>
      </div>

      {pending && <div className="doc-search-body muted">Поиск…</div>}
      {error && <div className="doc-search-body muted">Не удалось выполнить поиск: {error.message}</div>}

      {!pending && !error && results.length > 0 && (
        <div className="doc-hits" role="listbox">
          {results.map((result) => (
            <Hit key={result.path} result={result} navigation={navigation} selected={result.path === selected} onOpen={onOpen} />
          ))}
        </div>
      )}

      {!pending && !error && !results.length && (
        <div className="doc-search-body empty">
          <Icon.search size={26} />
          <b>Ничего не найдено по «{query}»</b>
          <p className="muted">
            Поиск идёт по заголовкам, тексту, тегам и алиасам. Попробуйте другое слово или путь к коду, например <span className="mono">src/team</span>.
          </p>
          <button type="button" className="btn" onClick={() => onCreate(query)}>
            + Создать страницу «{query}»
          </button>
        </div>
      )}
    </div>
  );
}
