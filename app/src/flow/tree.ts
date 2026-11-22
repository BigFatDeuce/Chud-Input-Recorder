// Pure helpers on the flow tree (immutable updates).
import type { Block, BlockType, Condition, FlowNode, Key, Target, TargetKind } from "../lib/types";

export type Slot = "root" | "body" | "then" | "otherwise";

export interface Place {
  /** Container block id, null for the root list. */
  parent: string | null;
  slot: Slot;
  index: number;
}

export function newId(): string {
  const a = new Uint8Array(4);
  crypto.getRandomValues(a);
  return [...a].map((b) => b.toString(16).padStart(2, "0")).join("");
}

const KEY_A: Key = { scan: 0x1e, vk: 0x41 };

export function defaultTarget(kind: TargetKind, image = ""): Target {
  switch (kind) {
    case "image":
      return { kind, image, threshold: 0.9, any_scale: false };
    case "text":
      return { kind, text: "", match_case: false };
    case "color":
      return { kind, color: "#ff0000", tolerance: 12 };
  }
}

/** Ready-made screen blocks of the palette. */
export type Preset = "click_image" | "click_text" | "find_color" | "wait_image" | "if_image";
export const PRESETS: Preset[] = ["click_image", "click_text", "wait_image", "if_image", "find_color"];

export function makePreset(p: Preset, image = ""): FlowNode {
  const id = newId();
  switch (p) {
    case "click_image":
      return { id, enabled: true, ...defaultBlock("find"), target: defaultTarget("image", image) } as FlowNode;
    case "click_text":
      return { id, enabled: true, ...defaultBlock("find"), target: defaultTarget("text") } as FlowNode;
    case "find_color":
      return { id, enabled: true, ...defaultBlock("find"), target: defaultTarget("color"), action: "move", if_missing: "continue", timeout_ms: 0 } as FlowNode;
    case "wait_image":
      return { id, enabled: true, type: "wait_until", cond: { type: "on_screen", target: defaultTarget("image", image), region: null }, timeout_ms: 30_000, poll_ms: 250 };
    case "if_image":
      return { id, enabled: true, type: "if", cond: { type: "on_screen", target: defaultTarget("image", image), region: null }, then: [], otherwise: [] };
  }
}

export function defaultBlock(type: BlockType): Block {
  switch (type) {
    case "play_recording":
      return { type, start: null, end: null, speed: 1 };
    case "play_macro":
      return { type, macro_id: "", name: "" };
    case "wait":
      return { type, ms: "500" };
    case "keys":
      return { type, keys: [], hold_ms: 40 };
    case "key_down":
    case "key_up":
      return { type, key: KEY_A };
    case "type_text":
      return { type, text: "", interval_ms: 25 };
    case "move_mouse":
      return { type, x: "0", y: "0", relative: false, duration_ms: 250, motion: "smooth" };
    case "click":
      return { type, button: "left", count: 1, interval_ms: 80, hold_ms: 40 };
    case "mouse_down":
    case "mouse_up":
      return { type, button: "left" };
    case "scroll":
      return { type, notches: -3, horizontal: false };
    case "repeat":
      return { type, count: "3", counter: "i", body: [] };
    case "while":
      return { type, cond: { type: "expr", expr: "i < 10" }, body: [], max_iterations: 1000 };
    case "if":
      return { type, cond: { type: "chance", percent: 50 }, then: [], otherwise: [] };
    case "wait_until":
      return { type, cond: { type: "window_active", title: "", process: "" }, timeout_ms: 10_000, poll_ms: 100 };
    case "find":
      return {
        type,
        target: defaultTarget("image"),
        region: null,
        timeout_ms: 5_000,
        poll_ms: 250,
        action: "click",
        offset_x: 0,
        offset_y: 0,
        duration_ms: 250,
        motion: "instant",
        var: "found",
        if_missing: "fail",
      };
    case "set_var":
      return { type, name: "x", value: "0" };
    case "notify":
      return { type, message: "" };
    case "break":
    case "continue":
    case "stop":
      return { type };
  }
}

export function makeNode(type: BlockType): FlowNode {
  return { id: newId(), enabled: true, ...defaultBlock(type) } as FlowNode;
}

export function slotsOf(n: FlowNode): Slot[] {
  if (n.type === "repeat" || n.type === "while") return ["body"];
  if (n.type === "if") return ["then", "otherwise"];
  return [];
}

export function childList(n: FlowNode, slot: Slot): FlowNode[] {
  if (slot === "body" && (n.type === "repeat" || n.type === "while")) return n.body;
  if (slot === "then" && n.type === "if") return n.then;
  if (slot === "otherwise" && n.type === "if") return n.otherwise;
  return [];
}

function withChild(n: FlowNode, slot: Slot, list: FlowNode[]): FlowNode {
  if (slot === "body" && (n.type === "repeat" || n.type === "while")) return { ...n, body: list };
  if (slot === "then" && n.type === "if") return { ...n, then: list };
  if (slot === "otherwise" && n.type === "if") return { ...n, otherwise: list };
  return n;
}

/** Maps every list of the tree (bottom-up). */
function mapLists(list: FlowNode[], f: (list: FlowNode[], parent: string | null, slot: Slot) => FlowNode[], parent: string | null = null, slot: Slot = "root"): FlowNode[] {
  const inner = list.map((n) => {
    let m = n;
    for (const s of slotsOf(n)) m = withChild(m, s, mapLists(childList(m, s), f, n.id, s));
    return m;
  });
  return f(inner, parent, slot);
}

