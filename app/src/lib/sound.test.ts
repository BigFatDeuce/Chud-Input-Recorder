import { describe, expect, it } from "vitest";
import { countdownCue } from "./sound";
import type { EngineStatus } from "./types";

const st = (mode: EngineStatus["mode"], countdown_ms = 0) => ({ mode, countdown_ms }) as EngineStatus;

describe("countdownCue", () => {
  it("ticks once per second, louder for the last three", () => {
    expect(countdownCue(st("idle"), st("countdown", 5000))).toBe("soft");
    expect(countdownCue(st("countdown", 5000), st("countdown", 4950))).toBeNull();
    expect(countdownCue(st("countdown", 4010), st("countdown", 3990))).toBe("soft");
    expect(countdownCue(st("countdown", 3010), st("countdown", 2990))).toBe("tick");
    expect(countdownCue(st("countdown", 1010), st("countdown", 990))).toBe("tick");
  });
  it("signals the start, and nothing when cancelled", () => {
    expect(countdownCue(st("countdown", 20), st("recording"))).toBe("go");
    expect(countdownCue(st("countdown", 1500), st("idle"))).toBeNull();
    expect(countdownCue(st("idle"), st("recording"))).toBeNull();
  });
});
