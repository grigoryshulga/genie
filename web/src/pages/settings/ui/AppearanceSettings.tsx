import { useState } from "react";
import { ACCENT_OPTIONS, readAppearancePreferences, saveAppearancePreferences, type AppearancePreferences, type AppearanceTheme, type AccentColor } from "@/shared/lib";
import { Icon } from "@/shared/ui";
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
        <h2>Оформление</h2>
        <p>Эти настройки сохраняются в браузере и влияют только на этот браузер.</p>
      </div>

      <section className="st-card">
        <div className="st-row">
          <div className="lbl">
            <b>Тема</b>
            <span>Выберите светлое, тёмное оформление или следование настройкам системы.</span>
          </div>
          <div className="appearance-themes" role="group" aria-label="Тема интерфейса">
            {(["light", "dark", "system"] as const).map((theme) => (
              <ThemeChoice key={theme} theme={theme} selected={preferences.theme === theme} onSelect={() => update({ theme })} />
            ))}
          </div>
        </div>

        <div className="st-row">
          <div className="lbl">
            <b>Акцентный цвет</b>
            <span>Используется для ссылок, выделения и основных действий.</span>
          </div>
          <div className="appearance-accents" role="group" aria-label="Акцентный цвет">
            {ACCENT_OPTIONS.map((option) => (
              <AccentChoice
                key={option.id}
                accent={option.id}
                label={option.label}
                color={option.color}
                selected={preferences.accent === option.id}
                onSelect={() => update({ accent: option.id })}
              />
            ))}
          </div>
        </div>
      </section>
    </div>
  );
}

function ThemeChoice({ theme, selected, onSelect }: { theme: AppearanceTheme; selected: boolean; onSelect: () => void }) {
  const label = theme === "light" ? "Светлая" : theme === "dark" ? "Тёмная" : "Системная";
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

function AccentChoice({ accent, label, color, selected, onSelect }: { accent: AccentColor; label: string; color: string; selected: boolean; onSelect: () => void }) {
  return (
    <button type="button" className="appearance-accent" aria-label={label} aria-pressed={selected} onClick={onSelect}>
      <span className="appearance-accent-dot" style={{ backgroundColor: color }} aria-hidden="true">
        {selected && <Icon.check size={13} />}
      </span>
      <span>{label}</span>
      <span className="sr-only">{accent}</span>
    </button>
  );
}
