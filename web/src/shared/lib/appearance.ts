import { readPref, writePref } from "./preferences.ts";

export type AppearanceTheme = "dark" | "light" | "system";
type ResolvedAppearanceTheme = Exclude<AppearanceTheme, "system">;

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

type MediaChange = { matches: boolean };
type SystemThemeQuery = {
  matches: boolean;
  addEventListener?: (type: "change", listener: (event: MediaChange) => void) => void;
  removeEventListener?: (type: "change", listener: (event: MediaChange) => void) => void;
  addListener?: (listener: (event: MediaChange) => void) => void;
  removeListener?: (listener: (event: MediaChange) => void) => void;
};
type AppearancePage = {
  documentElement?: { dataset: { theme?: string; themePreference?: string; accent?: string } };
  querySelector?: (selector: string) => unknown;
};

let activeSystemQuery: SystemThemeQuery | undefined;
let activeSystemListener: ((event: MediaChange) => void) | undefined;

export function readAppearancePreferences(): AppearancePreferences {
  const theme = readPref<string>(THEME_KEY, DEFAULT_APPEARANCE.theme);
  const accent = readPref<string>(ACCENT_KEY, DEFAULT_APPEARANCE.accent);
  return {
    theme: theme === "light" || theme === "system" ? theme : "dark",
    accent: ACCENT_OPTIONS.find((option) => option.id === accent)?.id ?? "violet",
  };
}

export function saveAppearancePreferences(preferences: AppearancePreferences): void {
  writePref(THEME_KEY, preferences.theme);
  writePref(ACCENT_KEY, preferences.accent);
  applyAppearancePreferences(preferences);
}

export function applyAppearancePreferences(preferences: AppearancePreferences): void {
  const page = Reflect.get(globalThis, "document") as AppearancePage | undefined;
  const root = page?.documentElement;
  if (!root) return;

  if (activeSystemQuery && activeSystemListener) {
    if (activeSystemQuery.removeEventListener) activeSystemQuery.removeEventListener("change", activeSystemListener);
    else activeSystemQuery.removeListener?.(activeSystemListener);
  }
  activeSystemQuery = undefined;
  activeSystemListener = undefined;

  const matchMedia = Reflect.get(globalThis, "matchMedia");
  let systemQuery: SystemThemeQuery | undefined;
  if (preferences.theme === "system" && typeof matchMedia === "function") {
    try {
      systemQuery = matchMedia.call(globalThis, "(prefers-color-scheme: dark)") as SystemThemeQuery;
    } catch {
      // Keep the system choice usable when media queries are unavailable.
    }
  }
  const resolvedTheme: ResolvedAppearanceTheme = preferences.theme === "system"
    ? (systemQuery?.matches ? "dark" : "light")
    : preferences.theme;

  root.dataset.themePreference = preferences.theme;
  root.dataset.theme = resolvedTheme;
  root.dataset.accent = preferences.accent;
  const themeColor = page.querySelector?.('meta[name="theme-color"]') as { content: string } | null | undefined;
  const setResolvedTheme = (theme: ResolvedAppearanceTheme) => {
    root.dataset.theme = theme;
    if (themeColor) themeColor.content = theme === "light" ? "#f0f1f4" : "#0e0f11";
  };
  setResolvedTheme(resolvedTheme);

  if (systemQuery) {
    activeSystemQuery = systemQuery;
    activeSystemListener = (event) => setResolvedTheme(event.matches ? "dark" : "light");
    if (systemQuery.addEventListener) systemQuery.addEventListener("change", activeSystemListener);
    else systemQuery.addListener?.(activeSystemListener);
  }
}
