// Lane layout of the timeline (pure, testable).
import type { Hold, Model } from "../lib/events";
import { lowerBound } from "../lib/format";
import type { ChudEvent, EventId, Micros } from "../lib/types";

export type LaneKind = "key" | "button" | "pad" | "wheel" | "pointer" | "raw" | "axis";
export type Section = "keyboard" | "mouse" | "gamepad";

export interface Lane {
  id: string;
  kind: LaneKind;
  /** Index in the model list for this kind (key rows, button rows…). */
  row: number;
  /** Top in content coordinates (below the ruler). */
  y: number;
  h: number;
  section: Section;
  first: boolean;
}

export interface LaneVisibility {
  keyboard: boolean;
  buttons: boolean;
  wheel: boolean;
  pointer: boolean;
  raw: boolean;
  gamepad: boolean;
}

export const DEFAULT_VISIBILITY: LaneVisibility = {
  keyboard: true,
  buttons: true,
  wheel: true,
  pointer: true,
  raw: true,
  gamepad: true,
};

export const SECTION_GAP = 10;

export function layoutLanes(m: Model, vis: LaneVisibility, compact = false): { lanes: Lane[]; height: number } {
  const lanes: Lane[] = [];
  let y = 4;
  const rowH = compact ? 16 : 20;
  let section: Section | null = null;
  const push = (kind: LaneKind, row: number, h: number, sec: Section) => {
    const first = section !== sec;
    if (first && section !== null) y += SECTION_GAP;
    section = sec;
    lanes.push({ id: `${kind}:${row}`, kind, row, y, h, section: sec, first });
    y += h;
  };
  if (vis.keyboard) m.keyRows.forEach((_, i) => push("key", i, rowH, "keyboard"));
  if (vis.buttons) m.buttonRows.forEach((_, i) => push("button", i, rowH, "mouse"));
  if (vis.wheel && m.wheel.length) push("wheel", 0, compact ? 26 : 32, "mouse");
  if (vis.pointer && m.moves.length) push("pointer", 0, compact ? 56 : 72, "mouse");
  if (vis.raw && m.raw.length) push("raw", 0, compact ? 30 : 38, "mouse");
  if (vis.gamepad) {
    m.padRows.forEach((_, i) => push("pad", i, rowH, "gamepad"));
    m.axisRows.forEach((_, i) => push("axis", i, compact ? 30 : 38, "gamepad"));
  }
  return { lanes, height: y + 8 };
}

/** Holds of a press/release lane. */
export function holdsOf(m: Model, lane: Lane): { holds: Hold[]; repeats: number[]; orphans: number[] } | null {
  switch (lane.kind) {
    case "key": {
      const r = m.keyRows[lane.row];
      return { holds: r.holds, repeats: r.repeats, orphans: r.orphans };
    }
    case "button": {
      const r = m.buttonRows[lane.row];
      return { holds: r.holds, repeats: [], orphans: r.orphans };
    }
    case "pad": {
      const r = m.padRows[lane.row];
      return { holds: r.holds, repeats: [], orphans: r.orphans };
    }
    default:
      return null;
  }
}

/** Event indices of a point lane (sorted by time). */
export function pointsOf(m: Model, lane: Lane): number[] | null {
  switch (lane.kind) {
    case "wheel":
      return m.wheel;
    case "pointer":
      return m.moves;
    case "raw":
      return m.raw;
    case "axis":
      return m.axisRows[lane.row].idx;
    default:
      return null;
  }
}

/** All event ids of a lane. */
export function laneIds(events: ChudEvent[], m: Model, lane: Lane): EventId[] {
  const h = holdsOf(m, lane);
  if (h) {
    const out: EventId[] = [];
    for (const x of h.holds) {
      out.push(events[x.down].id);
      if (x.up >= 0) out.push(events[x.up].id);
    }
    for (const i of h.repeats) out.push(events[i].id);
    for (const i of h.orphans) out.push(events[i].id);
    return out;
  }
  return (pointsOf(m, lane) ?? []).map((i) => events[i].id);
}

/** Ids of a lane whose time intersects [ta, tb]. */
export function laneIdsInRange(events: ChudEvent[], m: Model, lane: Lane, ta: Micros, tb: Micros): EventId[] {
  const out: EventId[] = [];
  const h = holdsOf(m, lane);
  if (h) {
    const first = lowerBound(h.holds.length, (i) => h.holds[i].end, ta);
    for (let i = first; i < h.holds.length; i++) {
      const x = h.holds[i];
      if (x.start > tb) break;
      out.push(events[x.down].id);
      if (x.up >= 0) out.push(events[x.up].id);
    }
    for (const i of [...h.repeats, ...h.orphans]) if (events[i].t >= ta && events[i].t <= tb) out.push(events[i].id);
    return out;
  }
  const pts = pointsOf(m, lane) ?? [];
  const a = lowerBound(pts.length, (i) => events[pts[i]].t, ta);
  for (let i = a; i < pts.length && events[pts[i]].t <= tb; i++) out.push(events[pts[i]].id);
  return out;
}

const NICE_STEPS = [
  10, 20, 50, 100, 200, 500, 1e3, 2e3, 5e3, 1e4, 2e4, 5e4, 1e5, 2e5, 5e5, 1e6, 2e6, 5e6, 1e7, 15e6, 3e7, 6e7, 12e7, 3e8, 6e8,
  9e8, 18e8, 36e8,
];

/** Major tick step (µs) so that labels are at least `minPx` apart. */
export function tickStep(scale: number, minPx = 90): Micros {
  for (const s of NICE_STEPS) if (s / scale >= minPx) return s;
  return NICE_STEPS[NICE_STEPS.length - 1];
}

/** Minor grid step used for drawing and snapping. */
export function minorStep(major: Micros): Micros {
  const lead = major / 10 ** Math.floor(Math.log10(major));
  return lead === 2 ? major / 4 : lead === 1.5 ? major / 3 : major / 5;
}
