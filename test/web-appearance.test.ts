import assert from "node:assert/strict";
import { test } from "node:test";
import { readAppearancePreferences, saveAppearancePreferences } from "../web/src/shared/lib/appearance.ts";

test("appearance preferences default safely and persist changes", () => {
  const values = new Map<string, string>();
  const themeColor = { content: "#09090d" };
  const document = {
    documentElement: { dataset: {} as Record<string, string> },
    querySelector: () => themeColor,
  };
  const previousStorage = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
  const previousDocument = Object.getOwnPropertyDescriptor(globalThis, "document");
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    },
  });
  Object.defineProperty(globalThis, "document", { configurable: true, value: document });

  try {
    assert.deepEqual(readAppearancePreferences(), { theme: "ink" });

    const preferences = { theme: "paper" } as const;
    saveAppearancePreferences(preferences);

    assert.deepEqual(readAppearancePreferences(), preferences);
    assert.deepEqual(document.documentElement.dataset, { theme: "paper", themePreference: "paper" });
    assert.equal(themeColor.content, "#f4f1ea");
  } finally {
    if (previousStorage) Object.defineProperty(globalThis, "localStorage", previousStorage);
    else Reflect.deleteProperty(globalThis, "localStorage");
    if (previousDocument) Object.defineProperty(globalThis, "document", previousDocument);
    else Reflect.deleteProperty(globalThis, "document");
  }
});

test("system appearance follows operating system color scheme changes", () => {
  const values = new Map<string, string>();
  const themeColor = { content: "#09090d" };
  const document = {
    documentElement: { dataset: {} as Record<string, string> },
    querySelector: () => themeColor,
  };
  const previousStorage = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
  const previousDocument = Object.getOwnPropertyDescriptor(globalThis, "document");
  const previousMatchMedia = Object.getOwnPropertyDescriptor(globalThis, "matchMedia");
  let onChange: ((event: { matches: boolean }) => void) | undefined;
  const mediaQuery = {
    matches: true,
    addEventListener: (_type: "change", listener: (event: { matches: boolean }) => void) => { onChange = listener; },
    removeEventListener: () => { onChange = undefined; },
  };

  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
    },
  });
  Object.defineProperty(globalThis, "document", { configurable: true, value: document });
  Object.defineProperty(globalThis, "matchMedia", { configurable: true, value: () => mediaQuery });

  try {
    const preferences = { theme: "system" } as const;
    saveAppearancePreferences(preferences);

    assert.deepEqual(readAppearancePreferences(), preferences);
    assert.equal(document.documentElement.dataset.themePreference, "system");
    assert.equal(document.documentElement.dataset.theme, "ink");
    assert.equal(themeColor.content, "#09090d");

    onChange?.({ matches: false });

    assert.equal(document.documentElement.dataset.theme, "paper");
    assert.equal(themeColor.content, "#f4f1ea");
  } finally {
    // Remove the test's media-query listener while the mocked document is still active.
    saveAppearancePreferences({ theme: "ink" });
    if (previousStorage) Object.defineProperty(globalThis, "localStorage", previousStorage);
    else Reflect.deleteProperty(globalThis, "localStorage");
    if (previousDocument) Object.defineProperty(globalThis, "document", previousDocument);
    else Reflect.deleteProperty(globalThis, "document");
    if (previousMatchMedia) Object.defineProperty(globalThis, "matchMedia", previousMatchMedia);
    else Reflect.deleteProperty(globalThis, "matchMedia");
  }
});

test("unsupported stored appearance values fall back to ink, and earlier names migrate", () => {
  const values = new Map([["genie.appearance.theme", "sepia"]]);
  const previousStorage = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => values.set(key, value) },
  });

  try {
    assert.deepEqual(readAppearancePreferences(), { theme: "ink" });
    values.set("genie.appearance.theme", "light");
    assert.deepEqual(readAppearancePreferences(), { theme: "paper" });
    values.set("genie.appearance.theme", "dark");
    assert.deepEqual(readAppearancePreferences(), { theme: "ink" });
  } finally {
    if (previousStorage) Object.defineProperty(globalThis, "localStorage", previousStorage);
    else Reflect.deleteProperty(globalThis, "localStorage");
  }
});
