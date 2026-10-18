import { useEffect, useState } from "react";
import { Sidebar } from "./Sidebar.tsx";
import { Icon } from "@/shared/ui";

type Props = React.ComponentProps<typeof Sidebar>;

export function MobileHeader(props: Props) {
  const [open, setOpen] = useState(false);

  useEffect(() => {
    if (!open) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [open]);

  return (
    <>
      <header className="mobile-header m-only">
        <button
          type="button"
          className="icon-btn"
          aria-label="Открыть меню разделов"
          aria-expanded={open}
          aria-controls="mobile-navigation-drawer"
          onClick={() => setOpen(true)}
        >
          <Icon.menu size={19} />
        </button>
        <span className="mobile-header-title">genie</span>
      </header>
      {open && (
        <div className="mobile-drawer-layer m-only">
          <button className="mobile-drawer-backdrop" type="button" aria-label="Закрыть меню" onClick={() => setOpen(false)} />
          <div id="mobile-navigation-drawer" className="mobile-drawer-panel" role="dialog" aria-modal="true" aria-label="Навигация">
            <div className="mobile-drawer-heading">
              <span>Навигация</span>
              <button type="button" className="icon-btn" aria-label="Закрыть меню" onClick={() => setOpen(false)}>
                <Icon.close size={17} />
              </button>
            </div>
            <div className="mobile-drawer-nav" onClick={(event) => {
              if ((event.target as HTMLElement).closest("a")) setOpen(false);
            }}>
              <Sidebar {...props} onNew={() => { setOpen(false); props.onNew(); }} />
            </div>
          </div>
        </div>
      )}
    </>
  );
}
