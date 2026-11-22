import { describe, expect, it } from "vitest";
import { applyResult, buildModel, withPartners } from "./events";
import { fmtDuration, fmtTime } from "./format";
import { setKeyTable } from "./keys";
import type { ChudEvent } from "./types";

setKeyTable([
  { scan: 0x1d, evdev: 29, vk: 0xa2, code: "ControlLeft", label: "Ctrl", group: "modifier" },
  { scan: 0x1e, evdev: 30, vk: 0x41, code: "KeyA", label: "A", group: "letter" },
]);

const A = { scan: 0x1e, vk: 0x41 };
const CTRL = { scan: 0x1d, vk: 0xa2 };

/** Distributive omit, so each event variant keeps its own fields. */
type EventBody = ChudEvent extends infer E ? (E extends ChudEvent ? Omit<E, "id" | "t"> : never) : never;

function ev(id: number, t: number, kind: EventBody): ChudEvent {
  return { id, t, ...kind } as ChudEvent;
}

const sample: ChudEvent[] = [
  ev(0, 0, { type: "key_down", key: CTRL }),
  ev(1, 10, { type: "key_down", key: A }),
  ev(2, 20, { type: "key_down", key: A, repeat: true }),
  ev(3, 30, { type: "key_up", key: A }),
  ev(4, 40, { type: "key_up", key: CTRL }),
  ev(5, 50, { type: "mouse_move", x: 10, y: 20 }),
  ev(6, 60, { type: "mouse_down", button: "left", x: 15, y: 25 }),
  ev(7, 70, { type: "mouse_up", button: "left", x: 15, y: 25 }),
  ev(8, 80, { type: "key_up", key: { scan: 0x30, vk: 0x42 } }),
  ev(9, 90, { type: "mouse_raw", dx: 3, dy: -4 }),
];

describe("lane model", () => {
  it("pairs holds, repeats and orphans", () => {
    const m = buildModel(sample);
    expect(m.duration).toBe(90);
    expect(m.keyRows.map((r) => r.label)[0]).toBe("Ctrl"); // modifiers first
    const a = m.keyRows.find((r) => r.key.scan === 0x1e)!;
    expect(a.holds).toHaveLength(1);
    expect(a.holds[0]).toMatchObject({ start: 10, end: 30, down: 1, up: 3 });
    expect(a.repeats).toEqual([2]);
    const b = m.keyRows.find((r) => r.key.scan === 0x30)!;
    expect(b.orphans).toEqual([8]);
    expect(m.buttonRows[0].holds[0]).toMatchObject({ start: 60, end: 70 });
    expect(m.bounds).toEqual({ minX: 10, maxX: 15, minY: 20, maxY: 25 });
    expect(m.rawMax).toBe(7);
  });

  it("finds partners", () => {
    expect([...withPartners(sample, [1])].sort()).toEqual([1, 3]);
    expect([...withPartners(sample, [7])].sort()).toEqual([6, 7]);
  });
});

describe("applying backend changes", () => {
  it("removes and merges in order", () => {
    const r = applyResult(sample, {
      removed: [5, 9],
      added: [ev(10, 55, { type: "wheel", delta: 120, horizontal: false, x: 0, y: 0 }), ev(5, 5, { type: "mouse_move", x: 1, y: 1 })],
    });
    expect(r.map((e) => e.id)).toEqual([0, 5, 1, 2, 3, 4, 10, 6, 7, 8]);
  });
});

describe("formatting", () => {
  it("formats times", () => {
    expect(fmtTime(0)).toBe("0:00.000");
    expect(fmtTime(61_234_000)).toBe("1:01.234");
    expect(fmtTime(3_723_400_000, 1)).toBe("1:02:03.4");
    expect(fmtDuration(850_000)).toBe("850 ms");
    expect(fmtDuration(4_200_000)).toBe("4.20 s");
    expect(fmtDuration(192_000_000)).toBe("3 min 12 s");
  });
});
