// «Управление» lives inside the work shell, under the top bar: a rail grouped into «Общие настройки»,
// «Агенты» and «Администрирование», and the page beside it. Each person sees only the pages they may
// open; the route guards repeat the same rule, so a hidden page stays closed when its URL is typed.
//
// Every hub page wears the same head: the page's name from the rail, then its tabs, then the page. Pages
// do not title themselves; their actions go into the head through `HubActions`.

import { createContext, type ReactNode, Suspense, useContext, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Link, Navigate, Outlet, useLocation, useNavigate } from "react-router";
import { useSession } from "@/entities/session";
import { activeHubItem, canManageProject, canOpen, currentHubTab, HUB_TABS, type HubViewer, visibleHub } from "@/shared/lib";
import { Field, Select, Tabs } from "@/shared/ui";
import { SettingsLoading } from "./SettingsLoading.tsx";
import "@/shared/ui/settings.css";
import "./manage-shell.css";

export function useHubViewer(): HubViewer | undefined {
  const session = useSession().data;
  if (!session) return undefined;
  const project = session.projects.find((p) => p.slug === session.project);
  return { serverAdmin: !!session.user.isAdmin, projectAdmin: canManageProject(!!session.user.isAdmin, project?.role) };
}

export function ManageShell() {
  const viewer = useHubViewer();
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const active = activeHubItem(pathname);
  const groups = viewer ? visibleHub(viewer) : [];
  const [slot, setSlot] = useState<HTMLDivElement | null>(null);
  return (
    <div className="settings-mode st-hub">
      <nav className="st-rail" aria-label="Управление">
        <div className="st-hub-mobile-navigation">
          <Field label="Раздел управления"><Select value={active?.to ?? ""} onChange={(event) => navigate(event.target.value)}>
            {groups.map((group) => <optgroup key={group.id} label={group.label}>{group.items.map((item) => <option key={item.to} value={item.to}>{item.label}</option>)}</optgroup>)}
          </Select></Field>
        </div>
        <div className="st-hub-groups">
          {groups.map((group) => (
            <section key={group.id} className="st-hub-group" aria-labelledby={`st-hub-${group.id}`}>
              <h2 id={`st-hub-${group.id}`} className="st-hub-group-label">{group.label}</h2>
              <div className="g-sheet-nav st-rail-nav">
                {group.items.map((item) => (
                  <Link key={item.id} to={item.to} className={active?.id === item.id ? "on" : undefined} aria-current={active?.id === item.id ? "page" : undefined}>
                    {item.label}
                  </Link>
                ))}
              </div>
            </section>
          ))}
        </div>
      </nav>
      <div className="st-hub-main">
        <header className="topbar page-head st-hub-head">
          <h1 className="g-page-title">{active?.label ?? "Управление"}</h1>
          <span className="grow" />
          <div ref={setSlot} className="st-hub-actions" />
        </header>
        <HubSlot.Provider value={slot}>
          <Suspense fallback={<div className="st-rail-fallback"><SettingsLoading rows={5} /></div>}>
            <Outlet />
          </Suspense>
        </HubSlot.Provider>
      </div>
    </div>
  );
}

const HubSlot = createContext<HTMLElement | null>(null);

/** A hub page's actions, shown at the right of the hub's head. Outside the hub there is no head and nothing renders. */
export function HubActions({ children }: { children: ReactNode }) {
  const slot = useContext(HubSlot);
  if (!slot) return null;
  return createPortal(
    <div className="page-toolbar" role="toolbar" aria-label="Инструменты страницы">
      {children}
    </div>,
    slot,
  );
}

/** A hub page with several views: a band of tabs, a URL each, over the view's own page. */
export function HubTabsSection({ section, label }: { section: keyof typeof HUB_TABS; label: string }) {
  const viewer = useHubViewer();
  const { pathname } = useLocation();
  const band = useRef<HTMLDivElement>(null);
  const tabs = viewer ? HUB_TABS[section].filter((t) => canOpen(t.access, viewer)) : [];
  const current = currentHubTab(tabs, pathname);
  useLayoutEffect(() => {
    const reveal = () => {
      const rail = band.current;
      const active = rail?.querySelector<HTMLElement>('[aria-current="page"]');
      if (!rail || !active) return;
      const bounds = rail.getBoundingClientRect();
      const item = active.getBoundingClientRect();
      const inset = parseFloat(getComputedStyle(rail).paddingLeft) || 0;
      const shift = item.left < bounds.left + inset ? item.left - bounds.left - inset : item.right > bounds.right - inset ? item.right - bounds.right + inset : 0;
      // Move only this band; revealing a tab must preserve the page's vertical position.
      rail.scrollLeft += shift;
    };
    reveal();
    window.addEventListener("resize", reveal);
    return () => window.removeEventListener("resize", reveal);
  }, [current?.id]);
  if (!viewer) return <SettingsLoading rows={3} />;
  // A page this person may not open (a member on the automation rules) falls through to the first one they may.
  if (!current && tabs[0]) return <Navigate to={tabs[0].to} replace />;
  return (
    <div className="st-hub-section">
      {tabs.length > 1 && (
        <div className="st-tabs-band" ref={band}>
          <Tabs label={label} value={current?.id ?? ""} items={tabs.map((t) => ({ id: t.id, label: t.label, to: t.to }))} />
        </div>
      )}
      <Outlet />
    </div>
  );
}
