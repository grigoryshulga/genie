import { useId, useState, type ReactNode } from "react";
import { Icon } from "@/shared/ui";

/** A task detail section that keeps its heading visible while its contents collapse. */
export function CollapsibleSection({ title, badge, action, children, className = "", open, onOpenChange }: {
  title: string;
  badge?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}) {
  const contentId = useId();
  const [internalOpen, setInternalOpen] = useState(true);
  const isOpen = open ?? internalOpen;
  const toggle = () => {
    const next = !isOpen;
    if (open === undefined) setInternalOpen(next);
    onOpenChange?.(next);
  };

  return (
    <section className={`sec ${className}`.trim()}>
      <div className="sec-head">
        <h3>
          <button type="button" className="sec-toggle" aria-expanded={isOpen} aria-controls={contentId} onClick={toggle}>
            <Icon.chevron size={12} />
            <span>{title}</span>
            {badge}
          </button>
        </h3>
        {action}
      </div>
      <div id={contentId} className="sec-content" hidden={!isOpen}>
        {children}
      </div>
    </section>
  );
}
