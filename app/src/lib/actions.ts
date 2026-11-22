// High level actions shared by the transport bar, menus and shortcuts.
import { api } from "./api";
import { keyLabel } from "./keys";
import { useStore } from "./store";
import type { Hotkey, HotkeyAction, Selection } from "./types";

export function hotkeyText(h: Hotkey | undefined): string {
  if (!h || !h.enabled) return "";
  const parts: string[] = [];
  if (h.modifiers.ctrl) parts.push("Ctrl");
  if (h.modifiers.shift) parts.push("Shift");
  if (h.modifiers.alt) parts.push("Alt");
  if (h.modifiers.meta) parts.push("Win");
  parts.push(keyLabel(h.key));
  return parts.join("+");
}

export function hotkeyFor(action: HotkeyAction): string {
  const hk = useStore.getState().settings?.hotkeys.find((h) => JSON.stringify(h.action) === JSON.stringify(action));
  return hotkeyText(hk);
}

/** What "Play" plays: the open document, else the selected library macro. */
export function playTarget(): string | null {
  const s = useStore.getState();
  if ((s.view === "studio" || s.view === "flow") && s.activeDoc) return s.activeDoc;
  if (s.view === "library" && s.libSelection.length === 1) return s.libSelection[0];
  return s.activeDoc ?? s.status.armed ?? (s.libSelection.length === 1 ? s.libSelection[0] : null);
}

export async function startRecording() {
  const s = useStore.getState();
  try {
    await api.recordStart(s.settings?.record);
  } catch (e) {
    s.fail(e, "error.record");
  }
}

export async function toggleRecording() {
  const s = useStore.getState();
  if (s.status.mode === "recording" || s.status.mode === "countdown") await api.recordStop();
  else if (s.status.mode === "idle") await startRecording();
}

/** Whether a macro has an automation flow (open document first, else the library). */
export function hasFlow(id: string): boolean {
  const s = useStore.getState();
  const doc = s.docs[id];
  if (doc) return Boolean(doc.flow?.blocks.length);
  return Boolean(s.listing?.entries.find((e) => e.id === id)?.manifest.summary.has_flow);
}

export async function playMacro(id: string, selection?: Selection, useFlow?: boolean) {
  const s = useStore.getState();
  try {
    // Unsaved settings of an open document are already on the backend.
    await api.play(id, { selection, useFlow: useFlow ?? (selection ? false : hasFlow(id)) });
  } catch (e) {
    s.fail(e, "error.play");
  }
}

export async function togglePlay() {
  const s = useStore.getState();
  if (s.status.mode === "playing") {
    await api.playStop();
    return;
  }
  const id = playTarget();
  if (id) await playMacro(id);
}

export async function togglePause() {
  await api.playPause();
}

export async function emergencyStop() {
  await api.emergencyStop();
}
