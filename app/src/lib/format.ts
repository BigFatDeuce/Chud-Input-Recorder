import type { Micros } from "./types";

/** `1:02.345` (m:ss.mmm) or `1:02:03.4` for long durations. */
export function fmtTime(us: Micros, precision: 0 | 1 | 2 | 3 = 3): string {
  const neg = us < 0;
  let v = Math.abs(us) / 1e6;
  const h = Math.floor(v / 3600);
  v -= h * 3600;
  const m = Math.floor(v / 60);
  v -= m * 60;
  const pad = precision === 0 ? 2 : precision + 3;
  let s = v.toFixed(precision).padStart(pad, "0");
  // toFixed may round up to 60.000
  if (s.startsWith("60")) s = (0).toFixed(precision).padStart(pad, "0");
  const core = h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
  return neg ? `-${core}` : core;
}

/** Short human duration: `850 ms`, `4.2 s`, `3 min 12 s`, `1 h 04 min`. */
export function fmtDuration(us: Micros): string {
  if (us === 0) return "0 s";
  const ms = us / 1000;
  if (Math.abs(ms) < 1) return `${Math.round(us)} µs`;
  if (Math.abs(ms) < 1000) return `${ms < 10 ? ms.toFixed(1) : Math.round(ms)} ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(s < 10 ? 2 : 1)} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min ${String(Math.floor(s % 60)).padStart(2, "0")} s`;
  const h = Math.floor(m / 60);
  return `${h} h ${String(m % 60).padStart(2, "0")} min`;
}

/** Milliseconds with up to 3 decimals (µs precision) for inputs. */
export function usToMs(us: Micros): number {
  return Math.round(us) / 1000;
}

export function msToUs(ms: number): Micros {
  return Math.round(ms * 1000);
}

export function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

export function clamp(v: number, lo: number, hi: number): number {
  return v < lo ? lo : v > hi ? hi : v;
}

/** First index whose value is >= x in a sorted numeric accessor. */
export function lowerBound(n: number, at: (i: number) => number, x: number): number {
  let lo = 0;
  let hi = n;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (at(mid) < x) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}
