import { useEffect, useState } from "react";
import { isDesktop, setSetting, setting } from "../api";

const CHANGE_EVENT = "kodo:setting-changed";
const values = new Map<string, string>();
type SettingChange = { key: string; value: string };

/**
 * A setting that survives restarts.
 *
 * Outside the desktop shell this degrades to in-memory state so demo routes
 * still respond to clicks.
 */
export function useSetting(key: string, fallback: string): [string, (next: string) => void] {
  const [value, setValue] = useState(() => values.get(key) ?? fallback);

  useEffect(() => {
    let alive = true;
    let changed = false;
    const onChange = (event: Event) => {
      const detail = (event as CustomEvent<SettingChange>).detail;
      if (detail.key !== key) return;
      changed = true;
      setValue(detail.value);
    };
    window.addEventListener(CHANGE_EVENT, onChange);
    if (isDesktop()) {
      void setting(key).then((stored) => {
        // A late startup read must not overwrite a choice already made here.
        if (alive && !changed && stored != null && stored !== "") {
          values.set(key, stored);
          setValue(stored);
        }
      });
    }
    return () => {
      alive = false;
      window.removeEventListener(CHANGE_EVENT, onChange);
    };
  }, [key]);

  const update = (next: string) => {
    values.set(key, next);
    setValue(next);
    window.dispatchEvent(new CustomEvent<SettingChange>(CHANGE_EVENT, { detail: { key, value: next } }));
    if (isDesktop()) void setSetting(key, next);
  };

  return [value, update];
}

export function useSettingBool(key: string, fallback: boolean): [boolean, (next: boolean) => void] {
  const [raw, setRaw] = useSetting(key, fallback ? "true" : "false");
  return [raw === "true", (next) => setRaw(next ? "true" : "false")];
}
