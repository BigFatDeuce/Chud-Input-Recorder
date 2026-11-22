// TypeScript mirror of the Rust data model (serde representations).

export type Micros = number;
export type EventId = number;

export interface Key {
  scan: number;
  vk: number;
}

export type MouseButton = "left" | "right" | "middle" | "x1" | "x2";
export const MOUSE_BUTTONS: MouseButton[] = ["left", "right", "middle", "x1", "x2"];

export type EventKind =
  | { type: "mouse_move"; x: number; y: number }
  | { type: "mouse_raw"; dx: number; dy: number }
  | { type: "mouse_down"; button: MouseButton; x: number; y: number }
  | { type: "mouse_up"; button: MouseButton; x: number; y: number }
  | { type: "wheel"; delta: number; horizontal: boolean; x: number; y: number }
  | { type: "key_down"; key: Key; repeat?: boolean }
  | { type: "key_up"; key: Key }
  | { type: "pad_button"; pad: number; button: number; down: boolean }
  | { type: "pad_axis"; pad: number; axis: number; value: number };

export type EventType = EventKind["type"];
export type ChudEvent = { id: EventId; t: Micros } & EventKind;

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Monitor {
  name: string;
  rect: Rect;
  scale: number;
  primary: boolean;
  refresh_hz: number | null;
}

export interface Environment {
  os: string;
  monitors: Monitor[];
  keyboard_layout: string;
  mouse_speed: number | null;
  mouse_acceleration: boolean | null;
  hostname_hash: string | null;
}

export interface MacroMeta {
  id: string;
  name: string;
  description: string;
  tags: string[];
  icon: string;
  color: string;
  favorite: boolean;
  created_ms: number;
  modified_ms: number;
  app_version: string;
  source_os: string;
}

export interface Marker {
  t: Micros;
  end: Micros | null;
  label: string;
  color: string;
}

export interface ContextSample {
  t: Micros;
  process: string;
  title: string;
  class: string;
  rect: Rect | null;
  redacted: boolean;
}

export type TimingMode = "exact" | "relative" | "adaptive" | "fast" | "custom";
export type MouseMode = "absolute" | "relative" | "off";
export type KeyMode = "physical" | "logical";
export type CoordMode = "as_recorded" | "adapt_to_screens";

export interface PlaybackSettings {
  speed: number;
  repeat: number;
  repeat_delay_ms: number;
  timing: TimingMode;
  adaptive_max_lag_ms: number;
  fast_gap_us: number;
  custom_min_gap_ms: number;
  custom_max_gap_ms: number;
  custom_fixed_gap_ms: number | null;
  mouse: MouseMode;
  keys: KeyMode;
  coords: CoordMode;
  gamepad: boolean;
  restore_cursor: boolean;
  abort_on_user_input: boolean;
}

export type Selection =
  | { kind: "all" }
  | { kind: "from"; t: Micros }
  | { kind: "range"; start: Micros; end: Micros }
  | { kind: "ids"; ids: EventId[] };

export type KeyFilter = { mode: "all" } | { mode: "only"; keys: number[] } | { mode: "except"; keys: number[] };

export interface RecordOptions {
  keyboard: boolean;
  mouse_buttons: boolean;
  mouse_move: boolean;
  mouse_raw: boolean;
  wheel: boolean;
  gamepad: boolean;
  countdown_ms: number;
  max_duration_ms: number | null;
  start_on_first_input: boolean;
  uniform_gap_ms: number | null;
  key_filter: KeyFilter;
  only_process: string | null;
  include_injected: boolean;
  excluded_processes: string[];
  mouse_max_hz: number | null;
  analog_threshold: number;
}

export type HotkeyAction =
  | "toggle_record"
  | "toggle_pause"
  | "toggle_play"
  | "emergency_stop"
  | "add_marker"
  | { trigger: number };

export interface Modifiers {
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
  meta: boolean;
}

export interface Hotkey {
  action: HotkeyAction;
  modifiers: Modifiers;
  key: Key;
  enabled: boolean;
}

export interface OverlaySettings {
  show_when_playing: boolean;
  position: "top" | "bottom";
}

export interface AppSettings {
  language: "auto" | "fr" | "en";
  hotkeys: Hotkey[];
  record: RecordOptions;
  playback: PlaybackSettings;
  overlay: OverlaySettings;
  countdown_sound: boolean;
  autosave: boolean;
  autosave_delay_ms: number;
  versions_kept: number;
  library_dir: string | null;
  reduce_motion: boolean;
  /** Interface zoom, 1 = 100 %. */
  ui_scale: number;
  glow: number;
  starfield: boolean;
  close_to_tray: boolean;
  confirm_delete: boolean;
  minimize_on_play: boolean;
  minimize_on_record: boolean;
  triggers: MacroTrigger[];
}

export interface MacroTrigger {
  slot: number;
  macro_id: string;
  use_flow: boolean;
}

