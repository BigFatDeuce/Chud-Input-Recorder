import { useSyncExternalStore } from "react";
import { en, type Dict } from "./en";
import { fr } from "./fr";

export type Lang = "fr" | "en";
export type TKey = keyof Dict;

const dicts: Record<Lang, Dict> = { en, fr };

function detect(): Lang {
  const l = (navigator.languages?.[0] ?? navigator.language ?? "en").toLowerCase();
  return l.startsWith("fr") ? "fr" : "en";
}

let current: Lang = detect();
const listeners = new Set<() => void>();

export function setLanguage(pref: "auto" | Lang) {
  const next = pref === "auto" ? detect() : pref;
  if (next !== current) {
    current = next;
    document.documentElement.lang = next;
    listeners.forEach((l) => l());
  }
}

export function language(): Lang {
  return current;
}

/** Translates `key`, replacing `{name}` placeholders. */
export function t(key: TKey, vars?: Record<string, string | number>): string {
  let s: string = dicts[current][key] ?? en[key] ?? key;
  if (vars) {
    for (const [k, v] of Object.entries(vars)) s = s.split(`{${k}}`).join(String(v));
  }
  return s;
}

/** Translates a dynamic key when it exists, else returns `fallback`. */
export function tOr(key: string, fallback: string, vars?: Record<string, string | number>): string {
  if (!(key in en)) return fallback;
  return t(key as TKey, vars);
}

/** Plural helper: picks `key.one` / `key.other` style pairs. */
export function tn(one: TKey, other: TKey, n: number, vars?: Record<string, string | number>): string {
  return t(n === 1 ? one : other, { n: fmtInt(n), ...vars });
}

export function fmtInt(n: number): string {
  return new Intl.NumberFormat(current).format(n);
}

export function fmtNum(n: number, digits = 1): string {
  return new Intl.NumberFormat(current, { maximumFractionDigits: digits, minimumFractionDigits: 0 }).format(n);
}

export function fmtDate(ms: number, withTime = true): string {
  const d = new Date(ms);
  return new Intl.DateTimeFormat(current, {
    dateStyle: "medium",
    ...(withTime ? { timeStyle: "short" } : {}),
  }).format(d);
}

export function fmtRelative(ms: number): string {
  const diff = (ms - Date.now()) / 1000;
  const rtf = new Intl.RelativeTimeFormat(current, { numeric: "auto" });
  const abs = Math.abs(diff);
  if (abs < 60) return rtf.format(Math.round(diff), "second");
  if (abs < 3600) return rtf.format(Math.round(diff / 60), "minute");
  if (abs < 86400) return rtf.format(Math.round(diff / 3600), "hour");
  if (abs < 86400 * 30) return rtf.format(Math.round(diff / 86400), "day");
  return fmtDate(ms, false);
}

/** Re-renders the component when the language changes. */
export function useLang(): Lang {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => current,
  );
}
