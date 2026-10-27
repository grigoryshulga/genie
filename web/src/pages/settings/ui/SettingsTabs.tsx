import { Link, useSearchParams } from "react-router";
import { Icon, Tabs } from "@/shared/ui";
import { isActionTab, SETTINGS_TABS, settingsTab, settingsTabHref, type ManagementScope } from "@/shared/lib";
import { HubActions } from "@/widgets/settings-shell";

export function useSettingsTab(scope: ManagementScope, section: string): string {
  const [search] = useSearchParams();
  return settingsTab(scope, section, search.get("tab"));
}

/** URL-backed navigation keeps browser history and deep links useful. Creating views («Новый проект») are
    a button in the hub's head rather than a tab; while one is open, the button leads back to the list. */
export function SettingsTabs({ scope, section, current }: { scope: ManagementScope; section: string; current?: string }) {
  const requested = useSettingsTab(scope, section);
  const all = SETTINGS_TABS[scope][section] ?? [];
  const selected = current ?? requested;
  const tabs = all.filter(([id]) => !isActionTab(scope, section, id));
  const actions = all.filter(([id]) => isActionTab(scope, section, id));
  const acting = isActionTab(scope, section, selected);
  return (
    <>
      {actions.length > 0 && (
        <HubActions>
          {acting ? (
            <Link className="g-btn secondary" to={settingsTabHref(scope, section, tabs[0]?.[0] ?? "")}>
              Отмена
            </Link>
          ) : (
            actions.map(([id, label]) => (
              <Link key={id} className="g-btn secondary" to={settingsTabHref(scope, section, id)}>
                <Icon.plus size={14} />
                {label}
              </Link>
            ))
          )}
        </HubActions>
      )}
      {/* `Tabs` renders links when `to` is set, so every tab keeps its own URL. An open form hides the band. */}
      {tabs.length > 1 && !acting && (
        <div className="st-tabs-band">
          <Tabs label="Подразделы настроек" value={selected} items={tabs.map(([id, label]) => ({ id, label, to: settingsTabHref(scope, section, id) }))} />
        </div>
      )}
    </>
  );
}
