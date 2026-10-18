import { readPref, writePref } from "./preferences.ts";

export type AppearanceTheme = "dark" | "light";

export const ACCENT_OPTIONS = [
  { id: "violet", label: "Фиолетовый", color: "#5e6ad2" },
  { id: "blue", label: "Синий", color: "#246fd2" },
  { id: "teal", label: "Бирюзовый", color: "#087d75" },
  { id: "green", label: "Зелёный", color: "#2d7d45" },
  { id: "orange", label: "Оранжевый", color: "#b85c10" },
  { id: "rose", label: "Розовый", color: "#bf3f58" },
] as const;

export type AccentColor = (typeof ACCENT_OPTIONS)[number]["id"];

export type AppearancePreferences = {
  theme: AppearanceTheme;
  accent: AccentColor;
};

const THEME_KEY = "genie.appearance.theme";
const ACCENT_KEY = "genie.appearance.accent";
const DEFAULT_APPEARANCE: AppearancePreferences = { theme: "dark", accent: "violet" };

export function readAppearancePreferences(): AppearancePreferences {
  const theme = readPref<string>(THEME_KEY, DEFAULT_APPEARANCE.theme);
  const accent = readPref<string>(ACCENT_KEY, DEFAULT_APPEARANCE.accent);
  return {
    theme: theme === "light" ? "light" : "dark",
    accent: ACCENT_OPTIONS.find((option) => option.id === accent)?.id ?? "violet",
  };
}

export function saveAppearancePreferences(preferences: AppearancePreferences): void {
  writePref(THEME_KEY, preferences.theme);
  writePref(ACCENT_KEY, preferences.accent);
  applyAppearancePreferences(preferences);
}

export function applyAppearancePreferences(preferences: AppearancePreferences): void {
  const page = Reflect.get(globalThis, "document") as {
    documentElement?: { dataset: { theme?: string; accent?: string } };
    querySelector?: (selector: string) => unknown;
  } | undefined;
  const root = page?.documentElement;
  if (!root) return;
  root.dataset.theme = preferences.theme;
  root.dataset.accent = preferences.accent;
  const themeColor = page.querySelector?.('meta[name="theme-color"]') as { content: string } | null | undefined;
  if (themeColor) themeColor.content = preferences.theme === "light" ? "#f0f1f4" : "#0e0f11";
}
