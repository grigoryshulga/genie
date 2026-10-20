import { readPref, writePref } from "./preferences.ts";

export type AppearanceTheme = "ink" | "paper" | "system";
type ResolvedAppearanceTheme = Exclude<AppearanceTheme, "system">;

export type AppearancePreferences = {
  theme: AppearanceTheme;
};

const THEME_KEY = "genie.appearance.theme";
const DEFAULT_APPEARANCE: AppearancePreferences = { theme: "ink" };
// The color of the browser chrome (theme-color) for each resolved theme: ink and paper from the design tokens.
const THEME_COLORS: Record<ResolvedAppearanceTheme, string> = { ink: "#09090d", paper: "#f4f1ea" };

type MediaChange = { matches: boolean };
type SystemThemeQuery = {
  matches: boolean;
  addEventListener?: (type: "change", listener: (event: MediaChange) => void) => void;
  removeEventListener?: (type: "change", listener: (event: MediaChange) => void) => void;
  addListener?: (listener: (event: MediaChange) => void) => void;
  removeListener?: (listener: (event: MediaChange) => void) => void;
};
type AppearancePage = {
  documentElement?: { dataset: { theme?: string; themePreference?: string } };
  querySelector?: (selector: string) => unknown;
};

let activeSystemQuery: SystemThemeQuery | undefined;
let activeSystemListener: ((event: MediaChange) => void) | undefined;

export function readAppearancePreferences(): AppearancePreferences {
  const theme = readPref<string>(THEME_KEY, DEFAULT_APPEARANCE.theme);
  // "dark" and "light" are the names the earlier appearance settings stored.
  if (theme === "paper" || theme === "light") return { theme: "paper" };
  if (theme === "system") return { theme: "system" };
  return { theme: "ink" };
}

export function saveAppearancePreferences(preferences: AppearancePreferences): void {
  writePref(THEME_KEY, preferences.theme);
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
    ? (systemQuery?.matches ? "ink" : "paper")
    : preferences.theme;

  root.dataset.themePreference = preferences.theme;
  root.dataset.theme = resolvedTheme;
  const themeColor = page.querySelector?.('meta[name="theme-color"]') as { content: string } | null | undefined;
  const setResolvedTheme = (theme: ResolvedAppearanceTheme) => {
    root.dataset.theme = theme;
    if (themeColor) themeColor.content = THEME_COLORS[theme];
  };
  setResolvedTheme(resolvedTheme);

  if (systemQuery) {
    activeSystemQuery = systemQuery;
    activeSystemListener = (event) => setResolvedTheme(event.matches ? "ink" : "paper");
    if (systemQuery.addEventListener) systemQuery.addEventListener("change", activeSystemListener);
    else systemQuery.addListener?.(activeSystemListener);
  }
}
