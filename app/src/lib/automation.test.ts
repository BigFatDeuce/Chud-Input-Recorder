import { describe, expect, it } from "vitest";
import { defaultWhen, newRule, parseTime, ruleProblem, timeText } from "./automation";

describe("automation rules", () => {
  it("creates rules that are off and stop on user input", () => {
    const r = newRule("m", true);
    expect(r.enabled).toBe(false);
    expect(r.stop_on_input).toBe(true);
    expect(r.use_flow).toBe(true);
    expect(newRule("m", false).id).not.toBe(r.id);
  });

  it("reports rules that can never start", () => {
    const r = newRule("m", false);
    expect(ruleProblem(r, false, true)).toBe("missing_macro");
    expect(ruleProblem(r, true, true)).toBe("no_window");
    const win = { ...r, when: { type: "window" as const, title: "", process: "notepad.exe", cooldown_s: 10 } };
    expect(ruleProblem(win, true, true)).toBeNull();
    expect(ruleProblem(win, true, false)).toBe("window_unavailable");
    const daily = { ...r, when: { type: "daily" as const, hour: 9, minute: 0, days: Array(7).fill(false) as boolean[] } };
    expect(ruleProblem(daily, true, true)).toBe("no_days");
    for (const type of ["startup", "daily", "interval"] as const) {
      expect(ruleProblem({ ...r, when: defaultWhen(type) }, true, false)).toBeNull();
    }
  });

  it("reads and writes times", () => {
    expect(timeText(7, 5)).toBe("07:05");
    expect(parseTime("07:05")).toEqual({ hour: 7, minute: 5 });
    expect(parseTime("")).toBeNull();
    expect(parseTime("24:00")).toBeNull();
    expect(parseTime("10:60")).toBeNull();
  });
});
