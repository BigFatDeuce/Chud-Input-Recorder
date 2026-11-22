// Live controller diagram: every standard button and axis in place, plus
// chips for the extra buttons some controllers have (paddles, touchpad…).
import { PAD_BUTTONS } from "../lib/keys";

export interface PadLive {
  buttons: Record<number, boolean>;
  axes: Record<number, number>;
}

// Button indices (same order as Rust `PadButton`).
const B = { A: 0, B: 1, X: 2, Y: 3, BACK: 4, GUIDE: 5, START: 6, L3: 7, R3: 8, LB: 9, RB: 10, UP: 11, DOWN: 12, LEFT: 13, RIGHT: 14 };
const FIRST_EXTRA = 15;

const FACE: { i: number; label: string; x: number; y: number; c: string }[] = [
  { i: B.Y, label: "Y", x: 318, y: 82, c: "var(--gold-400)" },
  { i: B.B, label: "B", x: 344, y: 108, c: "var(--rec)" },
  { i: B.A, label: "A", x: 318, y: 134, c: "var(--mint)" },
  { i: B.X, label: "X", x: 292, y: 108, c: "var(--sky)" },
];

const BODY =
  "M128 44 H292 C338 44 366 62 384 104 L410 182 C422 222 398 250 368 238 C346 229 330 204 312 188 C300 178 288 174 270 174 H150 C132 174 120 178 108 188 C90 204 74 229 52 238 C22 250 -2 222 10 182 L36 104 C54 62 82 44 128 44 Z";

function Stick({ cx, cy, x, y, pressed, label }: { cx: number; cy: number; x: number; y: number; pressed: boolean; label: string }) {
  const r = 27;
  const dx = Math.max(-1, Math.min(1, x / 32767)) * (r - 11);
  const dy = Math.max(-1, Math.min(1, y / 32767)) * (r - 11);
  const moved = Math.abs(x) > 2500 || Math.abs(y) > 2500;
  return (
    <g className={`gp-stick ${pressed ? "on" : ""} ${moved ? "moved" : ""}`}>
      <circle cx={cx} cy={cy} r={r} className="gp-well" />
      <line x1={cx} y1={cy} x2={cx + dx} y2={cy + dy} className="gp-vector" />
      <circle cx={cx + dx} cy={cy + dy} r={11} className="gp-cap" />
      <text x={cx} y={cy + r + 13} className="gp-cap-label">
        {label}
      </text>
    </g>
  );
}

function Trigger({ x, v, label }: { x: number; v: number; label: string }) {
  const w = 86;
  const f = Math.max(0, Math.min(1, v / 32767));
  return (
    <g className={`gp-trigger ${f > 0.04 ? "on" : ""}`}>
      <rect x={x} y={4} width={w} height={11} rx={5.5} className="gp-trigger-track" />
      <rect x={x} y={4} width={Math.max(f * w, f > 0 ? 6 : 0)} height={11} rx={5.5} className="gp-trigger-fill" />
      <text x={x + (label === "LT" ? -8 : w + 8)} y={13} className={`gp-side-label ${label === "LT" ? "end" : ""}`}>
        {label}
      </text>
    </g>
  );
}

export function GamepadView({ pad }: { pad: PadLive }) {
  const on = (i: number) => Boolean(pad.buttons[i]);
  const ax = (i: number) => pad.axes[i] ?? 0;
  const extras = Object.keys(pad.buttons)
    .map(Number)
    .filter((i) => i >= FIRST_EXTRA)
    .sort((a, b) => a - b);
  const pct = (v: number, trigger = false) => `${Math.round((v / (trigger ? 32767 : 32768)) * 100)}%`;
  return (
    <div className="gp">
      <svg viewBox="-4 0 428 262" className="gp-svg" role="img" aria-label="Gamepad">
        <Trigger x={74} v={ax(4)} label="LT" />
        <Trigger x={260} v={ax(5)} label="RT" />
        <rect x={70} y={22} width={94} height={16} rx={8} className={`gp-btn ${on(B.LB) ? "on" : ""}`} />
        <text x={117} y={34} className="gp-label">
          LB
        </text>
        <rect x={256} y={22} width={94} height={16} rx={8} className={`gp-btn ${on(B.RB) ? "on" : ""}`} />
        <text x={303} y={34} className="gp-label">
          RB
        </text>

        <path d={BODY} className="gp-body" />

        <Stick cx={112} cy={108} x={ax(0)} y={ax(1)} pressed={on(B.L3)} label="L3" />
        <Stick cx={262} cy={160} x={ax(2)} y={ax(3)} pressed={on(B.R3)} label="R3" />

        {/* D-pad */}
        <g className="gp-dpad">
          <rect x={148} y={134} width={20} height={22} rx={4} className={`gp-btn ${on(B.UP) ? "on" : ""}`} />
          <rect x={148} y={176} width={20} height={22} rx={4} className={`gp-btn ${on(B.DOWN) ? "on" : ""}`} />
          <rect x={126} y={156} width={22} height={20} rx={4} className={`gp-btn ${on(B.LEFT) ? "on" : ""}`} />
          <rect x={168} y={156} width={22} height={20} rx={4} className={`gp-btn ${on(B.RIGHT) ? "on" : ""}`} />
          <rect x={148} y={156} width={20} height={20} className="gp-dpad-hub" />
          <path d="M158 140 l-4 6 h8 z M158 192 l-4 -6 h8 z M132 166 l6 -4 v8 z M184 166 l-6 -4 v8 z" className="gp-arrow" />
        </g>

        {/* Face buttons */}
        {FACE.map((f) => (
          <g key={f.i} className={`gp-face ${on(f.i) ? "on" : ""}`} style={{ "--c": f.c } as React.CSSProperties}>
            <circle cx={f.x} cy={f.y} r={12.5} />
            <text x={f.x} y={f.y + 4.5}>
              {f.label}
            </text>
          </g>
        ))}

        {/* Centre cluster */}
        <rect x={176} y={100} width={22} height={11} rx={5.5} className={`gp-btn ${on(B.BACK) ? "on" : ""}`} />
        <text x={187} y={124} className="gp-tiny">
          Back
        </text>
        <rect x={222} y={100} width={22} height={11} rx={5.5} className={`gp-btn ${on(B.START) ? "on" : ""}`} />
        <text x={233} y={124} className="gp-tiny">
          Start
        </text>
        <circle cx={210} cy={76} r={12} className={`gp-btn gp-guide ${on(B.GUIDE) ? "on" : ""}`} />
        <circle cx={210} cy={76} r={4} className="gp-guide-dot" />
      </svg>

      <div className="gp-readout mono">
        <span>
          L <b>{pct(ax(0))}</b> <b>{pct(ax(1))}</b>
        </span>
        <span>
          R <b>{pct(ax(2))}</b> <b>{pct(ax(3))}</b>
        </span>
        <span>
          LT <b>{pct(ax(4), true)}</b>
        </span>
        <span>
          RT <b>{pct(ax(5), true)}</b>
        </span>
      </div>

      {extras.length > 0 && (
        <div className="gp-extras">
          {extras.map((i) => (
            <span key={i} className={`pad-btn ${on(i) ? "on" : ""}`}>
              {PAD_BUTTONS[i] ?? `Btn ${i >= 64 ? i - 63 : i}`}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}
