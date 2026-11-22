// Event helpers: ordering, applying backend changes, and the lane model the
// timeline draws.
import type { ChudEvent, EditResult, EventId, EventKind, Key, Micros, MouseButton } from "./types";
import { MOUSE_BUTTONS } from "./types";
import { isModifier, keyGroup, keyIdent, keyLabel, padAxisLabel, padButtonLabel } from "./keys";

export function order(a: ChudEvent, b: ChudEvent): number {
  return a.t - b.t || a.id - b.id;
}

/** Applies a backend change to a sorted event array (returns a new array). */
export function applyResult(events: ChudEvent[], r: Pick<EditResult, "removed" | "added">): ChudEvent[] {
  const removed = new Set(r.removed);
  const kept = removed.size ? events.filter((e) => !removed.has(e.id)) : events.slice();
  if (!r.added.length) return kept;
  const added = r.added.slice().sort(order);
  const out: ChudEvent[] = new Array(kept.length + added.length);
  let i = 0;
  let j = 0;
  let k = 0;
  while (i < kept.length && j < added.length) out[k++] = order(kept[i], added[j]) <= 0 ? kept[i++] : added[j++];
  while (i < kept.length) out[k++] = kept[i++];
  while (j < added.length) out[k++] = added[j++];
  return out;
}

export function kindOf(e: ChudEvent): EventKind {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  const { id, t, ...kind } = e;
  return kind as EventKind;
}

export type Category = "keyboard" | "mouse_button" | "mouse_move" | "mouse_raw" | "wheel" | "pad_button" | "pad_axis";

export function category(e: EventKind): Category {
  switch (e.type) {
    case "key_down":
    case "key_up":
      return "keyboard";
    case "mouse_down":
    case "mouse_up":
      return "mouse_button";
    case "mouse_move":
      return "mouse_move";
    case "mouse_raw":
      return "mouse_raw";
    case "wheel":
      return "wheel";
    case "pad_button":
      return "pad_button";
    case "pad_axis":
      return "pad_axis";
  }
}

/** Pairing identity of pressable controls (press/release). */
export function controlOf(e: EventKind): { id: string; down: boolean } | null {
  switch (e.type) {
    case "key_down":
      return { id: `k${keyIdent(e.key)}`, down: true };
    case "key_up":
      return { id: `k${keyIdent(e.key)}`, down: false };
    case "mouse_down":
      return { id: `m${e.button}`, down: true };
    case "mouse_up":
      return { id: `m${e.button}`, down: false };
    case "pad_button":
      return { id: `p${e.pad}:${e.button}`, down: e.down };
    default:
      return null;
  }
}

export function describe(e: EventKind): string {
  switch (e.type) {
    case "key_down":
      return `${keyLabel(e.key)} ↓${e.repeat ? " ⟳" : ""}`;
    case "key_up":
      return `${keyLabel(e.key)} ↑`;
    case "mouse_down":
      return `${e.button.toUpperCase()} ↓ (${e.x}, ${e.y})`;
    case "mouse_up":
      return `${e.button.toUpperCase()} ↑ (${e.x}, ${e.y})`;
    case "mouse_move":
      return `→ (${e.x}, ${e.y})`;
    case "mouse_raw":
      return `Δ (${e.dx}, ${e.dy})`;
    case "wheel":
      return `${e.horizontal ? "⇆" : "⇅"} ${e.delta > 0 ? "+" : ""}${e.delta / 120}`;
    case "pad_button":
      return `🎮${e.pad + 1} · ${padButtonLabel(e.button)} ${e.down ? "↓" : "↑"}`;
    case "pad_axis":
      return `🎮${e.pad + 1} · ${padAxisLabel(e.axis)} ${e.value}`;
  }
}

// ───────────────────────────── lane model ─────────────────────────────

export interface Hold {
  /** Index of the press in `events`. */
  down: number;
  /** Index of the release, -1 when never released. */
  up: number;
  start: Micros;
  end: Micros;
}

export interface KeyRow {
  ident: number;
  key: Key;
  label: string;
  group: string;
  modifier: boolean;
  holds: Hold[];
  /** Auto-repeat presses (indices). */
  repeats: number[];
  /** Orphan releases (indices). */
  orphans: number[];
}

export interface ButtonRow {
  button: MouseButton;
  holds: Hold[];
  orphans: number[];
}

export interface PadRow {
  pad: number;
  button: number;
  holds: Hold[];
  orphans: number[];
}

export interface AxisRow {
  pad: number;
  axis: number;
  idx: number[];
}

export interface Model {
  duration: Micros;
  count: number;
  keyRows: KeyRow[];
  buttonRows: ButtonRow[];
  padRows: PadRow[];
  axisRows: AxisRow[];
  /** Indices of pointer moves, raw motion and wheel events. */
  moves: number[];
  raw: number[];
  wheel: number[];
  /** Pointer bounds over moves and clicks. */
  bounds: { minX: number; maxX: number; minY: number; maxY: number } | null;
  /** Largest |dx|+|dy| of raw motion (for scaling). */
  rawMax: number;
  /** Index → row id for hit testing and selection by lane. */
  rowOf: Int32Array;
}

