// Automatic triggers: state mirrored from the backend, and rule helpers.
import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { create } from "zustand";
import { api } from "./api";
import type { Automation, AutomationStatus, AutomationWhen } from "./types";

export const useAutomation = create<AutomationStatus>(() => ({ enabled: false, suspended: false, pending: null, last: {} }));

/** Keeps `useAutomation` up to date (once per window). */
export function useAutomationEvents() {
  useEffect(() => {
    // Subscribed first: a change between the two is not lost, and the
    // answer to the question never overwrites a newer event.
    let seen = false;
    const un = listen<AutomationStatus>("automation-status", (e) => {
      seen = true;
      useAutomation.setState(e.payload);
    });
    un.then(() => api.automationStatus())
      .then((s) => {
        if (!seen) useAutomation.setState(s);
      })
      .catch(() => {});
    return () => {
      un.then((f) => f());
    };
  }, []);
}

export type WhenType = AutomationWhen["type"];

export function defaultWhen(type: WhenType): AutomationWhen {
  switch (type) {
    case "startup":
      return { type, delay_s: 10 };
    case "window":
      return { type, title: "", process: "", cooldown_s: 10 };
    case "daily":
      return { type, hour: 9, minute: 0, days: [true, true, true, true, true, true, true] };
    case "interval":
      return { type, minutes: 30 };
  }
}

/** A new rule is off until the user has looked at it. */
export function newRule(macroId: string, useFlow: boolean): Automation {
  const id = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
  return { id, enabled: false, macro_id: macroId, use_flow: useFlow, stop_on_input: true, when: defaultWhen("window") };
}

export type RuleProblem = "missing_macro" | "no_window" | "no_days" | "window_unavailable";

/** Why a rule can never start, if that is the case. */
export function ruleProblem(r: Automation, macroExists: boolean, canSeeWindows: boolean): RuleProblem | null {
  if (!macroExists) return "missing_macro";
  if (r.when.type === "window") {
    if (!canSeeWindows) return "window_unavailable";
    if (!r.when.title.trim() && !r.when.process.trim()) return "no_window";
  }
  if (r.when.type === "daily" && !r.when.days.some(Boolean)) return "no_days";
  return null;
}

export function timeText(hour: number, minute: number): string {
  return `${String(hour).padStart(2, "0")}:${String(minute).padStart(2, "0")}`;
}

/** Reads `HH:MM` (the value of a time field); null while it is incomplete. */
export function parseTime(s: string): { hour: number; minute: number } | null {
  const m = /^(\d{1,2}):(\d{2})/.exec(s);
  if (!m) return null;
  const hour = Number(m[1]);
  const minute = Number(m[2]);
  return hour < 24 && minute < 60 ? { hour, minute } : null;
}
