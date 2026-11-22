import { describe, expect, it } from "vitest";
import type { FlowNode } from "../lib/types";
import { cloneNode, countNodes, find, imageUses, insert, makeNode, makePreset, move, placeOf, remove, targetIssue, update, variables } from "./tree";

function sample(): FlowNode[] {
  const rep = makeNode("repeat");
  const wait = makeNode("wait");
  const iff = makeNode("if");
  const click = makeNode("click");
  const set = { ...makeNode("set_var"), name: "score" } as FlowNode;
  return [
    { ...rep, body: [wait, { ...iff, then: [click], otherwise: [] } as FlowNode] } as FlowNode,
    set,
  ];
}

describe("flow tree", () => {
  it("finds, places and counts nested nodes", () => {
    const f = sample();
    expect(countNodes(f)).toBe(5);
    const rep = f[0] as Extract<FlowNode, { type: "repeat" }>;
    const iff = rep.body[1] as Extract<FlowNode, { type: "if" }>;
    const click = iff.then[0];
    expect(find(f, click.id)?.type).toBe("click");
    expect(placeOf(f, click.id)).toEqual({ parent: iff.id, slot: "then", index: 0 });
  });

  it("updates and removes immutably", () => {
    const f = sample();
    const wait = (f[0] as Extract<FlowNode, { type: "repeat" }>).body[0];
    const g = update(f, wait.id, { ms: "250" } as Partial<FlowNode>);
    expect((find(g, wait.id) as Extract<FlowNode, { type: "wait" }>).ms).toBe("250");
    expect((find(f, wait.id) as Extract<FlowNode, { type: "wait" }>).ms).toBe("500");
    const h = remove(g, wait.id);
    expect(find(h, wait.id)).toBeNull();
    expect(countNodes(h)).toBe(4);
  });

  it("moves between lists and refuses moving into itself", () => {
    const f = sample();
    const rep = f[0] as Extract<FlowNode, { type: "repeat" }>;
    const iff = rep.body[1];
    const set = f[1];
    const g = move(f, set.id, { parent: iff.id, slot: "otherwise", index: 0 });
    expect(placeOf(g, set.id)).toEqual({ parent: iff.id, slot: "otherwise", index: 0 });
    expect(g.length).toBe(1);
    // Into its own descendant: unchanged.
    expect(move(f, rep.id, { parent: iff.id, slot: "then", index: 0 })).toBe(f);
    // Same list, forward.
    const h = insert(f, { parent: null, slot: "root", index: 2 }, [makeNode("stop")]);
    const moved = move(h, h[0].id, { parent: null, slot: "root", index: 3 });
    expect(moved.map((n) => n.type)).toEqual(["set_var", "stop", "repeat"]);
  });

  it("clones with fresh ids and lists variables", () => {
    const f = sample();
    const c = cloneNode(f[0]);
    expect(c.id).not.toBe(f[0].id);
    expect(countNodes([c])).toBe(4);
    expect(variables(f)).toEqual(["i", "score"]);
  });
});

describe("screen blocks", () => {
  it("presets build valid blocks and find sets its variables", () => {
    const click = makePreset("click_image", "img1");
    expect(click.type).toBe("find");
    const wait = makePreset("wait_image", "img1");
    const flow = [click, { ...makePreset("click_text"), var: "ok" } as typeof click, wait];
    expect(variables(flow)).toEqual(["found", "found_h", "found_score", "found_w", "found_x", "found_y", "ok", "ok_h", "ok_score", "ok_w", "ok_x", "ok_y"]);
    expect(imageUses(flow, "img1")).toBe(2);
    expect(imageUses(flow, "other")).toBe(0);
  });

  it("reports what a target misses", () => {
    const imgs = new Set(["a"]);
    expect(targetIssue({ kind: "image", image: "", threshold: 0.9, any_scale: false }, imgs)).toBe("need_image");
    expect(targetIssue({ kind: "image", image: "b", threshold: 0.9, any_scale: false }, imgs)).toBe("image_missing");
    expect(targetIssue({ kind: "image", image: "a", threshold: 0.9, any_scale: false }, imgs)).toBeNull();
    expect(targetIssue({ kind: "text", text: "  ", match_case: false }, imgs)).toBe("need_text");
    expect(targetIssue({ kind: "color", color: "#12abEF", tolerance: 0 }, imgs)).toBeNull();
    expect(targetIssue({ kind: "color", color: "red", tolerance: 0 }, imgs)).toBe("bad_color");
  });
});