/** Builds lanes from a sorted event array. O(n). */
export function buildModel(events: ChudEvent[]): Model {
  const keyRows = new Map<number, KeyRow>();
  const buttonRows = new Map<MouseButton, ButtonRow>();
  const padRows = new Map<string, PadRow>();
  const axisRows = new Map<string, AxisRow>();
  const open = new Map<string, { row: Hold[]; idx: number }>();
  const moves: number[] = [];
  const raw: number[] = [];
  const wheel: number[] = [];
  let minX = Infinity;
  let maxX = -Infinity;
  let minY = Infinity;
  let maxY = -Infinity;
  let rawMax = 1;
  const rowOf = new Int32Array(events.length).fill(-1);
  const end = events.length ? events[events.length - 1].t : 0;

  const pos = (x: number, y: number) => {
    if (x < minX) minX = x;
    if (x > maxX) maxX = x;
    if (y < minY) minY = y;
    if (y > maxY) maxY = y;
  };

  for (let i = 0; i < events.length; i++) {
    const e = events[i];
    switch (e.type) {
      case "key_down":
      case "key_up": {
        const ident = keyIdent(e.key);
        let row = keyRows.get(ident);
        if (!row) {
          row = {
            ident,
            key: e.key,
            label: keyLabel(e.key),
            group: keyGroup(e.key),
            modifier: isModifier(e.key),
            holds: [],
            repeats: [],
            orphans: [],
          };
          keyRows.set(ident, row);
        }
        const cid = `k${ident}`;
        if (e.type === "key_down") {
          if (e.repeat && open.has(cid)) {
            row.repeats.push(i);
          } else if (!open.has(cid)) {
            const h: Hold = { down: i, up: -1, start: e.t, end };
            row.holds.push(h);
            open.set(cid, { row: row.holds, idx: row.holds.length - 1 });
          } else {
            row.repeats.push(i);
          }
        } else {
          const o = open.get(cid);
          if (o) {
            const h = o.row[o.idx];
            h.up = i;
            h.end = e.t;
            open.delete(cid);
          } else row.orphans.push(i);
        }
        break;
      }
      case "mouse_down":
      case "mouse_up": {
        pos(e.x, e.y);
        let row = buttonRows.get(e.button);
        if (!row) {
          row = { button: e.button, holds: [], orphans: [] };
          buttonRows.set(e.button, row);
        }
        const cid = `m${e.button}`;
        if (e.type === "mouse_down") {
          if (!open.has(cid)) {
            row.holds.push({ down: i, up: -1, start: e.t, end });
            open.set(cid, { row: row.holds, idx: row.holds.length - 1 });
          }
        } else {
          const o = open.get(cid);
          if (o) {
            o.row[o.idx].up = i;
            o.row[o.idx].end = e.t;
            open.delete(cid);
          } else row.orphans.push(i);
        }
        break;
      }
      case "pad_button": {
        const k = `${e.pad}:${e.button}`;
        let row = padRows.get(k);
        if (!row) {
          row = { pad: e.pad, button: e.button, holds: [], orphans: [] };
          padRows.set(k, row);
        }
        const cid = `p${k}`;
        if (e.down) {
          if (!open.has(cid)) {
            row.holds.push({ down: i, up: -1, start: e.t, end });
            open.set(cid, { row: row.holds, idx: row.holds.length - 1 });
          }
        } else {
          const o = open.get(cid);
          if (o) {
            o.row[o.idx].up = i;
            o.row[o.idx].end = e.t;
            open.delete(cid);
          } else row.orphans.push(i);
        }
        break;
      }
      case "pad_axis": {
        const k = `${e.pad}:${e.axis}`;
        let row = axisRows.get(k);
        if (!row) {
          row = { pad: e.pad, axis: e.axis, idx: [] };
          axisRows.set(k, row);
        }
        row.idx.push(i);
        break;
      }
      case "mouse_move":
        pos(e.x, e.y);
        moves.push(i);
        break;
      case "mouse_raw": {
        raw.push(i);
        const m = Math.abs(e.dx) + Math.abs(e.dy);
        if (m > rawMax) rawMax = m;
        break;
      }
      case "wheel":
        wheel.push(i);
        break;
    }
  }

  // Keyboard rows: modifiers first, then by first use.
  const keys = [...keyRows.values()].sort((a, b) => {
    if (a.modifier !== b.modifier) return a.modifier ? -1 : 1;
    const fa = a.holds[0]?.start ?? Infinity;
    const fb = b.holds[0]?.start ?? Infinity;
    return fa - fb;
  });
  const buttons = MOUSE_BUTTONS.filter((b) => buttonRows.has(b)).map((b) => buttonRows.get(b)!);
  const pads = [...padRows.values()].sort((a, b) => a.pad - b.pad || a.button - b.button);
  const axes = [...axisRows.values()].sort((a, b) => a.pad - b.pad || a.axis - b.axis);

  keys.forEach((r, ri) => {
    for (const h of r.holds) {
      rowOf[h.down] = ri;
      if (h.up >= 0) rowOf[h.up] = ri;
    }
    for (const i of r.repeats) rowOf[i] = ri;
    for (const i of r.orphans) rowOf[i] = ri;
  });

  return {
    duration: end,
    count: events.length,
    keyRows: keys,
    buttonRows: buttons,
    padRows: pads,
    axisRows: axes,
    moves,
    raw,
    wheel,
    bounds: Number.isFinite(minX) ? { minX, maxX, minY, maxY } : null,
    rawMax,
    rowOf,
  };
}

/** Ids of the partner (press ↔ release) of each given event, plus the events. */
export function withPartners(events: ChudEvent[], ids: Iterable<EventId>): Set<EventId> {
  const want = new Set(ids);
  const out = new Set(want);
  const open = new Map<string, ChudEvent>();
  for (const e of events) {
    const c = controlOf(e);
    if (!c) continue;
    if (c.down) {
      if (!open.has(c.id)) open.set(c.id, e);
    } else {
      const d = open.get(c.id);
      if (d) {
        if (want.has(d.id)) out.add(e.id);
        if (want.has(e.id)) out.add(d.id);
        open.delete(c.id);
      }
    }
  }
  return out;
}
