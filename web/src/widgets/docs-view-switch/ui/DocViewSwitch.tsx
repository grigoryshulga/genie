import { Link, useLocation } from "react-router";
import { attentionChangesets, useChangesets } from "@/entities/doc";

/**
 * Страницы / Изменения: the two sides of the documentation space, in the top row of the docs and
 * changes screens. Links rather than tabs, because each side is a route with its own URL state.
 */
export function DocViewSwitch() {
  const changes = attentionChangesets(useChangesets({ view: "all" }).data ?? []).length;
  const { pathname } = useLocation();
  const onChanges = pathname.startsWith("/docs/changes");
  return (
    <nav className="docs-view-switch" aria-label="Документация и изменения">
      <div className="g-tabs">
        <Link to="/docs" aria-current={onChanges ? undefined : "page"}>
          Страницы
        </Link>
        <Link to="/docs/changes" aria-current={onChanges ? "page" : undefined}>
          Изменения{changes > 0 ? ` ${changes}` : ""}
        </Link>
      </div>
    </nav>
  );
}
