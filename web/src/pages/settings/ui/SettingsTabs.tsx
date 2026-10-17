import { Link, useSearchParams } from "react-router";
import { SETTINGS_TABS, settingsTab, settingsTabHref, type ManagementScope } from "@/shared/lib";

export function useSettingsTab(scope: ManagementScope, section: string): string {
  const [search] = useSearchParams();
  return settingsTab(scope, section, search.get("tab"));
}

/** URL-backed navigation keeps browser history and deep links useful. */
export function SettingsTabs({ scope, section, current }: { scope: ManagementScope; section: string; current?: string }) {
  const requested = useSettingsTab(scope, section);
  const tabs = SETTINGS_TABS[scope][section];
  if (!tabs || tabs.length < 2) return null;
  const selected = current ?? requested;
  return (
    <nav className="st-tabs" aria-label="Подразделы настроек">
      {tabs.map(([id, label]) => (
        <Link
          key={id}
          to={settingsTabHref(scope, section, id)}
          className={selected === id ? "on" : undefined}
          aria-current={selected === id ? "page" : undefined}
        >
          {label}
        </Link>
      ))}
    </nav>
  );
}