export interface Capabilities {
  os: string;
  absolute_pointer_capture: boolean;
  raw_mouse_capture: boolean;
  hotkey_swallow: boolean;
  foreground_window: boolean;
  pixel_read: boolean;
  screen_capture: boolean;
  text_recognition: boolean;
  gamepad_output: boolean;
  session: string;
}

export interface AppInfo {
  version: string;
  os: string;
  arch: string;
  portable: boolean;
  data_dir: string;
  settings_file: string;
  library_dir: string;
  logs_dir: string;
  capabilities: Capabilities;
  monitors: Monitor[];
}

export interface Check {
  id: string;
  ok: boolean;
  title: string;
  detail: string;
  fix: string | null;
  code: string;
  vars: [string, string][];
}

export interface KeyRow {
  scan: number;
  evdev: number;
  vk: number;
  code: string;
  label: string;
  group: string;
}

export interface JitterStats {
  samples: number;
  mean_us: number;
  median_us: number;
  p95_us: number;
  p99_us: number;
  max_us: number;
  histogram: [number, number][];
}

export interface Summary {
  event_count: number;
  duration_us: Micros;
  keyboard: number;
  mouse_buttons: number;
  mouse_moves: number;
  wheel: number;
  gamepad: number;
  has_flow: boolean;
  density: number[];
}

export interface Manifest {
  format: string;
  version: number;
  meta: MacroMeta;
  settings: PlaybackSettings;
  env: Environment;
  markers: Marker[];
  summary: Summary;
  has_original: boolean;
}

export interface LibraryEntry {
  id: string;
  path: string;
  folder: string;
  manifest: Manifest;
  size: number;
}

export interface Listing {
  root: string;
  folders: string[];
  entries: LibraryEntry[];
  broken: [string, string][];
}

export interface TrashItem {
  name: string;
  is_dir: boolean;
  deleted_ms: number;
}

export interface Version {
  ms: number;
  size: number;
}

export interface HistoryInfo {
  can_undo: boolean;
  can_redo: boolean;
  undo_label: string | null;
  redo_label: string | null;
  revision: number;
  dirty: boolean;
}

export interface DocPayload {
  meta: MacroMeta;
  settings: PlaybackSettings;
  env: Environment;
  events: ChudEvent[];
  markers: Marker[];
  contexts: ContextSample[];
  flow: Flow | null;
  images: DocImage[];
  has_original: boolean;
  history: HistoryInfo;
  dirty: boolean;
  path: string;
}

export interface EditResult {
  removed: EventId[];
  added: ChudEvent[];
  history: HistoryInfo;
  dirty: boolean;
  created: EventId[];
  report: Record<string, number> | null;
}

export interface Saved {
  id: string;
  modified_ms: number;
  history: HistoryInfo;
}

export interface CleanupOptions {
  orphan_releases: boolean;
  close_stuck: boolean;
  strip_repeats: boolean;
  redundant_moves: boolean;
}

type ItemKind = [Micros, EventKind];

export type EditOp =
  | { op: "delete"; ids: EventId[]; partners?: boolean }
  | { op: "insert"; items: ItemKind[] }
  | { op: "update"; edits: { id: EventId; t: Micros; kind: EventKind }[] }
  | { op: "shift"; ids: EventId[]; dt: Micros }
  | { op: "translate"; ids: EventId[]; dx: number; dy: number }
  | { op: "scale"; ids: EventId[]; anchor: Micros; factor: number }
  | { op: "quantize"; ids: EventId[]; grid: Micros; strength: number }
  | { op: "set_gaps"; ids: EventId[]; gap: Micros; simultaneous: Micros }
  | { op: "set_hold_duration"; id: EventId; duration: Micros }
  | { op: "paste"; items: ItemKind[]; at: Micros }
  | { op: "duplicate"; ids: EventId[]; offset: Micros | null }
  | { op: "insert_time"; at: Micros; dt: Micros }
  | { op: "delete_range"; start: Micros; end: Micros; ripple: boolean }
  | { op: "remove_dead_time"; threshold: Micros; keep: Micros; respect_holds: boolean }
  | { op: "simplify_mouse"; ids: EventId[] | null; epsilon: number; max_interval: Micros }
  | { op: "merge_raw_motion"; ids: EventId[] | null; interval: Micros }
  | { op: "merge_wheel"; window: Micros }
  | { op: "cleanup"; options: CleanupOptions }
  | { op: "uniform_timing"; gap: Micros; simultaneous: Micros }
  | { op: "speed"; factor: number }
  | { op: "revert_original" };

export type Mode = "idle" | "countdown" | "recording" | "playing";

export interface EngineStatus {
  mode: Mode;
  paused: boolean;
  countdown_ms: number;
  elapsed_us: Micros;
  event_count: number;
  waiting_first_input: boolean;
  play_position_us: Micros;
  play_duration_us: Micros;
  iteration: number;
  iterations: number;
  macro_id: string | null;
  macro_name: string;
  block: string;
  live: boolean;
  armed: string | null;
}

