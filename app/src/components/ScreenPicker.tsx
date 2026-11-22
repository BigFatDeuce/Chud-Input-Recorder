// Screen picker: a frozen capture of every screen to cut an image out of,
// mark a search area on, or pick a point and its colour from. Chud hides
// while the capture is taken; the capture stays in memory and is dropped
// when the picker closes.
import { Crosshair, Image as ImageIcon, LoaderCircle, Monitor as MonitorIcon, Pipette, SquareDashed, TriangleAlert, X } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { t } from "../i18n";
import { api, errorText } from "../lib/api";
import type { Monitor, Region, SnapshotInfo } from "../lib/types";

export type PickMode = "image" | "area" | "point";

export type PickResult =
  | { mode: "image" | "area"; region: Region }
  | { mode: "point"; x: number; y: number; color: string };

interface View {
  s: number;
  tx: number;
  ty: number;
}

interface Sel {
  x: number;
  y: number;
  w: number;
  h: number;
}

const hex2 = (v: number) => v.toString(16).padStart(2, "0");
const MAX_ZOOM = 24;
const LOUPE_PX = 15;
const LOUPE_SIZE = 150;

function screensOf(info: SnapshotInfo): Monitor[] {
  if (info.monitors.length) return info.monitors;
  return [{ name: "screen", rect: { x: info.x, y: info.y, width: info.w, height: info.h }, scale: 1, primary: true, refresh_hz: null }];
}

function startScreen(info: SnapshotInfo, screens: Monitor[]): number {
  const c = info.cursor;
  if (c) {
    const i = screens.findIndex((m) => c[0] >= m.rect.x && c[0] < m.rect.x + m.rect.width && c[1] >= m.rect.y && c[1] < m.rect.y + m.rect.height);
    if (i >= 0) return i;
  }
  return Math.max(0, screens.findIndex((m) => m.primary));
}

