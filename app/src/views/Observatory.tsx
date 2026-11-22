// Observatory: live view of every input (keyboard, mouse, raw motion, gamepads).
// Nothing is recorded or stored; it only listens while this view is open.
import { Eye, EyeOff, Pause, Play, Radio, RotateCcw } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Toggle } from "../components/ui";
import { fmtInt, fmtNum, t, useLang } from "../i18n";
import { api } from "../lib/api";
import { describe } from "../lib/events";
import { BUTTON_COLORS, BUTTON_LABELS, GROUP_COLORS, keyGroup, keyInfo, keyLabel } from "../lib/keys";
import { GamepadView, type PadLive } from "../components/GamepadView";
import { liveBus } from "../lib/live";
import { useStore } from "../lib/store";
import type { Key, LiveInput, MouseButton } from "../lib/types";
import { MOUSE_BUTTONS } from "../lib/types";

// Keyboard layout by scan code (set 1, 0xE0 prefix as 0xE0xx); widths in key units.
type Cap = [number, number?] | null; // [scan, width] or a gap
const MAIN: Cap[][] = [
  [[0x01], null, [0x3b], [0x3c], [0x3d], [0x3e], null, [0x3f], [0x40], [0x41], [0x42], null, [0x43], [0x44], [0x57], [0x58]],
  [[0x29], [0x02], [0x03], [0x04], [0x05], [0x06], [0x07], [0x08], [0x09], [0x0a], [0x0b], [0x0c], [0x0d], [0x0e, 2]],
  [[0x0f, 1.5], [0x10], [0x11], [0x12], [0x13], [0x14], [0x15], [0x16], [0x17], [0x18], [0x19], [0x1a], [0x1b], [0x2b, 1.5]],
  [[0x3a, 1.75], [0x1e], [0x1f], [0x20], [0x21], [0x22], [0x23], [0x24], [0x25], [0x26], [0x27], [0x28], [0x1c, 2.25]],
  [[0x2a, 1.25], [0x56], [0x2c], [0x2d], [0x2e], [0x2f], [0x30], [0x31], [0x32], [0x33], [0x34], [0x35], [0x36, 2.75]],
  [[0x1d, 1.25], [0xe05b, 1.25], [0x38, 1.25], [0x39, 6.25], [0xe038, 1.25], [0xe05c, 1.25], [0xe05d, 1.25], [0xe01d, 1.25]],
];
const NAV: Cap[][] = [
  [[0xe037], [0x46], [0x45]],
  [[0xe052], [0xe047], [0xe049]],
  [[0xe053], [0xe04f], [0xe051]],
  [null, null, null],
  [null, [0xe048], null],
  [[0xe04b], [0xe050], [0xe04d]],
];
const PAD: Cap[][] = [
  [null, null, null, null],
  [[0xe045], [0xe035], [0x37], [0x4a]],
  [[0x47], [0x48], [0x49], [0x4e]],
  [[0x4b], [0x4c], [0x4d], null],
  [[0x4f], [0x50], [0x51], [0xe01c]],
  [[0x52, 2], [0x53], null],
];

const FALLBACK: Record<number, string> = {
  0x01: "Esc", 0x0e: "⌫", 0x0f: "Tab", 0x3a: "Caps", 0x1c: "Enter", 0x2a: "Shift", 0x36: "Shift", 0x1d: "Ctrl", 0xe01d: "Ctrl",
  0x38: "Alt", 0xe038: "AltGr", 0x39: "", 0xe05b: "Win", 0xe05c: "Win", 0xe05d: "Menu", 0xe037: "PrtSc", 0x46: "ScrLk",
  0x45: "Pause", 0xe045: "Num", 0xe052: "Ins", 0xe047: "Home", 0xe049: "PgUp", 0xe053: "Del", 0xe04f: "End", 0xe051: "PgDn",
  0xe048: "↑", 0xe050: "↓", 0xe04b: "←", 0xe04d: "→", 0xe035: "/", 0x37: "*", 0x4a: "−", 0x4e: "+", 0xe01c: "⏎",
};

interface KeyState {
  down: boolean;
  count: number;
  last: number;
  vk: number;
}

const MAX_LOG = 60;

