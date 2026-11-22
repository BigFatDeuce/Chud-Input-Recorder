// Spatial view: monitors, pointer path, click heat map. Click to select.
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Segmented } from "../components/ui";
import { t, useLang } from "../i18n";
import { BUTTON_COLORS } from "../lib/keys";
import { useStore, type DocState } from "../lib/store";
import type { ChudEvent } from "../lib/types";

type Mode = "path" | "heat" | "clicks";

export function MapPanel({ doc }: { doc: DocState }) {
  useLang();
  const ref = useRef<HTMLCanvasElement>(null);
  const wrap = useRef<HTMLDivElement>(null);
  const [mode, setMode] = useState<Mode>("path");
  const [size, setSize] = useState({ w: 300, h: 220 });
  const [hover, setHover] = useState<{ x: number; y: number } | null>(null);
  const patchDoc = useStore((s) => s.patchDoc);

  const positional = useMemo(
    () => doc.events.filter((e) => e.type === "mouse_move" || e.type === "mouse_down" || e.type === "mouse_up" || e.type === "wheel"),
    [doc.events],
  );

  const world = useMemo(() => {
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (const m of doc.env.monitors) {
      minX = Math.min(minX, m.rect.x);
      minY = Math.min(minY, m.rect.y);
      maxX = Math.max(maxX, m.rect.x + m.rect.width);
      maxY = Math.max(maxY, m.rect.y + m.rect.height);
    }
    for (const e of positional) {
      const p = e as { x: number; y: number };
      minX = Math.min(minX, p.x);
      minY = Math.min(minY, p.y);
      maxX = Math.max(maxX, p.x);
      maxY = Math.max(maxY, p.y);
    }
    if (!Number.isFinite(minX)) return null;
    return { minX, minY, maxX: Math.max(maxX, minX + 1), maxY: Math.max(maxY, minY + 1) };
  }, [doc.env.monitors, positional]);

  useLayoutEffect(() => {
    const el = wrap.current!;
    const ro = new ResizeObserver(() => {
      const w = el.clientWidth;
      const ratio = world ? (world.maxY - world.minY) / (world.maxX - world.minX) : 0.6;
      setSize({ w, h: Math.max(140, Math.min(520, w * ratio)) });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [world]);

  const tf = useMemo(() => {
    if (!world) return null;
    const pad = 8;
    const s = Math.min((size.w - pad * 2) / (world.maxX - world.minX), (size.h - pad * 2) / (world.maxY - world.minY));
    const ox = pad + (size.w - pad * 2 - (world.maxX - world.minX) * s) / 2;
    const oy = pad + (size.h - pad * 2 - (world.maxY - world.minY) * s) / 2;
    return {
      s,
      x: (v: number) => ox + (v - world.minX) * s,
      y: (v: number) => oy + (v - world.minY) * s,
      ix: (px: number) => world.minX + (px - ox) / s,
      iy: (py: number) => world.minY + (py - oy) / s,
    };
  }, [world, size]);

  useEffect(() => {
    const c = ref.current;
    if (!c || !tf) return;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    c.width = size.w * dpr;
    c.height = size.h * dpr;
    const ctx = c.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, size.w, size.h);

    // Monitors
    ctx.font = "600 10px 'Sora Variable', sans-serif";
    for (const m of doc.env.monitors) {
      const x = tf.x(m.rect.x);
      const y = tf.y(m.rect.y);
      const w = m.rect.width * tf.s;
      const h = m.rect.height * tf.s;
      ctx.fillStyle = "rgba(10, 39, 80, 0.55)";
      ctx.fillRect(x, y, w, h);
      ctx.strokeStyle = m.primary ? "rgba(249, 200, 58, 0.55)" : "rgba(109, 140, 171, 0.45)";
      ctx.lineWidth = 1;
      ctx.strokeRect(x + 0.5, y + 0.5, w - 1, h - 1);
      ctx.fillStyle = "rgba(169, 189, 212, 0.65)";
      ctx.fillText(`${m.rect.width}×${m.rect.height}${m.primary ? " ★" : ""}`, x + 6, y + 13, w - 10);
    }

    const sel = doc.selection;
    const clicks = positional.filter((e) => e.type === "mouse_down") as Extract<ChudEvent, { type: "mouse_down" }>[];

    if (mode === "path") {
      const moves = positional.filter((e) => e.type === "mouse_move") as Extract<ChudEvent, { type: "mouse_move" }>[];
      const n = moves.length;
      if (n > 1) {
        const dur = Math.max(1, moves[n - 1].t - moves[0].t);
        ctx.lineWidth = 1.2;
        let lx = tf.x(moves[0].x);
        let ly = tf.y(moves[0].y);
        // Draw in segments so the colour follows time (gold → cyan).
        const seg = Math.max(1, Math.floor(n / 240));
        for (let i = seg; i < n; i += seg) {
          const p = moves[i];
          const f = (p.t - moves[0].t) / dur;
          ctx.strokeStyle = `rgba(${Math.round(249 - 193 * f)}, ${Math.round(200 + 14 * f)}, ${Math.round(58 + 197 * f)}, 0.55)`;
          ctx.beginPath();
          ctx.moveTo(lx, ly);
          for (let j = i - seg + 1; j <= i; j++) ctx.lineTo(tf.x(moves[j].x), tf.y(moves[j].y));
          ctx.stroke();
          lx = tf.x(p.x);
          ly = tf.y(p.y);
        }
      }
    }
    if (mode === "heat") {
      ctx.globalCompositeOperation = "lighter";
      const pts = positional.filter((e) => e.type === "mouse_down" || e.type === "mouse_move");
      const stride = Math.max(1, Math.floor(pts.length / 4000));
      const r = Math.max(10, Math.min(36, size.w / 18));
      for (let i = 0; i < pts.length; i += stride) {
        const p = pts[i] as { x: number; y: number; type: string };
        const x = tf.x(p.x);
        const y = tf.y(p.y);
        const strong = p.type === "mouse_down";
        const g = ctx.createRadialGradient(x, y, 0, x, y, strong ? r : r * 0.6);
        g.addColorStop(0, strong ? "rgba(249, 120, 40, 0.38)" : "rgba(56, 160, 255, 0.05)");
        g.addColorStop(1, "rgba(0, 0, 0, 0)");
        ctx.fillStyle = g;
        ctx.fillRect(x - r, y - r, r * 2, r * 2);
      }
      ctx.globalCompositeOperation = "source-over";
    }
    // Clicks
    for (const c2 of clicks) {
      const x = tf.x(c2.x);
      const y = tf.y(c2.y);
      const isSel = sel.has(c2.id);
      ctx.fillStyle = isSel ? "#ffffff" : BUTTON_COLORS[c2.button];
      ctx.globalAlpha = mode === "clicks" || isSel ? 0.95 : 0.7;
      ctx.beginPath();
      ctx.arc(x, y, isSel ? 4 : mode === "clicks" ? 3.2 : 2.4, 0, Math.PI * 2);
      ctx.fill();
      if (mode === "clicks") {
        ctx.strokeStyle = BUTTON_COLORS[c2.button];
        ctx.globalAlpha = 0.35;
        ctx.beginPath();
        ctx.arc(x, y, 7, 0, Math.PI * 2);
        ctx.stroke();
      }
    }
    ctx.globalAlpha = 1;
    // Selected moves
    for (const e of positional) {
      if (e.type !== "mouse_move" || !sel.has(e.id)) continue;
      ctx.fillStyle = "#ffffff";
      ctx.fillRect(tf.x(e.x) - 1.5, tf.y(e.y) - 1.5, 3, 3);
    }
  }, [doc.env.monitors, doc.selection, positional, mode, size, tf]);

  const pick = (e: React.MouseEvent) => {
    if (!tf) return;
    const r = ref.current!.getBoundingClientRect();
    const px = e.clientX - r.left;
    const py = e.clientY - r.top;
    let best: ChudEvent | null = null;
    let bd = 9;
    for (const ev of positional) {
      if (ev.type !== "mouse_down" && mode !== "path") continue;
      const p = ev as { x: number; y: number };
      const d = Math.hypot(tf.x(p.x) - px, tf.y(p.y) - py);
      const w = ev.type === "mouse_down" ? 0.6 : 1;
      if (d * w < bd) {
        bd = d * w;
        best = ev;
      }
    }
    if (best) patchDoc(doc.id, { selection: new Set([best.id]), cursor: best.t });
  };

  return (
    <div className="map-panel">
      <div className="row" style={{ justifyContent: "space-between", marginBottom: 8 }}>
        <Segmented<Mode>
          value={mode}
          onChange={setMode}
          options={[
            { value: "path", label: t("map.path") },
            { value: "heat", label: t("map.heat") },
            { value: "clicks", label: t("map.clicks") },
          ]}
        />
        {hover && (
          <span className="mono faint">
            {Math.round(hover.x)}, {Math.round(hover.y)}
          </span>
        )}
      </div>
      <div ref={wrap} className="map-wrap">
        {world ? (
          <canvas
            ref={ref}
            style={{ width: size.w, height: size.h }}
            onMouseMove={(e) => {
              if (!tf) return;
              const r = ref.current!.getBoundingClientRect();
              setHover({ x: tf.ix(e.clientX - r.left), y: tf.iy(e.clientY - r.top) });
            }}
            onMouseLeave={() => setHover(null)}
            onClick={pick}
          />
        ) : (
          <div className="empty faint">{t("map.no_data")}</div>
        )}
      </div>
      <p className="faint insp-hint">{t("map.hint")}</p>
    </div>
  );
}
