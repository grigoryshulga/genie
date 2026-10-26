import { useLocation, useNavigate, useSearchParams } from "react-router";
import { searchContext, searchParams } from "@/shared/lib";
import { Icon, IconButton } from "@/shared/ui";

export function ContextSearch({
  inputRef,
  compact = false,
  onExpand,
  onClose,
  className = "",
}: {
  inputRef?: React.RefObject<HTMLInputElement | null>;
  compact?: boolean;
  onExpand?: () => void;
  onClose?: () => void;
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
      <IconButton className="sidebar-search-toggle" label={context.label} onClick={onExpand}>
        <Icon.search size={16} />
      </IconButton>
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
            onClose?.();
          }
        }}
      />
      <kbd className="d-only">/</kbd>
    </label>
  );
}
