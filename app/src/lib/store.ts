// Global application state.
import { create } from "zustand";
import { api, errorText } from "./api";
import { applyResult } from "./events";
import type {
  AppInfo,
  AppSettings,
  ChudEvent,
  DocPayload,
  EditOp,
  EditResult,
  EngineStatus,
  EventId,
  Listing,
  Micros,
  WindowInfo,
} from "./types";
import { setLanguage, t, type TKey } from "../i18n";

export type View = "library" | "studio" | "flow" | "observatory" | "settings" | "diagnostics" | "about";

export interface Toast {
  id: number;
  level: "info" | "success" | "warning" | "error";
  text: string;
  action?: { label: string; run: () => void };
}

export interface DocView {
  /** Time at the left edge (µs). */
  t0: Micros;
  /** Microseconds per pixel. */
  scale: number;
}

export interface DocState extends Omit<DocPayload, "events"> {
  id: string;
  events: ChudEvent[];
  selection: Set<EventId>;
  cursor: Micros;
  range: [Micros, Micros] | null;
  view: DocView | null;
  /** Bumped on every change of `events` (cheap memo key). */
  version: number;
}

export interface Clip {
  items: [Micros, Omit<ChudEvent, "id" | "t">][];
}

const idleStatus: EngineStatus = {
  mode: "idle",
  paused: false,
  countdown_ms: 0,
  elapsed_us: 0,
  event_count: 0,
  waiting_first_input: false,
  play_position_us: 0,
  play_duration_us: 0,
  iteration: 0,
  iterations: 1,
  macro_id: null,
  macro_name: "",
  block: "",
  live: false,
  armed: null,
};

interface State {
  info: AppInfo | null;
  settings: AppSettings | null;
  status: EngineStatus;
  foreground: WindowInfo | null;
  view: View;
  listing: Listing | null;
  activeDoc: string | null;
  docs: Record<string, DocState>;
  clipboard: Clip | null;
  toasts: Toast[];
  /** Library selection (macro ids). */
  libSelection: string[];
  libFolder: string;

  setView: (v: View) => void;
  toast: (level: Toast["level"], text: string, action?: Toast["action"]) => void;
  dismiss: (id: number) => void;
  fail: (e: unknown, prefix?: TKey) => void;

  saveSettings: (s: AppSettings) => Promise<void>;
  refreshLibrary: () => Promise<void>;

  openDoc: (id: string, goToStudio?: boolean) => Promise<void>;
  closeDoc: (id: string) => Promise<void>;
  patchDoc: (id: string, patch: Partial<DocState>) => void;
  edit: (id: string, op: EditOp, opts?: { select?: "created" | "keep" | "none" }) => Promise<EditResult | null>;
  undo: (id: string) => Promise<void>;
  redo: (id: string) => Promise<void>;
  applyPayload: (p: DocPayload) => void;
  setClipboard: (c: Clip | null) => void;
}

let toastSeq = 1;

export const useStore = create<State>((set, get) => ({
  info: null,
  settings: null,
  status: idleStatus,
  foreground: null,
  view: "library",
  listing: null,
  activeDoc: null,
  docs: {},
  clipboard: null,
  toasts: [],
  libSelection: [],
  libFolder: "",

  setView: (view) => set({ view }),

  toast: (level, text, action) => {
    const id = toastSeq++;
    set((s) => ({ toasts: [...s.toasts.slice(-4), { id, level, text, action }] }));
    const ms = level === "error" ? 9000 : action ? 7000 : 4200;
    window.setTimeout(() => get().dismiss(id), ms);
  },
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((x) => x.id !== id) })),
  fail: (e, prefix) => get().toast("error", prefix ? `${t(prefix)} — ${errorText(e)}` : errorText(e)),

  saveSettings: async (s) => {
    try {
      const saved = await api.setSettings(s);
      set({ settings: saved });
      setLanguage(saved.language);
      document.documentElement.style.setProperty("--glow", String(saved.glow));
      document.documentElement.dataset.reduceMotion = String(saved.reduce_motion);
    } catch (e) {
      get().fail(e, "error.settings");
    }
  },

  refreshLibrary: async () => {
    try {
      set({ listing: await api.list() });
    } catch (e) {
      get().fail(e, "error.library");
    }
  },

  openDoc: async (id, goToStudio = true) => {
    try {
      const existing = get().docs[id];
      if (!existing) {
        const p = await api.docOpen(id);
        get().applyPayload(p);
      }
      set({ activeDoc: id, ...(goToStudio ? { view: "studio" as View } : {}) });
    } catch (e) {
      get().fail(e, "error.open");
    }
  },

  closeDoc: async (id) => {
    const d = get().docs[id];
    try {
      await api.docClose(id, Boolean(d?.dirty));
    } catch (e) {
      get().fail(e, "error.save");
    }
    set((s) => {
      const docs = { ...s.docs };
      delete docs[id];
      const ids = Object.keys(docs);
      return { docs, activeDoc: s.activeDoc === id ? (ids[ids.length - 1] ?? null) : s.activeDoc };
    });
  },

  patchDoc: (id, patch) =>
    set((s) => (s.docs[id] ? { docs: { ...s.docs, [id]: { ...s.docs[id], ...patch } } } : {})),

  applyPayload: (p) =>
    set((s) => {
      const prev = s.docs[p.meta.id];
      const { events, ...rest } = p;
      const doc: DocState = {
        ...rest,
        id: p.meta.id,
        events,
        selection: new Set(),
        cursor: prev?.cursor ?? 0,
        range: null,
        view: prev?.view ?? null,
        version: (prev?.version ?? 0) + 1,
      };
      return { docs: { ...s.docs, [p.meta.id]: doc } };
    }),

  edit: async (id, op, opts) => {
    try {
      const r = await api.docEdit(id, op);
      set((s) => {
        const d = s.docs[id];
        if (!d) return {};
        const events = applyResult(d.events, r);
        let selection = d.selection;
        if (opts?.select === "created" && r.created.length) selection = new Set(r.created);
        else if (opts?.select === "none") selection = new Set();
        else {
          const alive = new Set(events.map((e) => e.id));
          selection = new Set([...d.selection].filter((x) => alive.has(x)));
        }
        return {
          docs: {
            ...s.docs,
            [id]: { ...d, events, selection, history: r.history, dirty: r.dirty, version: d.version + 1 },
          },
        };
      });
      return r;
    } catch (e) {
      get().fail(e, "error.edit");
      return null;
    }
  },

  undo: async (id) => {
    try {
      const r = await api.docUndo(id);
      if (r) applyEdit(set, id, r);
    } catch (e) {
      get().fail(e);
    }
  },
  redo: async (id) => {
    try {
      const r = await api.docRedo(id);
      if (r) applyEdit(set, id, r);
    } catch (e) {
      get().fail(e);
    }
  },

  setClipboard: (clipboard) => set({ clipboard }),
}));

function applyEdit(set: (f: (s: State) => Partial<State>) => void, id: string, r: EditResult) {
  set((s) => {
    const d = s.docs[id];
    if (!d) return {};
    const events = applyResult(d.events, r);
    const alive = new Set(events.map((e) => e.id));
    const selection = new Set([...d.selection].filter((x) => alive.has(x)));
    return { docs: { ...s.docs, [id]: { ...d, events, selection, history: r.history, dirty: r.dirty, version: d.version + 1 } } };
  });
}

export function useActiveDoc(): DocState | null {
  return useStore((s) => (s.activeDoc ? (s.docs[s.activeDoc] ?? null) : null));
}