export function ScreenPicker({
  mode,
  onPick,
  onClose,
}: {
  mode: PickMode;
  /** May be async (e.g. storing the image): the capture is kept until it ends. */
  onPick: (r: PickResult) => unknown;
  onClose: () => void;
}) {
  const [info, setInfo] = useState<SnapshotInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [screen, setScreen] = useState(0);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [view, setView] = useState<View>({ s: 1, tx: 0, ty: 0 });
  const [sel, setSel] = useState<Sel | null>(null);
  const [hover, setHover] = useState<{ ix: number; iy: number; cx: number; cy: number } | null>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const loupeRef = useRef<HTMLCanvasElement>(null);
  const pixels = useRef<ImageData | null>(null);
  const fitRef = useRef(1);
  const viewRef = useRef(view);
  viewRef.current = view;

  const screens = useMemo(() => (info ? screensOf(info) : []), [info]);
  const mon = screens[screen];
  const W = mon?.rect.width ?? 0;
  const H = mon?.rect.height ?? 0;

  // Capture once.
  useEffect(() => {
    let alive = true;
    api
      .screenSnapshot()
      .then((i) => {
        if (!alive) return;
        setInfo(i);
        setScreen(startScreen(i, screensOf(i)));
      })
      .catch((e) => alive && setError(errorText(e)));
    return () => {
      alive = false;
      api.screenSnapshotClear().catch(() => {});
    };
  }, []);

  const fit = useCallback(() => {
    const st = stageRef.current;
    if (!st || !W || !H) return;
    const r = st.getBoundingClientRect();
    const s = Math.min(r.width / W, r.height / H, 1);
    fitRef.current = s;
    setView({ s, tx: (r.width - W * s) / 2, ty: (r.height - H * s) / 2 });
  }, [W, H]);

  // Pixels of the chosen screen.
  useEffect(() => {
    if (!mon) return;
    let alive = true;
    setLoading(true);
    setSel(null);
    api
      .screenSnapshotPixels({ x: mon.rect.x, y: mon.rect.y, w: mon.rect.width, h: mon.rect.height })
      .then((img) => {
        if (!alive) return;
        pixels.current = img;
        const c = canvasRef.current;
        if (c) {
          c.width = img.width;
          c.height = img.height;
          c.getContext("2d")?.putImageData(img, 0, 0);
        }
        setLoading(false);
      })
      .catch((e) => alive && setError(errorText(e)));
    return () => {
      alive = false;
    };
  }, [mon]);

  useLayoutEffect(() => {
    fit();
  }, [fit, loading]);

  useEffect(() => {
    const r = () => fit();
    window.addEventListener("resize", r);
    return () => window.removeEventListener("resize", r);
  }, [fit]);

  const toImage = (clientX: number, clientY: number) => {
    const r = stageRef.current!.getBoundingClientRect();
    const v = viewRef.current;
    const cx = clientX - r.left;
    const cy = clientY - r.top;
    return { ix: Math.floor((cx - v.tx) / v.s), iy: Math.floor((cy - v.ty) / v.s), cx, cy };
  };

  const colorAt = (ix: number, iy: number): string | null => {
    const p = pixels.current;
    if (!p || ix < 0 || iy < 0 || ix >= p.width || iy >= p.height) return null;
    const i = (iy * p.width + ix) * 4;
    return `#${hex2(p.data[i])}${hex2(p.data[i + 1])}${hex2(p.data[i + 2])}`;
  };

  // Loupe.
  useEffect(() => {
    const l = loupeRef.current;
    const c = canvasRef.current;
    if (!l || !c || !hover || loading) return;
    const g = l.getContext("2d");
    if (!g) return;
    const half = Math.floor(LOUPE_PX / 2);
    const cell = LOUPE_SIZE / LOUPE_PX;
    g.imageSmoothingEnabled = false;
    g.fillStyle = "#000";
    g.fillRect(0, 0, LOUPE_SIZE, LOUPE_SIZE);
    g.drawImage(c, hover.ix - half, hover.iy - half, LOUPE_PX, LOUPE_PX, 0, 0, LOUPE_SIZE, LOUPE_SIZE);
    g.strokeStyle = "rgba(255,255,255,0.08)";
    g.lineWidth = 1;
    for (let i = 1; i < LOUPE_PX; i++) {
      g.beginPath();
      g.moveTo(i * cell + 0.5, 0);
      g.lineTo(i * cell + 0.5, LOUPE_SIZE);
      g.moveTo(0, i * cell + 0.5);
      g.lineTo(LOUPE_SIZE, i * cell + 0.5);
      g.stroke();
    }
    g.strokeStyle = "#f9c83a";
    g.lineWidth = 2;
    g.strokeRect(half * cell, half * cell, cell, cell);
  }, [hover, loading]);

  const finish = async (r: PickResult) => {
    setBusy(true);
    try {
      await onPick(r);
      onClose();
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };

  const confirm = () => {
    if (!mon || !sel || sel.w < 1 || sel.h < 1) return;
    void finish({ mode: mode === "image" ? "image" : "area", region: { x: mon.rect.x + sel.x, y: mon.rect.y + sel.y, w: sel.w, h: sel.h } });
  };

  // Mouse: left drag selects (or picks a point), right/middle drag or
  // Space + drag pans, the wheel zooms.
  const space = useRef(false);
  const onMouseDown = (e: React.MouseEvent) => {
    if (loading || busy) return;
    e.preventDefault();
    const start = toImage(e.clientX, e.clientY);
    const pan = e.button === 1 || e.button === 2 || (e.button === 0 && space.current);
    if (pan) {
      const v0 = viewRef.current;
      const mv = (ev: MouseEvent) => setView({ ...v0, tx: v0.tx + ev.clientX - e.clientX, ty: v0.ty + ev.clientY - e.clientY });
      const up = () => {
        window.removeEventListener("mousemove", mv);
        window.removeEventListener("mouseup", up);
      };
      window.addEventListener("mousemove", mv);
      window.addEventListener("mouseup", up);
      return;
    }
    if (e.button !== 0) return;
    const clamp = (v: number, max: number) => Math.max(0, Math.min(max, v));
    let moved = false;
    const mv = (ev: MouseEvent) => {
      const p = toImage(ev.clientX, ev.clientY);
      if (!moved && Math.abs(p.cx - start.cx) + Math.abs(p.cy - start.cy) < 4) return;
      moved = true;
      if (mode === "point") return;
      const x0 = clamp(Math.min(start.ix, p.ix), W - 1);
      const y0 = clamp(Math.min(start.iy, p.iy), H - 1);
      const x1 = clamp(Math.max(start.ix, p.ix) + 1, W);
      const y1 = clamp(Math.max(start.iy, p.iy) + 1, H);
      setSel({ x: x0, y: y0, w: Math.max(1, x1 - x0), h: Math.max(1, y1 - y0) });
    };
    const up = (ev: MouseEvent) => {
      window.removeEventListener("mousemove", mv);
      window.removeEventListener("mouseup", up);
      if (mode === "point" && !moved && mon) {
        const p = toImage(ev.clientX, ev.clientY);
        const color = colorAt(p.ix, p.iy);
        if (color) void finish({ mode: "point", x: mon.rect.x + p.ix, y: mon.rect.y + p.iy, color });
      }
    };
    window.addEventListener("mousemove", mv);
    window.addEventListener("mouseup", up);
  };

  const onWheel = (e: React.WheelEvent) => {
    if (loading) return;
    const { cx, cy } = toImage(e.clientX, e.clientY);
    const v = viewRef.current;
    const s = Math.max(fitRef.current, Math.min(MAX_ZOOM, v.s * Math.pow(1.2, -e.deltaY / 100)));
    const k = s / v.s;
    setView({ s, tx: cx - (cx - v.tx) * k, ty: cy - (cy - v.ty) * k });
  };

  // Keyboard: Esc, Enter, arrows nudge the selection (Shift: resize), 0 fits.
  useEffect(() => {
    const down = (e: KeyboardEvent) => {
      // The picker owns the keyboard while open.
      e.stopPropagation();
      if (e.key === " ") {
        space.current = true;
        e.preventDefault();
        return;
      }
      if (e.key === "Escape") {
        if (sel) setSel(null);
        else onClose();
        return;
      }
      if (e.key === "Enter") return confirm();
      if (e.key === "0") return fit();
      const d = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[e.key];
      if (!d || !sel) return;
      e.preventDefault();
      const n = e.ctrlKey ? 10 : 1;
      setSel((s) => {
        if (!s) return s;
        if (e.shiftKey) {
          const w = Math.max(1, Math.min(W - s.x, s.w + d[0] * n));
          const h = Math.max(1, Math.min(H - s.y, s.h + d[1] * n));
          return { ...s, w, h };
        }
        return { ...s, x: Math.max(0, Math.min(W - s.w, s.x + d[0] * n)), y: Math.max(0, Math.min(H - s.h, s.y + d[1] * n)) };
      });
    };
    const up = (e: KeyboardEvent) => {
      if (e.key === " ") space.current = false;
    };
    window.addEventListener("keydown", down, true);
    window.addEventListener("keyup", up, true);
    return () => {
      window.removeEventListener("keydown", down, true);
      window.removeEventListener("keyup", up, true);
    };
  });

  const ModeIcon = mode === "image" ? ImageIcon : mode === "area" ? SquareDashed : Pipette;
  const hoverColor = hover ? colorAt(hover.ix, hover.iy) : null;
  const inside = hover && hover.ix >= 0 && hover.iy >= 0 && hover.ix < W && hover.iy < H;
  const stageRect = stageRef.current?.getBoundingClientRect();
  const loupeLeft = hover && stageRect && hover.cx + 24 + LOUPE_SIZE > stageRect.width ? hover.cx - 24 - LOUPE_SIZE : (hover?.cx ?? 0) + 24;
  const loupeTop = hover && stageRect && hover.cy + 24 + LOUPE_SIZE + 44 > stageRect.height ? hover.cy - 24 - LOUPE_SIZE - 44 : (hover?.cy ?? 0) + 24;

  return createPortal(
    <div className="picker" role="dialog" aria-modal aria-label={t(`picker.title_${mode}`)}>
      <div className="picker-bar">
        <ModeIcon size={16} className="gold-icon" />
        <h2>{t(`picker.title_${mode}`)}</h2>
        <span className="faint picker-hint">{t(`picker.hint_${mode}`)}</span>
        <div className="grow" />
        {screens.length > 1 && (
          <div className="segmented picker-screens" role="radiogroup">
            {screens.map((m, i) => (
              <button
                key={i}
                type="button"
                role="radio"
                aria-checked={i === screen}
                className={i === screen ? "active" : ""}
                onClick={() => setScreen(i)}
                title={`${m.rect.width}×${m.rect.height} · ${Math.round(m.scale * 100)} %${m.primary ? ` · ${t("picker.primary")}` : ""}`}
              >
                <MonitorIcon size={12} /> {i + 1}
              </button>
            ))}
          </div>
        )}
        <span className="chip mono">{Math.round(view.s * 100)} %</span>
        <button className="btn ghost icon" onClick={onClose} aria-label={t("common.cancel")} title={`${t("common.cancel")} (Esc)`}>
          <X size={16} />
        </button>
      </div>

      <div
        ref={stageRef}
        className={`picker-stage ${mode === "point" ? "point" : ""}`}
        onMouseDown={onMouseDown}
        onWheel={onWheel}
        onContextMenu={(e) => e.preventDefault()}
        onMouseMove={(e) => !loading && setHover(toImage(e.clientX, e.clientY))}
        onMouseLeave={() => setHover(null)}
      >
        <canvas
          ref={canvasRef}
          className="picker-canvas"
          style={{ transform: `translate(${view.tx}px, ${view.ty}px) scale(${view.s})`, imageRendering: view.s >= 2 ? "pixelated" : "auto", visibility: loading ? "hidden" : "visible" }}
        />
        {sel && (
          <div className="picker-sel" style={{ left: view.tx + sel.x * view.s, top: view.ty + sel.y * view.s, width: sel.w * view.s, height: sel.h * view.s }}>
            <span className="picker-sel-size mono">
              {sel.w} × {sel.h}
            </span>
          </div>
        )}
        {(loading || error) && (
          <div className="picker-center">
            {error ? (
              <div className="picker-error">
                <TriangleAlert size={18} /> {error}
              </div>
            ) : (
              <>
                <LoaderCircle size={22} className="spin" /> {info ? t("picker.loading") : t("picker.capturing")}
              </>
            )}
          </div>
        )}
        {hover && inside && !loading && (
          <div className="picker-loupe" style={{ left: loupeLeft, top: loupeTop }}>
            <canvas ref={loupeRef} width={LOUPE_SIZE} height={LOUPE_SIZE} />
            <div className="picker-loupe-info mono">
              <span className="picker-swatch" style={{ background: hoverColor ?? "transparent" }} />
              {hoverColor}
              <span className="faint">
                {(mon?.rect.x ?? 0) + hover.ix}, {(mon?.rect.y ?? 0) + hover.iy}
              </span>
            </div>
          </div>
        )}
      </div>

      <div className="picker-foot">
        <span className="faint picker-keys">{t("picker.keys")}</span>
        <div className="grow" />
        {mode !== "point" && sel && (
          <span className="mono faint">
            <Crosshair size={12} /> {(mon?.rect.x ?? 0) + sel.x}, {(mon?.rect.y ?? 0) + sel.y} · {sel.w} × {sel.h}
          </span>
        )}
        {mode === "image" && sel && (sel.w < 4 || sel.h < 4) && <span className="chip rec">{t("picker.too_small")}</span>}
        <button className="btn" onClick={onClose}>
          {t("common.cancel")}
        </button>
        {mode !== "point" && (
          <button className="btn primary" disabled={!sel || busy || (mode === "image" && (sel.w < 4 || sel.h < 4))} onClick={confirm}>
            {busy ? <LoaderCircle size={14} className="spin" /> : <ModeIcon size={14} />} {t(`picker.use_${mode}`)}
          </button>
        )}
      </div>
    </div>,
    document.body,
  );
}