export function find(list: FlowNode[], id: string): FlowNode | null {
  for (const n of list) {
    if (n.id === id) return n;
    for (const s of slotsOf(n)) {
      const r = find(childList(n, s), id);
      if (r) return r;
    }
  }
  return null;
}

export function placeOf(list: FlowNode[], id: string, parent: string | null = null, slot: Slot = "root"): Place | null {
  for (let i = 0; i < list.length; i++) {
    const n = list[i];
    if (n.id === id) return { parent, slot, index: i };
    for (const s of slotsOf(n)) {
      const r = placeOf(childList(n, s), id, n.id, s);
      if (r) return r;
    }
  }
  return null;
}

export function update(list: FlowNode[], id: string, patch: Partial<FlowNode>): FlowNode[] {
  return mapLists(list, (l) => l.map((n) => (n.id === id ? ({ ...n, ...patch } as FlowNode) : n)));
}

export function remove(list: FlowNode[], id: string): FlowNode[] {
  return mapLists(list, (l) => l.filter((n) => n.id !== id));
}

export function insert(list: FlowNode[], place: Place, nodes: FlowNode[]): FlowNode[] {
  if (place.parent === null) {
    const out = list.slice();
    out.splice(Math.min(place.index, out.length), 0, ...nodes);
    return out;
  }
  return mapLists(list, (l) =>
    l.map((n) => {
      if (n.id !== place.parent) return n;
      const c = childList(n, place.slot).slice();
      c.splice(Math.min(place.index, c.length), 0, ...nodes);
      return withChild(n, place.slot, c);
    }),
  );
}

/** True when `id` is `root` or inside it. */
export function contains(root: FlowNode, id: string): boolean {
  if (root.id === id) return true;
  return slotsOf(root).some((s) => childList(root, s).some((c) => contains(c, id)));
}

/** Moves a node to a place (no-op when moving into itself). */
export function move(list: FlowNode[], id: string, to: Place): FlowNode[] {
  const node = find(list, id);
  if (!node) return list;
  if (to.parent && contains(node, to.parent)) return list;
  const from = placeOf(list, id)!;
  let index = to.index;
  if (from.parent === to.parent && from.slot === to.slot && from.index < to.index) index -= 1;
  return insert(remove(list, id), { ...to, index }, [node]);
}

/** Deep copy with fresh ids. */
export function cloneNode(n: FlowNode): FlowNode {
  let c = { ...n, id: newId() } as FlowNode;
  for (const s of slotsOf(n)) c = withChild(c, s, childList(n, s).map(cloneNode));
  return c;
}

/** Variables assigned anywhere in the flow. */
export function variables(list: FlowNode[]): string[] {
  const out = new Set<string>();
  const walk = (l: FlowNode[]) => {
    for (const n of l) {
      if (n.type === "set_var" && n.name) out.add(n.name);
      if (n.type === "repeat" && n.counter) out.add(n.counter);
      if (n.type === "find") {
        const v = n.var.trim() || "found";
        for (const s of ["", "_x", "_y", "_w", "_h", "_score"]) out.add(v + s);
      }
      for (const s of slotsOf(n)) walk(childList(n, s));
    }
  };
  walk(list);
  return [...out].sort();
}

/** Screen targets of a condition (nested ones included). */
export function condTargets(c: Condition): Target[] {
  switch (c.type) {
    case "on_screen":
      return [c.target];
    case "not":
      return condTargets(c.cond);
    case "all":
    case "any":
      return c.conds.flatMap(condTargets);
    default:
      return [];
  }
}

/** Screen targets used by a block itself (not its children). */
export function targetsOf(n: FlowNode): Target[] {
  if (n.type === "find") return [n.target];
  if (n.type === "while" || n.type === "if" || n.type === "wait_until") return condTargets(n.cond);
  return [];
}

/** How many targets of the flow use an image. */
export function imageUses(list: FlowNode[], image: string): number {
  let n = 0;
  for (const x of list) {
    n += targetsOf(x).filter((t) => t.kind === "image" && t.image === image).length;
    for (const s of slotsOf(x)) n += imageUses(childList(x, s), image);
  }
  return n;
}

export type TargetIssue = "need_image" | "image_missing" | "need_text" | "bad_color";

/** What prevents a target from being searched, if anything. */
export function targetIssue(t: Target, images: ReadonlySet<string>): TargetIssue | null {
  switch (t.kind) {
    case "image":
      return !t.image ? "need_image" : images.has(t.image) ? null : "image_missing";
    case "text":
      return t.text.trim() ? null : "need_text";
    case "color":
      return /^#?[0-9a-f]{6}$/i.test(t.color.trim()) ? null : "bad_color";
  }
}

export function countNodes(list: FlowNode[]): number {
  let n = 0;
  for (const x of list) {
    n += 1;
    for (const s of slotsOf(x)) n += countNodes(childList(x, s));
  }
  return n;
}

/** Expression fields of a block (validated by the core parser). */
export function exprFields(n: FlowNode): string[] {
  switch (n.type) {
    case "wait":
      return [n.ms];
    case "move_mouse":
      return [n.x, n.y];
    case "repeat":
      return n.count === null ? [] : [n.count];
    case "set_var":
      return [n.value];
    case "while":
    case "if":
    case "wait_until":
      return condExprs(n.cond);
    default:
      return [];
  }
}

function condExprs(c: import("../lib/types").Condition): string[] {
  switch (c.type) {
    case "expr":
      return [c.expr];
    case "not":
      return condExprs(c.cond);
    case "all":
    case "any":
      return c.conds.flatMap(condExprs);
    default:
      return [];
  }
}
