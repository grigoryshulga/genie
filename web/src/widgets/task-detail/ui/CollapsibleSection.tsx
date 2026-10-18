import { useId, useState, type ReactNode } from "react";
import { Icon } from "@/shared/ui";

/** A task detail section that keeps its heading visible while its contents collapse. */
export function CollapsibleSection({ title, badge, action, children, className = "" }: {
  title: string;
  badge?: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  const contentId = useId();
  const [open, setOpen] = useState(true);

  return (
    <section className={`sec ${className}`.trim()}>
      <div className="sec-head">
        <h3>
          <button type="button" className="sec-toggle" aria-expanded={open} aria-controls={contentId} onClick={() => setOpen((value) => !value)}>
            <Icon.chevron size={12} />
            <span>{title}</span>
            {badge}
          </button>
        </h3>
        {action}
      </div>
      <div id={contentId} className="sec-content" hidden={!open}>
        {children}
      </div>
    </section>
  );
}
