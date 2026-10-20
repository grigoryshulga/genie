import { useLocation, useNavigate, useSearchParams } from "react-router";
import { searchContext, searchParams } from "@/shared/lib";
import { Icon } from "@/shared/ui";

export function ContextSearch({
  inputRef,
  compact = false,
  onExpand,
  className = "",
}: {
  inputRef?: React.RefObject<HTMLInputElement | null>;
  compact?: boolean;
  onExpand?: () => void;
  className?: string;
}) {
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const context = searchContext(pathname);
  const value = context.path === pathname ? (params.get("q") ?? "") : "";
  const change = (query: string) => {
    if (context.path === pathname) setParams(searchParams(params, query), { replace: true });
    else navigate({ pathname: context.path, search: searchParams(new URLSearchParams(), query).toString() });
  };
  if (compact)
    return (
      <button type="button" className="icon-btn sidebar-search-toggle" aria-label={context.label} title={context.label} onClick={onExpand}>
        <Icon.search size={16} />
      </button>
    );
  return (
    <label className={`search context-search ${className}`}>
      <Icon.search size={14} />
      <input
        ref={inputRef}
        type="search"
        placeholder={context.label}
        aria-label={context.label}
        value={value}
        onChange={(e) => change(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            change("");
            e.currentTarget.blur();
          }
        }}
      />
      <kbd className="d-only">/</kbd>
    </label>
  );
}
