import { type ReactNode, useId, useState } from "react";
import { readPref, writePref } from "@/shared/lib";
import { Icon } from "../icons.tsx";
import { cx } from "./cx.ts";
import "./disclosure-card.css";

/** A compact rail card; the whole heading toggles its body and remembers the choice. */
export function DisclosureCard({ title, icon, badge, storageKey, defaultOpen = true, children }: { title: string; icon?: ReactNode; badge?: ReactNode; storageKey: string; defaultOpen?: boolean; children: ReactNode }) {
  const id = useId();
  const [open, setOpen] = useState(() => readPref(storageKey, defaultOpen ? "open" : "closed") === "open");
  return (
    <section className={cx("g-disclosure-card", !open && "collapsed")}>
      <h3>
        <button type="button" className="g-disclosure-toggle" id={`${id}-heading`} aria-expanded={open} aria-controls={`${id}-body`} onClick={() => {
          writePref(storageKey, open ? "closed" : "open");
          setOpen(!open);
        }}>
          {icon}<span>{title}</span>{badge}<Icon.chevron size={12} />
        </button>
      </h3>
      <div className="g-disclosure-body" id={`${id}-body`} aria-labelledby={`${id}-heading`} hidden={!open}>{children}</div>
    </section>
  );
}
