import { describe, expect, it } from "vitest";
import { stepScale, UI_SCALES } from "./zoom";

describe("interface size steps", () => {
  it("moves one step at a time and stops at the ends", () => {
    expect(stepScale(1, 1)).toBe(1.1);
    expect(stepScale(1, -1)).toBe(0.9);
    expect(stepScale(1.5, 1)).toBe(1.5);
    expect(stepScale(0.8, -1)).toBe(0.8);
  });
  it("snaps a value between steps to the neighbouring step", () => {
    expect(stepScale(1.17, 1)).toBe(1.25);
    expect(stepScale(1.17, -1)).toBe(1.1);
  });
  it("offers 100 %", () => {
    expect(UI_SCALES).toContain(1);
  });
});
