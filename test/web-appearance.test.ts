import assert from "node:assert/strict";
import { test } from "node:test";
import { readAppearancePreferences, saveAppearancePreferences } from "../web/src/shared/lib/appearance.ts";

test("appearance preferences default safely and persist changes", () => {
  const values = new Map<string, string>();
  const themeColor = { content: "#0e0f11" };
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
    assert.deepEqual(readAppearancePreferences(), { theme: "dark", accent: "violet" });

    const preferences = { theme: "light", accent: "teal" } as const;
    saveAppearancePreferences(preferences);

    assert.deepEqual(readAppearancePreferences(), preferences);
    assert.deepEqual(document.documentElement.dataset, { theme: "light", accent: "teal" });
    assert.equal(themeColor.content, "#f0f1f4");
  } finally {
    if (previousStorage) Object.defineProperty(globalThis, "localStorage", previousStorage);
    else Reflect.deleteProperty(globalThis, "localStorage");
    if (previousDocument) Object.defineProperty(globalThis, "document", previousDocument);
    else Reflect.deleteProperty(globalThis, "document");
  }
});

test("unsupported stored appearance values fall back to dark violet", () => {
  const values = new Map([
    ["genie.appearance.theme", "sepia"],
    ["genie.appearance.accent", "rainbow"],
  ]);
  const previousStorage = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => values.set(key, value) },
  });

  try {
    assert.deepEqual(readAppearancePreferences(), { theme: "dark", accent: "violet" });
  } finally {
    if (previousStorage) Object.defineProperty(globalThis, "localStorage", previousStorage);
    else Reflect.deleteProperty(globalThis, "localStorage");
  }
});
