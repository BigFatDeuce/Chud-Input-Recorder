// Canvas timeline: ruler, markers, key roll, mouse lanes, pointer curves,
// gamepad lanes. Handles zoom/pan, selection, drag-move with snapping,
// hold resizing, range selection and markers.
import { forwardRef, useCallback, useEffect, useImperativeHandle, useLayoutEffect, useRef, useState } from "react";
import { t as tr } from "../i18n";
import { describe, withPartners, type Hold, type Model } from "../lib/events";
import { clamp, fmtDuration, fmtTime, lowerBound } from "../lib/format";
import { BUTTON_COLORS, BUTTON_LABELS, keyColor, padAxisLabel, padButtonLabel } from "../lib/keys";
import { useStore, type DocState, type DocView } from "../lib/store";
import type { ChudEvent, EventId, Marker, Micros } from "../lib/types";
import { holdsOf, laneIdsInRange, laneIds, minorStep, pointsOf, tickStep, type Lane } from "./lanes";

export const HEADER_W = 140;
export const RULER_H = 30;
const MIN_SCALE = 1; // µs per pixel
const MAX_SCALE = 2e6;

const C = {
  lane: "rgba(10, 39, 80, 0.30)",
  laneAlt: "rgba(10, 39, 80, 0.16)",
  grid: "rgba(109, 140, 171, 0.09)",
  gridMajor: "rgba(109, 140, 171, 0.2)",
  header: "rgba(4, 22, 48, 0.94)",
  ruler: "rgba(4, 22, 48, 0.9)",
  text: "#e8eef6",
  text2: "#a9bdd4",
  text3: "#7189a6",
  gold: "#f9c83a",
  gold2: "#ffe486",
  cyan: "#38d6ff",
  rec: "#e0342c",
  pink: "#ff86b0",
  violet: "#a99bff",
  mint: "#8fe8c2",
  sky: "#82b6ff",
  sel: "#ffffff",
};

export interface TimelineApi {
  zoomBy(f: number, at?: Micros): void;
  fit(): void;
  reveal(t: Micros, center?: boolean): void;
  view(): DocView;
  width(): number;
}

export interface Hit {
  index: number;
  id: EventId;
  lane: Lane;
  hold?: Hold;
  edge?: boolean;
}

export interface TimelineContext {
  clientX: number;
  clientY: number;
  hit: Hit | null;
  t: Micros;
  lane: Lane | null;
  marker: number | null;
}

type Drag =
  | { mode: "pan"; x0: number; y0: number; t00: Micros; sy0: number }
  | { mode: "rubber"; x0: number; y0: number; x1: number; y1: number; base: Set<EventId> }
  | {
      mode: "move";
      x0: number;
      anchorT: Micros;
      ids: Set<EventId>;
      dt: Micros;
      minT: Micros;
      active: boolean;
      snapT: Micros | null;
      /** Clicked group, selected alone on a click without drag. */
      collapse: EventId[] | null;
    }
  | { mode: "resize"; downId: EventId; start: Micros; end: Micros; limit: Micros; x0: number; newEnd: Micros; snapT: Micros | null }
  | { mode: "scrub"; x0: number; ta: Micros; tb: Micros; active: boolean }
  | { mode: "marker"; index: number; x0: number; t: Micros; active: boolean };

interface Props {
  doc: DocState;
  model: Model;
  lanes: Lane[];
  contentH: number;
  snap: boolean;
  playhead: Micros | null;
  onContext: (c: TimelineContext) => void;
  onOpenEvent: (id: EventId) => void;
  onAddMarker: (t: Micros) => void;
  onMarkers: (m: Marker[]) => void;
}

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  const rr = Math.max(0, Math.min(r, w / 2, h / 2));
  ctx.beginPath();
  ctx.moveTo(x + rr, y);
  ctx.arcTo(x + w, y, x + w, y + h, rr);
  ctx.arcTo(x + w, y + h, x, y + h, rr);
  ctx.arcTo(x, y + h, x, y, rr);
  ctx.arcTo(x, y, x + w, y, rr);
  ctx.closePath();
}

