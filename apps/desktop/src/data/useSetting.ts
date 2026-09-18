import { useEffect, useState } from "react";
import { isDesktop, setSetting, setting } from "../api";

/**
 * A setting that survives restarts.
 *
 * Outside the desktop shell this degrades to in-memory state so demo routes
 * still respond to clicks.
 */
export function useSetting(key: string, fallback: string): [string, (next: string) => void] {
  const [value, setValue] = useState(fallback);

  useEffect(() => {
    if (!isDesktop()) return;
    let alive = true;
    void setting(key).then((stored) => {
      if (alive && stored != null && stored !== "") setValue(stored);
    });
    return () => {
      alive = false;
    };
  }, [key]);

  const update = (next: string) => {
    setValue(next);
    if (isDesktop()) void setSetting(key, next);
  };

  return [value, update];
}

export function useSettingBool(key: string, fallback: boolean): [boolean, (next: boolean) => void] {
  const [raw, setRaw] = useSetting(key, fallback ? "true" : "false");
  return [raw === "true", (next) => setRaw(next ? "true" : "false")];
}
