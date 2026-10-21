import { useSearchParams } from "react-router";
import { Tabs } from "@/shared/ui/kit";
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
    // `Tabs` renders links when `to` is set, so every tab keeps its own URL.
    <div className="st-tabs-band">
      <Tabs label="Подразделы настроек" value={selected} items={tabs.map(([id, label]) => ({ id, label, to: settingsTabHref(scope, section, id) }))} />
    </div>
  );
}