export function ObservatoryView() {
  useLang();
  const info = useStore((s) => s.info);
  const fg = useStore((s) => s.foreground);
  const [paused, setPaused] = useState(false);
  const [mask, setMask] = useState(true);
  const [keys, setKeys] = useState<Record<number, KeyState>>({});
  const [buttons, setButtons] = useState<Record<string, boolean>>({});
  // One state per controller; the view follows the last one used unless
  // the user picks one.
  const [pads, setPads] = useState<Record<number, PadLive>>({});
  const [padPick, setPadPick] = useState<number | null>(null);
  const [lastPad, setLastPad] = useState(0);
  const [log, setLog] = useState<LiveInput[]>([]);
  const [counters, setCounters] = useState({ keys: 0, clicks: 0, wheel: 0, distance: 0, rate: 0, injected: 0 });
  const padIds = Object.keys(pads)
    .map(Number)
    .sort((a, b) => a - b);
  const shownPad = padPick !== null && pads[padPick] ? padPick : pads[lastPad] ? lastPad : (padIds[0] ?? null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const scope = useRef<HTMLCanvasElement>(null);
  const trail = useRef<{ x: number; y: number; at: number }[]>([]);
  const ripples = useRef<{ x: number; y: number; at: number; color: string }[]>([]);
  const rawVec = useRef<{ dx: number; dy: number; at: number }[]>([]);
  const pending = useRef<LiveInput[]>([]);
  const rateWin = useRef<number[]>([]);
  const lastPos = useRef<{ x: number; y: number } | null>(null);
  const pausedRef = useRef(paused);
  pausedRef.current = paused;

  // Live capture while this view is open.
  useEffect(() => {
    api.liveSet(!paused).catch(() => {});
    return () => {
      api.liveSet(false).catch(() => {});
    };
  }, [paused]);

  useEffect(() => {
    const off = liveBus.on((batch) => {
      if (pausedRef.current) return;
      const now = performance.now();
      const keyUpd: Record<number, Partial<KeyState>> = {};
      const btnUpd: Record<string, boolean> = {};
      const padUpd: Record<number, PadLive> = {};
      let touchedPad: number | null = null;
      const padOf = (i: number) => (padUpd[i] ??= { buttons: {}, axes: {} });
      let dKeys = 0;
      let dClicks = 0;
      let dWheel = 0;
      let dDist = 0;
      let dInj = 0;
      for (const e of batch) {
        if (e.injected) dInj++;
        rateWin.current.push(now);
        switch (e.type) {
          case "key_down":
            keyUpd[e.key.scan] = { down: true, vk: e.key.vk, last: now };
            if (!e.repeat) dKeys++;
            break;
          case "key_up":
            keyUpd[e.key.scan] = { ...(keyUpd[e.key.scan] ?? {}), down: false, vk: e.key.vk };
            break;
          case "mouse_down":
            btnUpd[e.button] = true;
            dClicks++;
            ripples.current.push({ x: e.x, y: e.y, at: now, color: BUTTON_COLORS[e.button] });
            break;
          case "mouse_up":
            btnUpd[e.button] = false;
            break;
          case "mouse_move": {
            const lp = lastPos.current;
            if (lp) dDist += Math.hypot(e.x - lp.x, e.y - lp.y);
            lastPos.current = { x: e.x, y: e.y };
            trail.current.push({ x: e.x, y: e.y, at: now });
            break;
          }
          case "mouse_raw":
            rawVec.current.push({ dx: e.dx, dy: e.dy, at: now });
            if (!info?.capabilities.absolute_pointer_capture) dDist += Math.hypot(e.dx, e.dy);
            break;
          case "wheel":
            dWheel += Math.abs(e.delta) / 120;
            break;
          case "pad_button":
            padOf(e.pad).buttons[e.button] = e.down;
            touchedPad = e.pad;
            break;
          case "pad_axis":
            padOf(e.pad).axes[e.axis] = e.value;
            touchedPad = e.pad;
            break;
        }
        if (e.type !== "mouse_move" && e.type !== "mouse_raw" && e.type !== "pad_axis") pending.current.push(e);
      }
      if (Object.keys(keyUpd).length)
        setKeys((ks) => {
          const next = { ...ks };
          for (const [s, u] of Object.entries(keyUpd)) {
            const cur = next[+s] ?? { down: false, count: 0, last: 0, vk: 0 };
            next[+s] = { ...cur, ...u, count: cur.count + (u.down && !cur.down ? 1 : 0) };
          }
          return next;
        });
      if (Object.keys(btnUpd).length) setButtons((b) => ({ ...b, ...btnUpd }));
      if (touchedPad !== null) {
        setPads((ps) => {
          const next = { ...ps };
          for (const [i, u] of Object.entries(padUpd)) {
            const cur = next[+i] ?? { buttons: {}, axes: {} };
            next[+i] = { buttons: { ...cur.buttons, ...u.buttons }, axes: { ...cur.axes, ...u.axes } };
          }
          return next;
        });
        setLastPad(touchedPad);
      }
      if (dKeys || dClicks || dWheel || dDist || dInj)
        setCounters((c) => ({
          ...c,
          keys: c.keys + dKeys,
          clicks: c.clicks + dClicks,
          wheel: c.wheel + dWheel,
          distance: c.distance + dDist,
          injected: c.injected + dInj,
        }));
    });
    // Flush the log and rate at a calm pace.
    const iv = window.setInterval(() => {
      const now = performance.now();
      rateWin.current = rateWin.current.filter((x) => now - x < 1000);
      setCounters((c) => (c.rate === rateWin.current.length ? c : { ...c, rate: rateWin.current.length }));
      if (pending.current.length) {
        const add = pending.current.splice(0);
        setLog((l) => [...add.reverse(), ...l].slice(0, MAX_LOG));
      }
    }, 120);
    return () => {
      off();
      window.clearInterval(iv);
    };
  }, [info]);

  // Pointer canvas and raw-motion scope.
  const monitors = info?.monitors ?? [];
  const world = useMemo(() => {
    if (!monitors.length) return { minX: 0, minY: 0, maxX: 1920, maxY: 1080 };
    return {
      minX: Math.min(...monitors.map((m) => m.rect.x)),
      minY: Math.min(...monitors.map((m) => m.rect.y)),
      maxX: Math.max(...monitors.map((m) => m.rect.x + m.rect.width)),
      maxY: Math.max(...monitors.map((m) => m.rect.y + m.rect.height)),
    };
  }, [monitors]);

  useEffect(() => {
    let raf = 0;
    const draw = () => {
      raf = requestAnimationFrame(draw);
      const now = performance.now();
      const c = canvas.current;
      if (c) {
        const w = c.clientWidth;
        const h = c.clientHeight;
        const dpr = Math.min(window.devicePixelRatio || 1, 2);
        if (c.width !== Math.round(w * dpr)) {
          c.width = Math.round(w * dpr);
          c.height = Math.round(h * dpr);
        }
        const ctx = c.getContext("2d")!;
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        ctx.clearRect(0, 0, w, h);
        const s = Math.min((w - 20) / (world.maxX - world.minX), (h - 20) / (world.maxY - world.minY));
        const ox = (w - (world.maxX - world.minX) * s) / 2;
        const oy = (h - (world.maxY - world.minY) * s) / 2;
        const X = (x: number) => ox + (x - world.minX) * s;
        const Y = (y: number) => oy + (y - world.minY) * s;
        for (const m of monitors) {
          ctx.fillStyle = "rgba(10, 39, 80, 0.5)";
          ctx.fillRect(X(m.rect.x), Y(m.rect.y), m.rect.width * s, m.rect.height * s);
          ctx.strokeStyle = m.primary ? "rgba(249, 200, 58, 0.5)" : "rgba(109, 140, 171, 0.4)";
          ctx.strokeRect(X(m.rect.x) + 0.5, Y(m.rect.y) + 0.5, m.rect.width * s - 1, m.rect.height * s - 1);
          ctx.fillStyle = "rgba(169, 189, 212, 0.55)";
          ctx.font = "10px 'JetBrains Mono Variable', monospace";
          ctx.fillText(m.name.replace(/^\\\\\.\\/, ""), X(m.rect.x) + 6, Y(m.rect.y) + 14);
        }
        trail.current = trail.current.filter((p) => now - p.at < 1600);
        const tr = trail.current;
        for (let i = 1; i < tr.length; i++) {
          const a = 1 - (now - tr[i].at) / 1600;
          ctx.strokeStyle = `rgba(56, 214, 255, ${a * 0.8})`;
          ctx.lineWidth = 1 + a * 1.5;
          ctx.beginPath();
          ctx.moveTo(X(tr[i - 1].x), Y(tr[i - 1].y));
          ctx.lineTo(X(tr[i].x), Y(tr[i].y));
          ctx.stroke();
        }
        ripples.current = ripples.current.filter((r) => now - r.at < 700);
        for (const r of ripples.current) {
          const f = (now - r.at) / 700;
          ctx.strokeStyle = r.color;
          ctx.globalAlpha = 1 - f;
          ctx.lineWidth = 2;
          ctx.beginPath();
          ctx.arc(X(r.x), Y(r.y), 4 + f * 22, 0, Math.PI * 2);
          ctx.stroke();
          ctx.globalAlpha = 1;
        }
        const lp = lastPos.current;
        if (lp) {
          ctx.fillStyle = "#ffe486";
          ctx.shadowColor = "#f9c83a";
          ctx.shadowBlur = 12;
          ctx.beginPath();
          ctx.arc(X(lp.x), Y(lp.y), 4, 0, Math.PI * 2);
          ctx.fill();
          ctx.shadowBlur = 0;
        }
      }
      const sc = scope.current;
      if (sc) {
        const w = sc.clientWidth;
        const h = sc.clientHeight;
        const dpr = Math.min(window.devicePixelRatio || 1, 2);
        if (sc.width !== Math.round(w * dpr)) {
          sc.width = Math.round(w * dpr);
          sc.height = Math.round(h * dpr);
        }
        const ctx = sc.getContext("2d")!;
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        ctx.clearRect(0, 0, w, h);
        const cx = w / 2;
        const cy = h / 2;
        const R = Math.min(w, h) / 2 - 6;
        ctx.strokeStyle = "rgba(109, 140, 171, 0.25)";
        for (const f of [0.33, 0.66, 1]) {
          ctx.beginPath();
          ctx.arc(cx, cy, R * f, 0, Math.PI * 2);
          ctx.stroke();
        }
        ctx.beginPath();
        ctx.moveTo(cx - R, cy);
        ctx.lineTo(cx + R, cy);
        ctx.moveTo(cx, cy - R);
        ctx.lineTo(cx, cy + R);
        ctx.stroke();
        rawVec.current = rawVec.current.filter((v) => now - v.at < 500);
        for (const v of rawVec.current) {
          const a = 1 - (now - v.at) / 500;
          const m = Math.hypot(v.dx, v.dy) || 1;
          const len = Math.min(1, Math.log1p(m) / Math.log1p(80)) * R;
          ctx.strokeStyle = `rgba(169, 155, 255, ${a})`;
          ctx.lineWidth = 2;
          ctx.beginPath();
          ctx.moveTo(cx, cy);
          ctx.lineTo(cx + (v.dx / m) * len, cy + (v.dy / m) * len);
          ctx.stroke();
        }
      }
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [world, monitors]);

  const reset = () => {
    setKeys((ks) => Object.fromEntries(Object.entries(ks).map(([k, v]) => [k, { ...v, count: 0 }])));
    setCounters({ keys: 0, clicks: 0, wheel: 0, distance: 0, rate: 0, injected: 0 });
    setLog([]);
  };

  const maxCount = Math.max(1, ...Object.values(keys).map((k) => k.count));

  const renderCaps = (rows: Cap[][], cls: string) => (
    <div className={`kb-block ${cls}`}>
      {rows.map((row, ri) => (
        <div key={ri} className="kb-row">
          {row.map((cap, ci) => {
            if (!cap) return <span key={ci} className="kb-gap" />;
            const [scan, w = 1] = cap;
            const st = keys[scan];
            const k: Key = { scan, vk: st?.vk ?? keyInfo({ scan, vk: 0 })?.vk ?? 0 };
            const label = FALLBACK[scan] ?? keyLabel(k);
            const heat = st ? st.count / maxCount : 0;
            const color = GROUP_COLORS[keyGroup(k)] ?? "#f9c83a";
            return (
              <span
                key={ci}
                className={`kb-key ${st?.down ? "down" : ""}`}
                style={{
                  flexGrow: w,
                  ["--key-c" as string]: color,
                  ["--heat" as string]: heat.toFixed(3),
                }}
                title={st ? `${label} · ${fmtInt(st.count)}` : label}
              >
                {label}
              </span>
            );
          })}
        </div>
      ))}
    </div>
  );

  const heldButtons = MOUSE_BUTTONS.filter((b) => buttons[b]);
  return (
    <div className="observatory">
      <div className="obs-bar">
        <div className="row" style={{ gap: 10 }}>
          <span className={`live-dot ${paused ? "" : "on"}`} />
          <h1>{t("obs.title")}</h1>
          <span className="faint">{paused ? t("obs.paused") : t("obs.listening")}</span>
        </div>
        <div className="spacer" />
        <div className="obs-counters">
          <Counter label={t("obs.rate")} value={`${fmtInt(counters.rate)}/s`} />
          <Counter label={t("stats.key_presses")} value={fmtInt(counters.keys)} />
          <Counter label={t("stats.clicks")} value={fmtInt(counters.clicks)} />
          <Counter label={t("stats.wheel")} value={fmtNum(counters.wheel, 0)} />
          <Counter label={t("stats.distance")} value={`${fmtInt(Math.round(counters.distance))} px`} />
        </div>
        <button className="btn sm" onClick={reset} title={t("obs.reset")}>
          <RotateCcw size={13} />
        </button>
        <button className={`btn sm ${paused ? "primary" : ""}`} onClick={() => setPaused(!paused)}>
          {paused ? <Play size={13} /> : <Pause size={13} />} {paused ? t("obs.resume") : t("transport.pause")}
        </button>
      </div>

      <div className="obs-grid">
        <section className="panel obs-keyboard">
          <div className="panel-head">
            <h3>{t("lane.keyboard")}</h3>
            <span className="faint">{t("obs.heat_hint")}</span>
          </div>
          <div className="kb">
            {renderCaps(MAIN, "main")}
            {renderCaps(NAV, "nav")}
            {renderCaps(PAD, "numpad")}
          </div>
        </section>

        <section className="panel obs-mouse">
          <div className="panel-head">
            <h3>{t("obs.pointer")}</h3>
            <div className="row" style={{ gap: 6 }}>
              {MOUSE_BUTTONS.map((b: MouseButton) => (
                <span key={b} className={`btn-ind ${buttons[b] ? "on" : ""}`} style={{ ["--c" as string]: BUTTON_COLORS[b] }}>
                  {BUTTON_LABELS[b]}
                </span>
              ))}
            </div>
          </div>
          <canvas ref={canvas} className="obs-canvas" />
          <div className="obs-foot faint mono">
            {lastPos.current ? `${lastPos.current.x}, ${lastPos.current.y}` : t("obs.move_mouse")}
            {heldButtons.length > 0 && ` · ${heldButtons.map((b) => BUTTON_LABELS[b]).join("+")}`}
          </div>
        </section>

        <section className="panel obs-raw">
          <div className="panel-head">
            <h3>{t("lane.raw")}</h3>
          </div>
          <canvas ref={scope} className="obs-scope" />
          <p className="faint obs-note">{t("obs.raw_hint")}</p>
        </section>

        <section className="panel obs-pad">
          <div className="panel-head">
            <h3>{t("lane.gamepad")}</h3>
            {padIds.length > 1 && (
            <div className="gp-tabs">
              {padIds.map((i) => (
                <button key={i} className={`chip ${i === shownPad ? "gold" : ""}`} onClick={() => setPadPick(i === padPick ? null : i)}>
                  #{i + 1}
                </button>
              ))}
            </div>
            )}
          </div>
          {shownPad !== null ? (
            <GamepadView pad={pads[shownPad]} />
          ) : (
            <div className="empty faint">{t("obs.no_pad")}</div>
          )}
        </section>

        <section className="panel obs-log">
          <div className="panel-head">
            <h3>{t("obs.stream")}</h3>
            <label className="row faint" style={{ gap: 6 }}>
              {mask ? <EyeOff size={13} /> : <Eye size={13} />}
              {t("obs.mask")}
              <Toggle value={mask} onChange={setMask} />
            </label>
          </div>
          <div className="log-list">
            {log.length === 0 && <div className="faint" style={{ padding: 12 }}>{t("obs.waiting")}</div>}
            {log.map((e, i) => (
              <div key={i} className={`log-row ${e.injected ? "injected" : ""}`}>
                <span className="log-kind">{e.type.replace("_", " ")}</span>
                <span className="ellipsis">{mask && (e.type === "key_down" || e.type === "key_up") && isTextKey(e.key) ? "•" : describe(e)}</span>
                {e.injected && <span className="chip cyan">{t("obs.injected")}</span>}
              </div>
            ))}
          </div>
          {fg && (
            <div className="obs-foot faint">
              <Radio size={11} /> {fg.process} — {fg.title}
            </div>
          )}
        </section>
      </div>
      <p className="faint obs-privacy">{t("obs.privacy")}</p>
    </div>
  );
}

function isTextKey(k: Key): boolean {
  const g = keyGroup(k);
  return g === "letter" || g === "digit" || g === "symbol" || g === "numpad";
}

function Counter({ label, value }: { label: string; value: string }) {
  return (
    <div className="counter">
      <b className="mono">{value}</b>
      <span>{label}</span>
    </div>
  );
}

