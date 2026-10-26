import { Link, useLocation } from "react-router";
import { useProposals } from "@/entities/platform";

/**
 * Страницы / Предложения: the two sides of the documentation space, in the top row of the docs and
 * proposals screens. Links rather than tabs, because each side is a route with its own URL state.
 */
export function DocViewSwitch() {
  const proposals = useProposals().data?.length ?? 0;
  const { pathname } = useLocation();
  const onProposals = pathname.startsWith("/docs/proposals");
  return (
    <nav className="docs-view-switch" aria-label="Документация и предложения">
      <div className="g-tabs">
        <Link to="/docs" aria-current={onProposals ? undefined : "page"}>
          Страницы
        </Link>
        <Link to="/docs/proposals" aria-current={onProposals ? "page" : undefined}>
          Предложения{proposals > 0 ? ` ${proposals}` : ""}
        </Link>
      </div>
    </nav>
  );
}
