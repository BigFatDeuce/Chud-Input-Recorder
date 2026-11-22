// Typed wrappers around the Rust commands.
import { invoke } from "@tauri-apps/api/core";
import type {
  AppInfo,
  AppSettings,
  Check,
  DocImage,
  DocPayload,
  EditOp,
  EditResult,
  EngineStatus,
  EventId,
  Flow,
  ImportReport,
  Key,
  KeyRow,
  Listing,
  MacroMeta,
  Marker,
  PlaybackSettings,
  RecordOptions,
  Region,
  Repetition,
  Saved,
  ScreenTest,
  Selection,
  SnapshotInfo,
  Stats,
  Target,
  TimerReport,
  TrashItem,
  Version,
} from "./types";

export interface MetaPatch {
  name?: string;
  description?: string;
  tags?: string[];
  icon?: string;
  color?: string;
  favorite?: boolean;
}

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  checks: () => invoke<Check[]>("checks"),
  getSettings: () => invoke<AppSettings>("get_settings"),
  setSettings: (settings: AppSettings) => invoke<AppSettings>("set_settings", { settings }),
  defaultSettings: () => invoke<AppSettings>("default_settings"),
  keyTable: () => invoke<KeyRow[]>("key_table"),
  diagTimer: () => invoke<TimerReport>("diag_timer"),
  engineStatus: () => invoke<EngineStatus>("engine_status"),
  quit: () => invoke<void>("quit_app"),

  list: () => invoke<Listing>("library_list"),
  createFolder: (parent: string, name: string) => invoke<string>("library_create_folder", { parent, name }),
  renameFolder: (folder: string, name: string) => invoke<string>("library_rename_folder", { folder, name }),
  moveFolder: (folder: string, into: string) => invoke<string>("library_move_folder", { folder, into }),
  trashFolder: (folder: string) => invoke<void>("library_trash_folder", { folder }),
  move: (ids: string[], folder: string) => invoke<void>("library_move", { ids, folder }),
  trash: (ids: string[]) => invoke<void>("library_trash", { ids }),
  trashList: () => invoke<TrashItem[]>("library_trash_list"),
  restore: (deletedMs: number, name: string) => invoke<void>("library_restore", { deletedMs, name }),
  emptyTrash: () => invoke<number>("library_empty_trash"),
  duplicate: (id: string, copySuffix: string) => invoke<string>("library_duplicate", { id, copySuffix }),
  importFiles: (paths: string[], folder: string) => invoke<ImportReport>("library_import", { paths, folder }),
  exportMacro: (id: string, dest: string) => invoke<void>("library_export", { id, dest }),
  setMeta: (id: string, patch: MetaPatch) => invoke<MacroMeta>("library_set_meta", { id, patch }),
  versions: (id: string) => invoke<Version[]>("library_versions", { id }),
  restoreVersion: (id: string, ms: number) => invoke<DocPayload | null>("library_restore_version", { id, ms }),
  absolutePath: (id: string | null) => invoke<string>("library_absolute_path", { id }),

  docNew: (name: string, folder: string) => invoke<string>("doc_new", { name, folder }),
  docOpen: (id: string) => invoke<DocPayload>("doc_open", { id }),
  docClose: (id: string, save: boolean) => invoke<void>("doc_close", { id, save }),
  docEdit: (id: string, op: EditOp) => invoke<EditResult>("doc_edit", { id, op }),
  docUndo: (id: string) => invoke<EditResult | null>("doc_undo", { id }),
  docRedo: (id: string) => invoke<EditResult | null>("doc_redo", { id }),
  docSave: (id: string) => invoke<Saved | null>("doc_save", { id }),
  docSetSettings: (id: string, settings: PlaybackSettings) => invoke<void>("doc_set_settings", { id, settings }),
  docSetMarkers: (id: string, markers: Marker[]) => invoke<void>("doc_set_markers", { id, markers }),
  docSetFlow: (id: string, flow: Flow | null) => invoke<void>("doc_set_flow", { id, flow }),
  docStats: (id: string) => invoke<Stats>("doc_stats", { id }),
  docRepetitions: (id: string, minLen: number, maxResults: number) =>
    invoke<Repetition[]>("doc_repetitions", { id, minLen, maxResults }),
  docExtract: (id: string, ids: EventId[], name: string) => invoke<string>("doc_extract", { id, ids, name }),
  exprCheck: (src: string) => invoke<string | null>("expr_check", { src }),
  openFolder: (which: "data" | "logs" | "library") => invoke<void>("open_folder", { which }),

  recordStart: (options?: RecordOptions) => invoke<void>("record_start", { options: options ?? null }),
  recordStop: () => invoke<void>("record_stop"),
  recordPause: () => invoke<void>("record_pause"),
  recordMarker: (label: string) => invoke<void>("record_marker", { label }),
  play: (id: string, opts: { settings?: PlaybackSettings; selection?: Selection; useFlow?: boolean } = {}) =>
    invoke<void>("play", {
      id,
      settings: opts.settings ?? null,
      selection: opts.selection ?? null,
      useFlow: opts.useFlow ?? false,
    }),
  playStop: () => invoke<void>("play_stop"),
  playPause: () => invoke<void>("play_pause"),
  emergencyStop: () => invoke<void>("emergency_stop"),
  arm: (id: string | null, useFlow: boolean) => invoke<void>("arm", { id, useFlow }),
  liveSet: (on: boolean) => invoke<void>("live_set", { on }),
  localKey: (key: Key, down: boolean) => invoke<void>("local_key", { key, down }),

  /** Captures every screen (Chud hides meanwhile) for the picker. */
  screenSnapshot: () => invoke<SnapshotInfo>("screen_snapshot"),
  /** RGBA pixels of part of the snapshot. */
  screenSnapshotPixels: async (region: Region): Promise<ImageData> => {
    const buf = await invoke<ArrayBuffer>("screen_snapshot_pixels", { region });
    const head = new DataView(buf, 0, 8);
    const w = head.getUint32(0, true);
    const h = head.getUint32(4, true);
    return new ImageData(new Uint8ClampedArray(buf, 8, w * h * 4), w, h);
  },
  screenSnapshotClear: () => invoke<void>("screen_snapshot_clear"),
  docAddImage: (id: string, region: Region) => invoke<DocImage>("doc_add_image", { id, region }),
  docImportImage: (id: string, path: string) => invoke<DocImage>("doc_import_image", { id, path }),
  docRemoveImage: (id: string, imageId: string) => invoke<void>("doc_remove_image", { id, imageId }),
  screenTest: (id: string, target: Target, region: Region | null) => invoke<ScreenTest>("screen_test", { id, target, region }),
};

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  return String(e);
}