export type LiveInput = { t: Micros; injected: boolean } & EventKind;

export interface WindowInfo {
  handle: number;
  pid: number;
  process: string;
  title: string;
  class: string;
  rect: Rect | null;
}

export type PlayEnd = { kind: "completed" } | { kind: "stopped" } | { kind: "failed"; detail: string };

export type EngineEventMsg =
  | { type: "hotkey"; action: HotkeyAction }
  | { type: "foreground"; window: WindowInfo }
  | { type: "playback_finished"; end: PlayEnd; macro_id: string | null }
  | { type: "notice"; level: string; message: string };

export interface Stats {
  events: number;
  duration_us: Micros;
  key_presses: number;
  clicks: number;
  wheel_notches: number;
  mouse_distance_px: number;
  pad_presses: number;
  apm: number;
  top_keys: [Key, number][];
  longest_pause_us: Micros;
  avg_hold_us: Micros;
  max_hold_us: Micros;
  mouse_rate_hz: number;
  clicks_by_button: [MouseButton, number][];
}

export interface Repetition {
  length: number;
  occurrences: [Micros, Micros][];
  first_ids: EventId[];
  summary: string;
}

export interface TimerReport {
  fine: JitterStats;
  coarse: JitterStats;
}

export interface ImportReport {
  imported: string[];
  failed: [string, string][];
}

// ── Flow ──

export type Motion = "instant" | "linear" | "smooth" | "human";

/** Screen pixels, virtual desktop coordinates. */
export interface Region {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** An image stored in a macro (searched on screen). */
export interface DocImage {
  id: string;
  w: number;
  h: number;
  /** PNG data URL. */
  url: string;
}

export type Target =
  | { kind: "image"; image: string; threshold: number; any_scale: boolean }
  | { kind: "color"; color: string; tolerance: number }
  | { kind: "text"; text: string; match_case: boolean };
export type TargetKind = Target["kind"];
export type FoundAction = "none" | "move" | "click" | "double_click" | "right_click" | "middle_click";
export type IfMissing = "continue" | "stop" | "fail";

export interface Found {
  x: number;
  y: number;
  w: number;
  h: number;
  score: number;
}

export interface ScreenTest {
  found: Found | null;
  best: Found | null;
  ms: number;
  preview: string | null;
  words: number | null;
}

export interface SnapshotInfo {
  x: number;
  y: number;
  w: number;
  h: number;
  monitors: Monitor[];
  cursor: [number, number] | null;
}

export type Condition =
  | { type: "expr"; expr: string }
  | { type: "window_active"; title: string; process: string }
  | { type: "window_exists"; title: string; process: string }
  | { type: "pixel_color"; x: number; y: number; color: string; tolerance: number }
  | { type: "key_held"; key: Key }
  | { type: "on_screen"; target: Target; region: Region | null }
  | { type: "chance"; percent: number }
  | { type: "not"; cond: Condition }
  | { type: "all"; conds: Condition[] }
  | { type: "any"; conds: Condition[] };

export type Block =
  | { type: "play_recording"; start: Micros | null; end: Micros | null; speed: number }
  | { type: "play_macro"; macro_id: string; name: string }
  | { type: "wait"; ms: string }
  | { type: "keys"; keys: Key[]; hold_ms: number }
  | { type: "key_down"; key: Key }
  | { type: "key_up"; key: Key }
  | { type: "type_text"; text: string; interval_ms: number }
  | { type: "move_mouse"; x: string; y: string; relative: boolean; duration_ms: number; motion: Motion }
  | { type: "click"; button: MouseButton; count: number; interval_ms: number; hold_ms: number }
  | { type: "mouse_down"; button: MouseButton }
  | { type: "mouse_up"; button: MouseButton }
  | { type: "scroll"; notches: number; horizontal: boolean }
  | { type: "repeat"; count: string | null; counter: string; body: FlowNode[] }
  | { type: "while"; cond: Condition; body: FlowNode[]; max_iterations: number | null }
  | { type: "if"; cond: Condition; then: FlowNode[]; otherwise: FlowNode[] }
  | { type: "wait_until"; cond: Condition; timeout_ms: number | null; poll_ms: number }
  | {
      type: "find";
      target: Target;
      region: Region | null;
      timeout_ms: number;
      poll_ms: number;
      action: FoundAction;
      offset_x: number;
      offset_y: number;
      duration_ms: number;
      motion: Motion;
      var: string;
      if_missing: IfMissing;
    }
  | { type: "set_var"; name: string; value: string }
  | { type: "notify"; message: string }
  | { type: "break" }
  | { type: "continue" }
  | { type: "stop" };

export type BlockType = Block["type"];
export type FlowNode = { id: string; enabled: boolean } & Block;

export interface Flow {
  blocks: FlowNode[];
}
