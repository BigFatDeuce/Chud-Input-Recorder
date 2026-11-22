// Windows does not show keys typed into Chud's own window to the global
// keyboard hook, so the window forwards them itself: global hotkeys and the
// live view keep working while Chud is in front. They are never recorded
// (recordings exclude Chud's window), and the backend ignores a key the hook
// already reported. Key pickers stop the event before it gets here, so
// choosing F9 as a shortcut does not start a recording.
import { useEffect } from "react";
import { api } from "./api";
import { keyFromDom } from "./keys";

/** Alt or F10 alone would put the frameless window in menu mode (frozen until Esc). */
function opensSystemMenu(e: KeyboardEvent): boolean {
  if (e.ctrlKey || e.shiftKey || e.metaKey) return false;
  return e.key === "Alt" || (e.key === "F10" && !e.altKey);
}

export function useLocalKeyBridge() {
  useEffect(() => {
    const forward = (down: boolean) => (e: KeyboardEvent) => {
      if (opensSystemMenu(e)) e.preventDefault();
      if (down && e.repeat) return;
      const key = keyFromDom(e);
      if (key) void api.localKey(key, down).catch(() => {});
    };
    const onDown = forward(true);
    const onUp = forward(false);
    window.addEventListener("keydown", onDown);
    window.addEventListener("keyup", onUp);
    return () => {
      window.removeEventListener("keydown", onDown);
      window.removeEventListener("keyup", onUp);
    };
  }, []);
}
