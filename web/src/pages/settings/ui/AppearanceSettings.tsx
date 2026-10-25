import { useState } from "react";
import { readAppearancePreferences, saveAppearancePreferences, type AppearancePreferences, type AppearanceTheme } from "@/shared/lib";
import "./appearance.css";

export function AppearanceSettings() {
  const [preferences, setPreferences] = useState(readAppearancePreferences);

  function update(patch: Partial<AppearancePreferences>) {
    const next = { ...preferences, ...patch };
    setPreferences(next);
    saveAppearancePreferences(next);
  }

  return (
    <div className="st-body appearance-settings">
      <div className="st-head">
        <h1 className="g-page-title">Оформление</h1>
      </div>

      <section className="st-card">
        <div className="st-row">
          <div className="lbl">
            <b>Тема</b>
            <span>Выберите тёмные «Чернила», светлую «Бумагу» или следование настройкам системы.</span>
          </div>
          <div className="appearance-themes" role="group" aria-label="Тема интерфейса">
            {(["ink", "paper", "system"] as const).map((theme) => (
              <ThemeChoice key={theme} theme={theme} selected={preferences.theme === theme} onSelect={() => update({ theme })} />
            ))}
          </div>
        </div>

      </section>
    </div>
  );
}

function ThemeChoice({ theme, selected, onSelect }: { theme: AppearanceTheme; selected: boolean; onSelect: () => void }) {
  const label = theme === "paper" ? "Бумага" : theme === "ink" ? "Чернила" : "Системная";
  return (
    <button type="button" className="appearance-theme" aria-pressed={selected} onClick={onSelect}>
      <span className={`appearance-theme-preview ${theme}`} aria-hidden="true">
        <span />
        <span />
      </span>
      <span>{label}</span>
    </button>
  );
}
