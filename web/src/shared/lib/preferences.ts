export type Layout = "list" | "board";

type PreferenceStorage = {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
};

function storage(): PreferenceStorage | undefined {
  try {
    return Reflect.get(globalThis, "localStorage") as PreferenceStorage | undefined;
  } catch {
    return undefined;
  }
}

export function readPref<T extends string>(key: string, fallback: T): T {
  try {
    return (storage()?.getItem(key) as T | null) ?? fallback;
  } catch {
    return fallback;
  }
}

export function writePref(key: string, value: string): void {
  try {
    storage()?.setItem(key, value);
  } catch {
    // private mode: preference is not remembered
  }
}