function alpha(hex: string, a: number): string {
  const n = parseInt(hex.slice(1), 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${a})`;
}

function rulerLabel(t: Micros, step: Micros): string {
  if (step < 1000) return `${(t / 1000).toFixed(step < 100 ? 2 : 1)} ms`;
  return fmtTime(t, step >= 1e6 ? 0 : step >= 1e5 ? 1 : step >= 1e4 ? 2 : 3);
}

export const Timeline = forwardRef<TimelineApi, Props>(function Timeline(props, ref) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const size = useRef({ w: 800, h: 400 });
  const [view, setView] = useState<DocView>(() => props.doc.view ?? { t0: 0, scale: 1000 });
  const [scrollY, setScrollY] = useState(0);
  const drag = useRef<Drag | null>(null);
  const hover = useRef<{ x: number; y: number } | null>(null);
  const [tip, setTip] = useState<{ x: number; y: number; text: string; sub?: string } | null>(null);
  const [cursorStyle, setCursorStyle] = useState("default");
  const raf = useRef(0);
  const fitted = useRef(Boolean(props.doc.view));

  // Latest props/state for the draw and event handlers.
  const S = useRef({ props, view, scrollY });
  S.current = { props, view, scrollY };

  const patchDoc = useStore((s) => s.patchDoc);

  // Persist the view on the document (debounced).
  useEffect(() => {
    const id = window.setTimeout(() => patchDoc(props.doc.id, { view }), 250);
    return () => window.clearTimeout(id);
  }, [view, props.doc.id, patchDoc]);

  const xOf = (t: Micros, v = S.current.view) => HEADER_W + (t - v.t0) / v.scale;
  const tAt = (x: number, v = S.current.view) => v.t0 + (x - HEADER_W) * v.scale;

  const clampView = useCallback((v: DocView): DocView => {
    const { model } = S.current.props;
    const scale = clamp(v.scale, MIN_SCALE, MAX_SCALE);
    const visible = (size.current.w - HEADER_W) * scale;
    const maxT0 = Math.max(0, model.duration - visible * 0.5);
    return { scale, t0: clamp(v.t0, -visible * 0.05, maxT0 + visible * 0.05) };
  }, []);

  const maxScroll = () => Math.max(0, S.current.props.contentH - (size.current.h - RULER_H));

  const fit = useCallback(() => {
    const { model } = S.current.props;
    const w = Math.max(100, size.current.w - HEADER_W - 24);
    const dur = Math.max(model.duration, 1_000_000);
    setView(clampView({ t0: -dur * 0.01, scale: (dur * 1.03) / w }));
  }, [clampView]);

  const zoomBy = useCallback(
    (f: number, at?: Micros) => {
      setView((v) => {
        const anchor = at ?? v.t0 + ((size.current.w - HEADER_W) * v.scale) / 2;
        const scale = clamp(v.scale / f, MIN_SCALE, MAX_SCALE);
        return clampView({ scale, t0: anchor - (anchor - v.t0) * (scale / v.scale) });
      });
    },
    [clampView],
  );

  const reveal = useCallback(
    (t: Micros, center = false) => {
      setView((v) => {
        const span = (size.current.w - HEADER_W) * v.scale;
        if (!center && t >= v.t0 + span * 0.05 && t <= v.t0 + span * 0.95) return v;
        return clampView({ ...v, t0: t - span * (center ? 0.5 : 0.1) });
      });
    },
    [clampView],
  );

  useImperativeHandle(ref, () => ({ zoomBy, fit, reveal, view: () => S.current.view, width: () => size.current.w - HEADER_W }), [
    zoomBy,
    fit,
    reveal,
  ]);

  // ───────────────────────────── drawing ─────────────────────────────

  const draw = useCallback(() => {
    raf.current = 0;
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const { w, h } = size.current;
    const { props: p, view: v, scrollY: sy } = S.current;
    const { doc, model, lanes } = p;
    const events = doc.events;
    const sel = doc.selection;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    const x = (t: Micros) => HEADER_W + (t - v.t0) / v.scale;
    const tMin = v.t0;
    const tMax = v.t0 + (w - HEADER_W) * v.scale;
    const d = drag.current;
    const moving = d?.mode === "move" && d.active ? d : null;
    const resizing = d?.mode === "resize" ? d : null;
    const off = (id: EventId) => (moving && moving.ids.has(id) ? moving.dt : 0);
    const pad = moving ? Math.abs(moving.dt) : 0;

    const major = tickStep(v.scale);
    const minor = minorStep(major);

    // ── lane backgrounds and grid ──
    ctx.save();
    ctx.beginPath();
    ctx.rect(HEADER_W, RULER_H, w - HEADER_W, h - RULER_H);
    ctx.clip();

    for (let i = 0; i < lanes.length; i++) {
      const L = lanes[i];
      const y = RULER_H + L.y - sy;
      if (y > h || y + L.h < RULER_H) continue;
      ctx.fillStyle = i % 2 ? C.laneAlt : C.lane;
      ctx.fillRect(HEADER_W, y, w - HEADER_W, L.h);
    }
    // Area past the end of the macro.
    const endX = x(model.duration);
    if (endX < w) {
      ctx.fillStyle = "rgba(0, 6, 16, 0.35)";
      ctx.fillRect(Math.max(HEADER_W, endX), RULER_H, w - Math.max(HEADER_W, endX), h - RULER_H);
    }
    // Grid
    const gStart = Math.floor(tMin / minor) * minor;
    for (let t = gStart; t <= tMax; t += minor) {
      const gx = Math.round(x(t)) + 0.5;
      const isMajor = Math.abs(t / major - Math.round(t / major)) < 1e-6;
      ctx.fillStyle = isMajor ? C.gridMajor : C.grid;
      ctx.fillRect(gx, RULER_H, 1, h - RULER_H);
    }

    // Markers (lines/bands through the lanes)
    doc.markers.forEach((m, mi) => {
      const mt = d?.mode === "marker" && d.index === mi && d.active ? d.t : m.t;
      const mx = x(mt);
      if (m.end !== null) {
        const ex = x(m.end + (mt - m.t));
        ctx.fillStyle = alpha(m.color, 0.06);
        ctx.fillRect(mx, RULER_H, ex - mx, h - RULER_H);
      }
      ctx.fillStyle = alpha(m.color, 0.35);
      ctx.fillRect(Math.round(mx), RULER_H, 1, h - RULER_H);
    });

    // Range
    if (doc.range || (d?.mode === "scrub" && d.active)) {
      const [ra, rb] = d?.mode === "scrub" && d.active ? [Math.min(d.ta, d.tb), Math.max(d.ta, d.tb)] : doc.range!;
      ctx.fillStyle = "rgba(249, 200, 58, 0.07)";
      ctx.fillRect(x(ra), RULER_H, x(rb) - x(ra), h - RULER_H);
      ctx.fillStyle = "rgba(249, 200, 58, 0.5)";
      ctx.fillRect(Math.round(x(ra)), RULER_H, 1, h - RULER_H);
      ctx.fillRect(Math.round(x(rb)), RULER_H, 1, h - RULER_H);
    }

    ctx.font = "600 10.5px 'Sora Variable', 'Segoe UI', sans-serif";
    ctx.textBaseline = "middle";

    // ── lanes ──
    for (const L of lanes) {
      const y = RULER_H + L.y - sy;
      if (y > h || y + L.h < RULER_H) continue;
      const hs = holdsOf(model, L);
      if (hs) {
        let color = C.gold;
        let label = "";
        if (L.kind === "key") {
          const r = model.keyRows[L.row];
          color = keyColor(r.key);
          label = r.label;
        } else if (L.kind === "button") {
          const r = model.buttonRows[L.row];
          color = BUTTON_COLORS[r.button];
          label = BUTTON_LABELS[r.button];
        } else {
          const r = model.padRows[L.row];
          color = C.pink;
          label = padButtonLabel(r.button);
        }
        const by = y + 3;
        const bh = L.h - 6;
        const first = lowerBound(hs.holds.length, (i) => hs.holds[i].end, tMin - pad);
        for (let i = first; i < hs.holds.length; i++) {
          const hd = hs.holds[i];
          if (hd.start > tMax + pad) break;
          const downId = events[hd.down].id;
          const o = off(downId);
          let end = hd.end;
          if (resizing && resizing.downId === downId) end = resizing.newEnd;
          const x0 = x(hd.start + o);
          const x1 = Math.max(x0 + 2, x(end + o));
          const selected = sel.has(downId) || (hd.up >= 0 && sel.has(events[hd.up].id));
          const open = hd.up < 0;
          const bw = x1 - x0;
          roundRect(ctx, x0, by, bw, bh, 4);
          if (open) {
            const g = ctx.createLinearGradient(x0, 0, x1, 0);
            g.addColorStop(0, alpha(color, 0.75));
            g.addColorStop(1, alpha(C.rec, 0.25));
            ctx.fillStyle = g;
          } else ctx.fillStyle = alpha(color, selected ? 0.95 : 0.62);
          ctx.fill();
          if (selected) {
            ctx.strokeStyle = C.sel;
            ctx.lineWidth = 1.5;
            ctx.stroke();
          } else if (open) {
            ctx.strokeStyle = alpha(C.rec, 0.9);
            ctx.setLineDash([3, 3]);
            ctx.lineWidth = 1;
            ctx.stroke();
            ctx.setLineDash([]);
          }
          // Press edge accent
          ctx.fillStyle = selected ? C.sel : alpha(color, 1);
          ctx.fillRect(x0, by, Math.min(2, bw), bh);
          if (bw > label.length * 6.6 + 10) {
            ctx.fillStyle = selected ? "#011128" : "rgba(1, 17, 40, 0.92)";
            ctx.fillText(label, x0 + 5, by + bh / 2 + 0.5);
          }
        }
        // Auto-repeats and orphan releases
        for (const ri of hs.repeats) {
          const e = events[ri];
          if (e.t < tMin - pad || e.t > tMax + pad) continue;
          const rx = x(e.t + off(e.id));
          ctx.fillStyle = sel.has(e.id) ? C.sel : alpha(color, 0.9);
          ctx.fillRect(rx - 0.5, y + L.h - 7, 1.5, 4);
        }
        for (const oi of hs.orphans) {
          const e = events[oi];
          if (e.t < tMin - pad || e.t > tMax + pad) continue;
          const ox = x(e.t + off(e.id));
          ctx.fillStyle = sel.has(e.id) ? C.sel : C.rec;
          ctx.beginPath();
          ctx.moveTo(ox, y + 3);
          ctx.lineTo(ox + 5, y + L.h / 2);
          ctx.lineTo(ox, y + L.h - 3);
          ctx.closePath();
          ctx.fill();
        }
        continue;
      }

      const pts = pointsOf(model, L) ?? [];
      const a = Math.max(0, lowerBound(pts.length, (i) => events[pts[i]].t, tMin - pad) - 1);
      const b = Math.min(pts.length, lowerBound(pts.length, (i) => events[pts[i]].t, tMax + pad) + 1);

      if (L.kind === "wheel") {
        const mid = y + L.h / 2;
        ctx.fillStyle = "rgba(143, 232, 194, 0.18)";
        ctx.fillRect(HEADER_W, mid, w - HEADER_W, 1);
        for (let k = a; k < b; k++) {
          const e = events[pts[k]] as Extract<ChudEvent, { type: "wheel" }>;
          const wx = x(e.t + off(e.id));
          const n = clamp(Math.abs(e.delta) / 120, 0.4, 3);
          const len = (n / 3) * (L.h / 2 - 3);
          const up = e.delta > 0;
          const col = sel.has(e.id) ? C.sel : e.horizontal ? C.violet : C.mint;
          ctx.fillStyle = col;
          ctx.fillRect(wx - 1, up ? mid - len : mid, 2.5, len);
          ctx.beginPath();
          ctx.arc(wx + 0.25, up ? mid - len : mid + len, 2.2, 0, Math.PI * 2);
          ctx.fill();
        }
      } else if (L.kind === "pointer" || L.kind === "axis") {
        const series: { color: string; val: (e: ChudEvent) => number; lo: number; hi: number }[] = [];
        if (L.kind === "pointer") {
          const bd = model.bounds ?? { minX: 0, maxX: 1, minY: 0, maxY: 1 };
          series.push({ color: C.cyan, val: (e) => (e as { x: number }).x, lo: bd.minX, hi: bd.maxX });
          series.push({ color: C.pink, val: (e) => (e as { y: number }).y, lo: bd.minY, hi: bd.maxY });
        } else {
          const ax = model.axisRows[L.row].axis;
          const trigger = ax >= 4;
          series.push({ color: trigger ? C.gold : C.pink, val: (e) => (e as { value: number }).value, lo: trigger ? 0 : -32768, hi: 32767 });
          if (!trigger) {
            ctx.fillStyle = "rgba(255, 134, 176, 0.16)";
            ctx.fillRect(HEADER_W, y + L.h / 2, w - HEADER_W, 1);
          }
        }
        const top = y + 4;
        const hh = L.h - 8;
        for (const s of series) {
          const span = s.hi - s.lo || 1;
          ctx.beginPath();
          let bpx = NaN;
          let bFirst = 0;
          let bMin = 0;
          let bMax = 0;
          let bLast = 0;
          let started = false;
          const flush = () => {
            if (Number.isNaN(bpx)) return;
            if (!started) {
              ctx.moveTo(bpx, bFirst);
              started = true;
            } else ctx.lineTo(bpx, bFirst);
            if (bMin !== bMax) {
              ctx.lineTo(bpx, bMin);
              ctx.lineTo(bpx, bMax);
            }
            ctx.lineTo(bpx, bLast);
          };
          for (let k = a; k < b; k++) {
            const e = events[pts[k]];
            const px = Math.round(x(e.t + off(e.id)));
            const vy = top + hh - ((s.val(e) - s.lo) / span) * hh;
            if (px !== bpx) {
              flush();
              bpx = px;
              bFirst = bMin = bMax = bLast = vy;
            } else {
              if (vy < bMin) bMin = vy;
              if (vy > bMax) bMax = vy;
              bLast = vy;
            }
          }
          flush();
          ctx.strokeStyle = alpha(s.color, 0.85);
          ctx.lineWidth = 1.25;
          ctx.stroke();
        }
        // Points when zoomed in enough, and the selection.
        const dense = b - a > (w - HEADER_W) / 5;
        for (let k = a; k < b; k++) {
          const e = events[pts[k]];
          const isSel = sel.has(e.id);
          if (dense && !isSel) continue;
          const px = x(e.t + off(e.id));
          for (const s of series) {
            const vy = top + hh - ((s.val(e) - s.lo) / (s.hi - s.lo || 1)) * hh;
            ctx.fillStyle = isSel ? C.sel : s.color;
            ctx.beginPath();
            ctx.arc(px, vy, isSel ? 2.6 : 1.8, 0, Math.PI * 2);
            ctx.fill();
          }
        }
      } else if (L.kind === "raw") {
        const base = y + L.h - 3;
        const hh = L.h - 6;
        let bpx = NaN;
        let bmax = 0;
        let bsel = false;
        const flush = () => {
          if (Number.isNaN(bpx)) return;
          const bh = Math.max(1, Math.sqrt(bmax / model.rawMax) * hh);
          ctx.fillStyle = bsel ? C.sel : "rgba(169, 155, 255, 0.75)";
          ctx.fillRect(bpx, base - bh, 1, bh);
        };
        for (let k = a; k < b; k++) {
          const e = events[pts[k]] as Extract<ChudEvent, { type: "mouse_raw" }>;
          const px = Math.round(x(e.t + off(e.id)));
          const m = Math.abs(e.dx) + Math.abs(e.dy);
          if (px !== bpx) {
            flush();
            bpx = px;
            bmax = m;
            bsel = sel.has(e.id);
          } else {
            if (m > bmax) bmax = m;
            if (sel.has(e.id)) bsel = true;
          }
        }
        flush();
      }
    }

    // Snap guide
    const snapT = moving?.snapT ?? resizing?.snapT ?? null;
    if (snapT !== null) {
      ctx.fillStyle = C.cyan;
      ctx.fillRect(Math.round(x(snapT)), RULER_H, 1, h - RULER_H);
    }

    // Cursor
    const cx = x(doc.cursor);
    ctx.fillStyle = C.gold;
    ctx.fillRect(Math.round(cx), RULER_H, 1, h - RULER_H);

    // Playhead
    if (p.playhead !== null) {
      const px = Math.round(x(p.playhead));
      ctx.shadowColor = C.cyan;
      ctx.shadowBlur = 10;
      ctx.fillStyle = C.cyan;
      ctx.fillRect(px - 0.5, RULER_H, 2, h - RULER_H);
      ctx.shadowBlur = 0;
    }

    // Rubber band
    if (d?.mode === "rubber") {
      const rx = Math.min(d.x0, d.x1);
      const ry = Math.min(d.y0, d.y1);
      ctx.fillStyle = "rgba(56, 214, 255, 0.08)";
      ctx.strokeStyle = "rgba(56, 214, 255, 0.7)";
      ctx.lineWidth = 1;
      ctx.fillRect(rx, ry, Math.abs(d.x1 - d.x0), Math.abs(d.y1 - d.y0));
      ctx.strokeRect(rx + 0.5, ry + 0.5, Math.abs(d.x1 - d.x0), Math.abs(d.y1 - d.y0));
    }

    // Vertical scrollbar
    const ms = maxScroll();
    if (ms > 0) {
      const vh = h - RULER_H;
      const th = Math.max(24, (vh * vh) / p.contentH);
      const ty = RULER_H + (sy / ms) * (vh - th);
      ctx.fillStyle = "rgba(109, 140, 171, 0.35)";
      roundRect(ctx, w - 6, ty, 4, th, 2);
      ctx.fill();
    }
    ctx.restore();

    // ── header column ──
    ctx.fillStyle = C.header;
    ctx.fillRect(0, RULER_H, HEADER_W, h - RULER_H);
    ctx.fillStyle = "rgba(109, 140, 171, 0.22)";
    ctx.fillRect(HEADER_W - 1, 0, 1, h);
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, RULER_H, HEADER_W, h - RULER_H);
    ctx.clip();
    for (const L of lanes) {
      const y = RULER_H + L.y - sy;
      if (y > h || y + L.h < RULER_H) continue;
      const secColor = L.section === "keyboard" ? C.gold : L.section === "mouse" ? C.cyan : C.pink;
      ctx.fillStyle = alpha(secColor, 0.55);
      ctx.fillRect(0, y + 1, 3, L.h - 2);
      const mid = y + L.h / 2 + 0.5;
      ctx.font = "500 11px 'Sora Variable', 'Segoe UI', sans-serif";
      if (L.kind === "key") {
        const r = model.keyRows[L.row];
        ctx.fillStyle = keyColor(r.key);
        ctx.beginPath();
        ctx.arc(14, mid, 3, 0, Math.PI * 2);
        ctx.fill();
        ctx.fillStyle = C.text2;
        ctx.fillText(r.label, 24, mid, HEADER_W - 56);
        ctx.fillStyle = C.text3;
        ctx.font = "10px 'JetBrains Mono Variable', monospace";
        const n = r.holds.length;
        ctx.textAlign = "right";
        ctx.fillText(String(n), HEADER_W - 10, mid);
        ctx.textAlign = "left";
      } else if (L.kind === "button") {
        const r = model.buttonRows[L.row];
        ctx.fillStyle = BUTTON_COLORS[r.button];
        ctx.beginPath();
        ctx.arc(14, mid, 3, 0, Math.PI * 2);
        ctx.fill();
        ctx.fillStyle = C.text2;
        ctx.fillText(tr(`button.${r.button}`), 24, mid, HEADER_W - 56);
        ctx.fillStyle = C.text3;
        ctx.font = "10px 'JetBrains Mono Variable', monospace";
        ctx.textAlign = "right";
        ctx.fillText(String(r.holds.length), HEADER_W - 10, mid);
        ctx.textAlign = "left";
      } else if (L.kind === "pad") {
        const r = model.padRows[L.row];
        ctx.fillStyle = C.text2;
        ctx.fillText(`${r.pad + 1} · ${padButtonLabel(r.button)}`, 12, mid, HEADER_W - 20);
      } else if (L.kind === "axis") {
        const r = model.axisRows[L.row];
        ctx.fillStyle = C.text2;
        ctx.fillText(`${r.pad + 1} · ${padAxisLabel(r.axis)}`, 12, mid, HEADER_W - 20);
      } else if (L.kind === "wheel") {
        ctx.fillStyle = C.text2;
        ctx.fillText(tr("lane.wheel"), 12, mid);
      } else if (L.kind === "pointer") {
        ctx.fillStyle = C.text2;
        ctx.fillText(tr("lane.pointer"), 12, mid - 8);
        ctx.font = "600 10px 'JetBrains Mono Variable', monospace";
        ctx.fillStyle = C.cyan;
        ctx.fillText("X", 12, mid + 8);
        ctx.fillStyle = C.pink;
        ctx.fillText("Y", 24, mid + 8);
        const bd = model.bounds;
        if (bd) {
          ctx.fillStyle = C.text3;
          ctx.font = "9.5px 'JetBrains Mono Variable', monospace";
          ctx.fillText(`${bd.minX}…${bd.maxX}`, 38, mid + 8, HEADER_W - 46);
        }
      } else if (L.kind === "raw") {
        ctx.fillStyle = C.text2;
        ctx.fillText(tr("lane.raw"), 12, mid);
      }
    }
    ctx.restore();

    // ── ruler ──
    ctx.fillStyle = C.ruler;
    ctx.fillRect(0, 0, w, RULER_H);
    ctx.fillStyle = "rgba(109, 140, 171, 0.25)";
    ctx.fillRect(0, RULER_H - 1, w, 1);
    ctx.save();
    ctx.beginPath();
    ctx.rect(HEADER_W, 0, w - HEADER_W, RULER_H);
    ctx.clip();
    ctx.font = "10.5px 'JetBrains Mono Variable', monospace";
    for (let t = gStart; t <= tMax; t += minor) {
      const gx = Math.round(x(t)) + 0.5;
      const isMajor = Math.abs(t / major - Math.round(t / major)) < 1e-6;
      ctx.fillStyle = isMajor ? C.text3 : "rgba(113, 137, 166, 0.45)";
      ctx.fillRect(gx, isMajor ? RULER_H - 10 : RULER_H - 5, 1, isMajor ? 10 : 5);
      if (isMajor && t >= 0) {
        ctx.fillStyle = C.text3;
        ctx.fillText(rulerLabel(t, major), gx + 4, 10);
      }
    }
    // Range in ruler
    if (doc.range) {
      const [ra, rb] = doc.range;
      ctx.fillStyle = "rgba(249, 200, 58, 0.25)";
      ctx.fillRect(x(ra), RULER_H - 6, x(rb) - x(ra), 6);
    }
    // Markers
    ctx.font = "600 10px 'Sora Variable', 'Segoe UI', sans-serif";
    doc.markers.forEach((m, mi) => {
      const mt = d?.mode === "marker" && d.index === mi && d.active ? d.t : m.t;
      const mx = x(mt);
      ctx.fillStyle = m.color;
      ctx.beginPath();
      ctx.moveTo(mx, RULER_H - 1);
      ctx.lineTo(mx, 14);
      ctx.lineTo(mx + 8, 18);
      ctx.lineTo(mx, 22);
      ctx.fill();
      ctx.fillRect(mx - 0.5, 12, 1.5, RULER_H - 12);
      if (m.label) {
        ctx.fillStyle = alpha(m.color, 0.95);
        ctx.fillText(m.label, mx + 10, 21, 140);
      }
    });
    // Cursor head
    ctx.fillStyle = C.gold;
    ctx.beginPath();
    ctx.moveTo(cx - 5, RULER_H - 8);
    ctx.lineTo(cx + 5, RULER_H - 8);
    ctx.lineTo(cx, RULER_H - 1);
    ctx.fill();
    if (p.playhead !== null) {
      const px = x(p.playhead);
      ctx.fillStyle = C.cyan;
      ctx.beginPath();
      ctx.moveTo(px - 5, 0);
      ctx.lineTo(px + 5, 0);
      ctx.lineTo(px, 8);
      ctx.fill();
    }
    ctx.restore();

    // Header corner: time under the mouse
    ctx.fillStyle = C.header;
    ctx.fillRect(0, 0, HEADER_W - 1, RULER_H - 1);
    ctx.font = "11px 'JetBrains Mono Variable', monospace";
    ctx.fillStyle = C.gold2;
    const ht = hover.current && hover.current.x > HEADER_W ? tAt(hover.current.x, v) : doc.cursor;
    ctx.fillText(fmtTime(Math.max(0, ht), 3), 12, RULER_H / 2);

    if (!lanes.length) {
      ctx.font = "500 13px 'Sora Variable', 'Segoe UI', sans-serif";
      ctx.fillStyle = C.text3;
      ctx.textAlign = "center";
      ctx.fillText(tr("studio.empty_timeline"), HEADER_W + (w - HEADER_W) / 2, RULER_H + 60);
      ctx.textAlign = "left";
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const requestDraw = useCallback(() => {
    if (!raf.current) raf.current = requestAnimationFrame(draw);
  }, [draw]);

  useLayoutEffect(() => {
    requestDraw();
  });

  // Size
  useLayoutEffect(() => {
    const wrap = wrapRef.current!;
    const canvas = canvasRef.current!;
    const ro = new ResizeObserver(() => {
      const r = wrap.getBoundingClientRect();
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      size.current = { w: Math.max(200, r.width), h: Math.max(120, r.height) };
      canvas.width = Math.round(size.current.w * dpr);
      canvas.height = Math.round(size.current.h * dpr);
      canvas.style.width = `${size.current.w}px`;
      canvas.style.height = `${size.current.h}px`;
      if (!fitted.current) {
        fitted.current = true;
        fit();
      }
      setScrollY((s) => clamp(s, 0, maxScroll()));
      draw();
    });
    ro.observe(wrap);
    document.fonts?.ready.then(requestDraw);
    return () => ro.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Follow the playhead.
  useEffect(() => {
    if (props.playhead === null) return;
    const v = S.current.view;
    const span = (size.current.w - HEADER_W) * v.scale;
    if (props.playhead > v.t0 + span * 0.92 || props.playhead < v.t0) setView(clampView({ ...v, t0: props.playhead - span * 0.08 }));
  }, [props.playhead, clampView]);

  // ───────────────────────────── hit testing ─────────────────────────────

  const laneAt = (y: number): Lane | null => {
    const cy = y - RULER_H + S.current.scrollY;
    for (const L of S.current.props.lanes) if (cy >= L.y && cy < L.y + L.h) return L;
    return null;
  };

  const hitTest = (px: number, py: number): Hit | null => {
    if (px < HEADER_W || py < RULER_H) return null;
    const L = laneAt(py);
    if (!L) return null;
    const { doc, model } = S.current.props;
    const events = doc.events;
    const v = S.current.view;
    const t = tAt(px);
    const tol = 4 * v.scale;
    const hs = holdsOf(model, L);
    if (hs) {
      // Point marks first (repeats, orphans), then holds.
      for (const i of [...hs.orphans, ...hs.repeats]) {
        if (Math.abs(events[i].t - t) <= tol) return { index: i, id: events[i].id, lane: L };
      }
      const k = lowerBound(hs.holds.length, (i) => hs.holds[i].end, t - tol);
      for (let i = k; i < hs.holds.length; i++) {
        const hd = hs.holds[i];
        if (hd.start - tol > t) break;
        const minEnd = hd.start + 2 * v.scale;
        const end = Math.max(hd.end, minEnd);
        if (t >= hd.start - tol && t <= end + tol) {
          const edge = hd.up >= 0 && Math.abs(xOf(end) - px) <= 5 && xOf(end) - xOf(hd.start) > 10;
          return { index: hd.down, id: events[hd.down].id, lane: L, hold: hd, edge };
        }
      }
      return null;
    }
    const pts = pointsOf(model, L) ?? [];
    if (!pts.length) return null;
    const k = lowerBound(pts.length, (i) => events[pts[i]].t, t);
    let best = -1;
    let bestD = Infinity;
    for (const j of [k - 1, k]) {
      if (j < 0 || j >= pts.length) continue;
      const dd = Math.abs(events[pts[j]].t - t);
      if (dd < bestD) {
        bestD = dd;
        best = pts[j];
      }
    }
    if (best >= 0 && bestD <= 5 * v.scale) return { index: best, id: events[best].id, lane: L };
    return null;
  };

  const markerAt = (px: number): number | null => {
    const { doc } = S.current.props;
    let best: number | null = null;
    let bd = 7;
    doc.markers.forEach((m, i) => {
      const dd = Math.abs(xOf(m.t) - px);
      if (dd < bd) {
        bd = dd;
        best = i;
      }
    });
    return best;
  };

  const snapTime = (t: Micros, exclude: Set<EventId> | null): { t: Micros; snapped: Micros | null } => {
    const { props: p, view: v } = S.current;
    if (!p.snap) return { t, snapped: null };
    const thr = 7 * v.scale;
    let best = t;
    let bestD = thr;
    let snapped: Micros | null = null;
    const consider = (c: Micros, weight = 1) => {
      const dd = Math.abs(c - t) * weight;
      if (dd < bestD) {
        bestD = dd;
        best = c;
        snapped = c;
      }
    };
    const ev = p.doc.events;
    const i0 = lowerBound(ev.length, (i) => ev[i].t, t - thr);
    for (let i = i0; i < ev.length && ev[i].t <= t + thr; i++) {
      if (exclude?.has(ev[i].id)) continue;
      if (ev[i].type === "mouse_move" || ev[i].type === "mouse_raw") continue;
      consider(ev[i].t);
    }
    p.doc.markers.forEach((m) => consider(m.t));
    consider(p.doc.cursor, 1.1);
    consider(0, 1.1);
    const g = minorStep(tickStep(v.scale));
    consider(Math.round(t / g) * g, 1.6);
    return { t: best, snapped };
  };

  // ───────────────────────────── interaction ─────────────────────────────

  const local = (e: { clientX: number; clientY: number }) => {
    const r = canvasRef.current!.getBoundingClientRect();
    return { px: e.clientX - r.left, py: e.clientY - r.top };
  };

  const setSelection = (s: Set<EventId>) => patchDoc(S.current.props.doc.id, { selection: s });

  const onMouseDown = (e: React.MouseEvent) => {
    const { px, py } = local(e);
    const { doc, model } = S.current.props;
    canvasRef.current?.focus();
    if (e.button === 1 || (e.button === 0 && e.altKey && e.shiftKey)) {
      e.preventDefault();
      drag.current = { mode: "pan", x0: px, y0: py, t00: S.current.view.t0, sy0: S.current.scrollY };
      setCursorStyle("grabbing");
      return;
    }
    if (e.button !== 0) return;
    if (py < RULER_H && px >= HEADER_W) {
      const mi = markerAt(px);
      if (mi !== null) {
        drag.current = { mode: "marker", index: mi, x0: px, t: doc.markers[mi].t, active: false };
        return;
      }
      const t = Math.max(0, snapTime(tAt(px), null).t);
      drag.current = { mode: "scrub", x0: px, ta: t, tb: t, active: false };
      return;
    }
    if (px < HEADER_W) {
      const L = laneAt(py);
      if (!L) return;
      const ids = laneIds(doc.events, model, L);
      const next = e.ctrlKey || e.shiftKey ? new Set([...doc.selection, ...ids]) : new Set(ids);
      setSelection(next);
      return;
    }
    const hit = hitTest(px, py);
    if (hit?.edge && hit.hold && hit.hold.up >= 0) {
      const hs = holdsOf(model, hit.lane)!;
      const k = hs.holds.indexOf(hit.hold);
      const next = hs.holds[k + 1];
      drag.current = {
        mode: "resize",
        downId: hit.id,
        start: hit.hold.start,
        end: hit.hold.end,
        limit: next ? next.start - 1 : Number.MAX_SAFE_INTEGER,
        x0: px,
        newEnd: hit.hold.end,
        snapT: null,
      };
      return;
    }
    if (hit) {
      const group = hit.hold
        ? [hit.id, ...(hit.hold.up >= 0 ? [doc.events[hit.hold.up].id] : [])]
        : [hit.id];
      let sel = doc.selection;
      if (e.ctrlKey || e.metaKey) {
        sel = new Set(sel);
        const has = group.every((g) => sel.has(g));
        group.forEach((g) => (has ? sel.delete(g) : sel.add(g)));
      } else if (e.shiftKey) {
        sel = new Set([...sel, ...group]);
      } else if (!group.every((g) => sel.has(g))) {
        sel = new Set(group);
      }
      const collapse = sel === doc.selection && sel.size > group.length && !(e.ctrlKey || e.metaKey || e.shiftKey) ? group : null;
      if (sel !== doc.selection) setSelection(sel);
      if (sel.has(hit.id)) {
        const ids = withPartners(doc.events, sel);
        let minT = Infinity;
        for (const ev of doc.events) if (ids.has(ev.id) && ev.t < minT) minT = ev.t;
        drag.current = {
          mode: "move",
          x0: px,
          anchorT: doc.events[hit.index].t,
          ids,
          dt: 0,
          minT,
          active: false,
          snapT: null,
          collapse,
        };
      }
      return;
    }
    drag.current = {
      mode: "rubber",
      x0: px,
      y0: py,
      x1: px,
      y1: py,
      base: e.ctrlKey || e.shiftKey ? new Set(doc.selection) : new Set(),
    };
    if (!(e.ctrlKey || e.shiftKey) && doc.selection.size) setSelection(new Set());
  };

  const onMouseMoveWin = useCallback((e: MouseEvent) => {
    const d = drag.current;
    if (!d) return;
    const { px, py } = local(e);
    const v = S.current.view;
    if (d.mode === "pan") {
      setView(clampView({ ...v, t0: d.t00 - (px - d.x0) * v.scale }));
      setScrollY(clamp(d.sy0 - (py - d.y0), 0, maxScroll()));
    } else if (d.mode === "rubber") {
      d.x1 = clamp(px, HEADER_W, size.current.w);
      d.y1 = clamp(py, RULER_H, size.current.h);
      requestDraw();
    } else if (d.mode === "move") {
      if (!d.active && Math.abs(px - d.x0) < 4) return;
      d.active = true;
      const raw = (px - d.x0) * v.scale;
      const s = e.altKey ? { t: d.anchorT + raw, snapped: null } : snapTime(d.anchorT + raw, d.ids);
      d.dt = Math.round(Math.max(s.t - d.anchorT, -d.minT));
      d.snapT = s.snapped;
      const sign = d.dt >= 0 ? "+" : "−";
      setTip({ x: px, y: py, text: `${sign}${fmtDuration(Math.abs(d.dt))}`, sub: tr("studio.alt_no_snap") });
      setCursorStyle("grabbing");
      requestDraw();
    } else if (d.mode === "resize") {
      const raw = d.end + (px - d.x0) * v.scale;
      const s = e.altKey ? { t: raw, snapped: null } : snapTime(raw, null);
      d.newEnd = Math.round(clamp(s.t, d.start + 1000, d.limit));
      d.snapT = s.snapped;
      setTip({ x: px, y: py, text: fmtDuration(d.newEnd - d.start) });
      requestDraw();
    } else if (d.mode === "scrub") {
      if (!d.active && Math.abs(px - d.x0) < 4) return;
      d.active = true;
      d.tb = Math.max(0, e.altKey ? tAt(px) : snapTime(tAt(px), null).t);
      setTip({ x: px, y: RULER_H + 6, text: fmtDuration(Math.abs(d.tb - d.ta)) });
      requestDraw();
    } else if (d.mode === "marker") {
      if (!d.active && Math.abs(px - d.x0) < 3) return;
      d.active = true;
      d.t = Math.max(0, Math.round(e.altKey ? tAt(px) : snapTime(tAt(px), null).t));
      setTip({ x: px, y: RULER_H + 6, text: fmtTime(d.t, 3) });
      requestDraw();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const onMouseUpWin = useCallback(() => {
    const d = drag.current;
    drag.current = null;
    if (!d) return;
    const { doc, model, lanes } = S.current.props;
    const st = useStore.getState();
    setTip(null);
    setCursorStyle("default");
    if (d.mode === "rubber") {
      const ta = Math.min(tAt(d.x0), tAt(d.x1));
      const tb = Math.max(tAt(d.x0), tAt(d.x1));
      const ya = Math.min(d.y0, d.y1) - RULER_H + S.current.scrollY;
      const yb = Math.max(d.y0, d.y1) - RULER_H + S.current.scrollY;
      if (Math.abs(d.x1 - d.x0) < 3 && Math.abs(d.y1 - d.y0) < 3) {
        // Plain click on empty space: move the cursor.
        st.patchDoc(doc.id, { cursor: Math.max(0, Math.round(snapTime(tAt(d.x0), null).t)), range: null });
      } else {
        const next = new Set(d.base);
        for (const L of lanes) {
          if (L.y + L.h < ya || L.y > yb) continue;
          for (const id of laneIdsInRange(doc.events, model, L, ta, tb)) next.add(id);
        }
        st.patchDoc(doc.id, { selection: next });
      }
    } else if (d.mode === "move" && d.active && d.dt !== 0) {
      st.edit(doc.id, { op: "shift", ids: [...d.ids], dt: d.dt });
    } else if (d.mode === "move" && !d.active && d.collapse) {
      st.patchDoc(doc.id, { selection: new Set(d.collapse) });
    } else if (d.mode === "resize" && d.newEnd !== d.end) {
      st.edit(doc.id, { op: "set_hold_duration", id: d.downId, duration: d.newEnd - d.start });
    } else if (d.mode === "scrub") {
      if (d.active && d.ta !== d.tb) {
        const a = Math.round(Math.min(d.ta, d.tb));
        const b = Math.round(Math.max(d.ta, d.tb));
        st.patchDoc(doc.id, { range: [a, b], cursor: a });
      } else st.patchDoc(doc.id, { cursor: Math.round(d.ta), range: null });
    } else if (d.mode === "marker" && d.active) {
      const ms = doc.markers.map((m, i) => {
        if (i !== d.index) return m;
        const dt = d.t - m.t;
        return { ...m, t: d.t, end: m.end !== null ? m.end + dt : null };
      });
      S.current.props.onMarkers(ms);
    }
    requestDraw();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    window.addEventListener("mousemove", onMouseMoveWin);
    window.addEventListener("mouseup", onMouseUpWin);
    return () => {
      window.removeEventListener("mousemove", onMouseMoveWin);
      window.removeEventListener("mouseup", onMouseUpWin);
    };
  }, [onMouseMoveWin, onMouseUpWin]);

  const onHover = (e: React.MouseEvent) => {
    const { px, py } = local(e);
    hover.current = { x: px, y: py };
    requestDraw();
    if (drag.current) return;
    if (py < RULER_H) {
      const mi = px >= HEADER_W ? markerAt(px) : null;
      setCursorStyle(mi !== null ? "ew-resize" : px >= HEADER_W ? "text" : "default");
      if (mi !== null) {
        const m = S.current.props.doc.markers[mi];
        setTip({ x: px, y: py, text: m.label || tr("studio.marker"), sub: fmtTime(m.t, 3) });
      } else setTip(null);
      return;
    }
    const hit = hitTest(px, py);
    if (!hit) {
      setTip(null);
      setCursorStyle(px < HEADER_W ? "pointer" : "default");
      return;
    }
    const ev = S.current.props.doc.events[hit.index];
    setCursorStyle(hit.edge ? "ew-resize" : "grab");
    let sub = fmtTime(ev.t, 3);
    if (hit.hold) {
      sub += hit.hold.up >= 0 ? ` · ${fmtDuration(hit.hold.end - hit.hold.start)}` : ` · ${tr("studio.never_released")}`;
    }
    setTip({ x: px, y: py, text: describe(ev), sub });
  };

  const onWheel = (e: React.WheelEvent) => {
    const { px } = local(e);
    const v = S.current.view;
    if (e.ctrlKey || e.metaKey) {
      const f = Math.exp(-e.deltaY * 0.0022);
      zoomBy(f, px > HEADER_W ? tAt(px) : undefined);
    } else if (e.shiftKey || Math.abs(e.deltaX) > Math.abs(e.deltaY)) {
      const dx = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
      setView(clampView({ ...v, t0: v.t0 + dx * v.scale }));
    } else if (maxScroll() > 0) {
      setScrollY((s) => clamp(s + e.deltaY, 0, maxScroll()));
    } else {
      setView(clampView({ ...v, t0: v.t0 + e.deltaY * v.scale }));
    }
  };

  // Wheel needs a non-passive listener to stop page zoom on Ctrl+wheel.
  useEffect(() => {
    const el = canvasRef.current!;
    const stop = (e: WheelEvent) => {
      if (e.ctrlKey) e.preventDefault();
    };
    el.addEventListener("wheel", stop, { passive: false });
    return () => el.removeEventListener("wheel", stop);
  }, []);

  const onContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    const { px, py } = local(e);
    const { doc } = S.current.props;
    const hit = hitTest(px, py);
    if (hit && !doc.selection.has(hit.id)) {
      const ids = hit.hold && hit.hold.up >= 0 ? [hit.id, doc.events[hit.hold.up].id] : [hit.id];
      setSelection(new Set(ids));
    }
    S.current.props.onContext({
      clientX: e.clientX,
      clientY: e.clientY,
      hit,
      t: Math.max(0, Math.round(tAt(px))),
      lane: py >= RULER_H ? laneAt(py) : null,
      marker: py < RULER_H ? markerAt(px) : null,
    });
  };

  const onDoubleClick = (e: React.MouseEvent) => {
    const { px, py } = local(e);
    if (px < HEADER_W) return;
    if (py < RULER_H) {
      if (markerAt(px) === null) S.current.props.onAddMarker(Math.max(0, Math.round(snapTime(tAt(px), null).t)));
      return;
    }
    const hit = hitTest(px, py);
    if (hit) S.current.props.onOpenEvent(hit.id);
  };

  return (
    <div ref={wrapRef} className="timeline" onMouseLeave={() => ((hover.current = null), setTip(null), requestDraw())}>
      <canvas
        ref={canvasRef}
        tabIndex={0}
        className="timeline-canvas"
        style={{ cursor: cursorStyle }}
        onMouseDown={onMouseDown}
        onMouseMove={onHover}
        onWheel={onWheel}
        onContextMenu={onContextMenu}
        onDoubleClick={onDoubleClick}
      />
      {tip && (
        <div className="timeline-tip" style={{ left: Math.min(tip.x + 14, size.current.w - 180), top: tip.y + 16 }}>
          <div>{tip.text}</div>
          {tip.sub && <div className="faint mono">{tip.sub}</div>}
        </div>
      )}
    </div>
  );
});
