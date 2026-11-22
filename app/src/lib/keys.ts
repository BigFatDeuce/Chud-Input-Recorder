import type { Key, KeyRow, MouseButton } from "./types";

let table: KeyRow[] = [];
const byScan = new Map<number, KeyRow>();
const byVk = new Map<number, KeyRow>();
const byCode = new Map<string, KeyRow>();

export function setKeyTable(rows: KeyRow[]) {
  table = rows;
  byScan.clear();
  byVk.clear();
  byCode.clear();
  for (const r of rows) {
    if (r.scan && !byScan.has(r.scan)) byScan.set(r.scan, r);
    if (r.vk && !byVk.has(r.vk)) byVk.set(r.vk, r);
    byCode.set(r.code, r);
  }
}

export function keyTable(): KeyRow[] {
  return table;
}

export const EVDEV_PRIVATE = 0xf000;

/** Identity used to pair presses and releases (same as Rust `Key::ident`). */
export function keyIdent(k: Key): number {
  return k.scan !== 0 ? k.scan : 0x1_0000 | k.vk;
}

export function keyInfo(k: Key): KeyRow | undefined {
  return byScan.get(k.scan) ?? byVk.get(k.vk);
}

export function keyByCode(code: string): KeyRow | undefined {
  return byCode.get(code);
}

/** Human label; letters and digits follow the recorded logical key. */
export function keyLabel(k: Key): string {
  const vk = k.vk;
  if ((vk >= 0x30 && vk <= 0x39) || (vk >= 0x41 && vk <= 0x5a)) return String.fromCharCode(vk);
  const info = keyInfo(k);
  if (info) return info.label;
  if ((k.scan & 0xf000) === EVDEV_PRIVATE) return `Key ${k.scan & 0xfff}`;
  if (vk) return `VK ${vk.toString(16).toUpperCase().padStart(2, "0")}`;
  return `SC ${k.scan.toString(16).toUpperCase().padStart(4, "0")}`;
}

export function keyGroup(k: Key): string {
  return keyInfo(k)?.group ?? "system";
}

const MODIFIER_SCANS = new Set([0x1d, 0xe01d, 0x2a, 0x36, 0x38, 0xe038, 0xe05b, 0xe05c]);
export function isModifier(k: Key): boolean {
  return MODIFIER_SCANS.has(k.scan) || [0x10, 0x11, 0x12, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0x5b, 0x5c].includes(k.vk);
}

/** Colour of a key group (timeline lanes, visualizers). */
export const GROUP_COLORS: Record<string, string> = {
  letter: "#f9c83a",
  digit: "#ffe486",
  symbol: "#ffd36b",
  whitespace: "#c3d2e4",
  modifier: "#38d6ff",
  function: "#ffad5c",
  navigation: "#82b6ff",
  numpad: "#8fe8c2",
  media: "#ff86b0",
  system: "#a99bff",
  international: "#ff86b0",
};

export function keyColor(k: Key): string {
  return GROUP_COLORS[keyGroup(k)] ?? "#f9c83a";
}

export const BUTTON_LABELS: Record<MouseButton, string> = {
  left: "L",
  right: "R",
  middle: "M",
  x1: "X1",
  x2: "X2",
};

export const BUTTON_COLORS: Record<MouseButton, string> = {
  left: "#38d6ff",
  right: "#82b6ff",
  middle: "#8fe8c2",
  x1: "#a99bff",
  x2: "#ff86b0",
};

/** Converts a DOM keyboard event to a Chud key (for hotkey capture, pickers). */
export function keyFromDom(e: KeyboardEvent): Key | null {
  const info = byCode.get(e.code);
  if (info) return { scan: info.scan, vk: info.vk };
  return null;
}

// Same order as Rust `PadButton` (SDL gamepad convention).
export const PAD_BUTTONS = [
  "A / Cross",
  "B / Circle",
  "X / Square",
  "Y / Triangle",
  "Back",
  "Guide",
  "Start",
  "L3",
  "R3",
  "LB",
  "RB",
  "D-pad ↑",
  "D-pad ↓",
  "D-pad ←",
  "D-pad →",
  "Share",
  "Paddle R1",
  "Paddle L1",
  "Paddle R2",
  "Paddle L2",
  "Touchpad",
];

export const PAD_AXES = ["Left X", "Left Y", "Right X", "Right Y", "LT", "RT"];

export function padButtonLabel(b: number): string {
  return PAD_BUTTONS[b] ?? `Button ${b}`;
}

export function padAxisLabel(a: number): string {
  return PAD_AXES[a] ?? `Axis ${a}`;
}
