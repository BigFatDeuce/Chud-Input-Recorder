// "Interface size": zooms the whole interface on top of the system display
// scaling (applied by the backend as the webview zoom, so the layout's
// breakpoints follow it). Ctrl+= / Ctrl+- / Ctrl+0 like a browser.
import { useEffect } from "react";
import { useStore } from "./store";

export const UI_SCALES = [0.8, 0.9, 1, 1.1, 1.25, 1.5] as const;

/** The next step up (+1) or down (-1) from `current`. */
export function stepScale(current: number, dir: 1 | -1): number {
  if (dir > 0) return UI_SCALES.find((s) => s > current + 0.001) ?? UI_SCALES[UI_SCALES.length - 1];
  return [...UI_SCALES].reverse().find((s) => s < current - 0.001) ?? UI_SCALES[0];
}

export function useZoomShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
      const { settings, saveSettings } = useStore.getState();
      if (!settings) return;
      let next: number | null = null;
      if (e.key === "=" || e.key === "+") next = stepScale(settings.ui_scale, 1);
      else if (e.key === "-" || e.key === "_") next = stepScale(settings.ui_scale, -1);
      else if (e.key === "0" && e.code !== "Numpad0") next = 1;
      if (next === null) return;
      e.preventDefault();
      if (next !== settings.ui_scale) void saveSettings({ ...settings, ui_scale: next });
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
