// Statistics of the open macro (computed by the core) plus an activity chart.
import { useEffect, useMemo, useState } from "react";
import { fmtInt, fmtNum, t, useLang } from "../i18n";
import { api } from "../lib/api";
import { category } from "../lib/events";
import { fmtDuration } from "../lib/format";
import { BUTTON_COLORS, keyColor, keyLabel } from "../lib/keys";
import type { DocState } from "../lib/store";
import type { Stats } from "../lib/types";

const BUCKETS = 72;
const SERIES = [
  { cat: "keyboard", color: "#f9c83a", label: "lane.keyboard" },
  { cat: "mouse_button", color: "#38d6ff", label: "lane.buttons" },
  { cat: "wheel", color: "#8fe8c2", label: "lane.wheel" },
  { cat: "mouse_move", color: "#82b6ff", label: "lane.moves" },
  { cat: "pad", color: "#ff86b0", label: "lane.gamepad" },
] as const;

export function StatsPanel({ doc }: { doc: DocState }) {
  useLang();
  const [stats, setStats] = useState<Stats | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const id = window.setTimeout(() => {
      api
        .docStats(doc.id)
        .then((s) => {
          setStats(s);
          setError(null);
        })
        .catch((e) => setError(String(e)));
    }, 150);
    return () => window.clearTimeout(id);
  }, [doc.id, doc.version]);

  const hist = useMemo(() => {
    const ev = doc.events;
    const dur = ev.length ? ev[ev.length - 1].t : 0;
    const rows = SERIES.map(() => new Array<number>(BUCKETS).fill(0));
    if (!dur) return { rows, max: 1 };
    for (const e of ev) {
      const b = Math.min(BUCKETS - 1, Math.floor((e.t / dur) * BUCKETS));
      const c = category(e);
      const si =
        c === "keyboard" ? 0 : c === "mouse_button" ? 1 : c === "wheel" ? 2 : c === "mouse_move" || c === "mouse_raw" ? 3 : 4;
      // Moves are numerous: weight them down so other inputs stay visible.
      rows[si][b] += si === 3 ? 0.08 : 1;
    }
    let max = 1;
    for (let b = 0; b < BUCKETS; b++) {
      let s = 0;
      for (const r of rows) s += r[b];
      if (s > max) max = s;
    }
    return { rows, max };
  }, [doc.events]);

  if (error) return <div className="empty faint">{error}</div>;
  if (!stats) return <div className="empty"><div className="spinner" /></div>;

  const topMax = Math.max(1, ...stats.top_keys.map(([, n]) => n));
  const btnMax = Math.max(1, ...stats.clicks_by_button.map(([, n]) => n));

  return (
    <div className="stats-panel">
      <div className="stat-grid big">
        <Big label={t("stats.duration")} value={fmtDuration(stats.duration_us)} />
        <Big label={t("stats.events")} value={fmtInt(stats.events)} />
        <Big label={t("stats.key_presses")} value={fmtInt(stats.key_presses)} />
        <Big label={t("stats.clicks")} value={fmtInt(stats.clicks)} />
        <Big label={t("stats.apm")} value={fmtNum(stats.apm, 0)} accent />
        <Big label={t("stats.wheel")} value={fmtInt(stats.wheel_notches)} />
        <Big label={t("stats.distance")} value={`${fmtInt(Math.round(stats.mouse_distance_px))} px`} />
        <Big label={t("stats.mouse_rate")} value={`${fmtNum(stats.mouse_rate_hz, 0)} Hz`} />
        {stats.pad_presses > 0 && <Big label={t("stats.pad")} value={fmtInt(stats.pad_presses)} />}
      </div>

      <div className="insp-section">
        <div className="insp-head">
          <span className="eyebrow grow">{t("stats.activity")}</span>
        </div>
        <svg className="activity-chart" viewBox={`0 0 ${BUCKETS * 4} 60`} preserveAspectRatio="none">
          {Array.from({ length: BUCKETS }, (_, b) => {
            let y = 60;
            return SERIES.map((s, si) => {
              const v = hist.rows[si][b];
              if (!v) return null;
              const h = (v / hist.max) * 56;
              y -= h;
              return <rect key={`${b}-${si}`} x={b * 4 + 0.5} y={y} width={3} height={h} fill={s.color} opacity={0.85} rx={0.6} />;
            });
          })}
        </svg>
        <div className="legend">
          {SERIES.map((s) => (
            <span key={s.cat}>
              <i style={{ background: s.color }} /> {t(s.label)}
            </span>
          ))}
        </div>
      </div>

      <div className="insp-section">
        <div className="insp-head">
          <span className="eyebrow grow">{t("stats.rhythm")}</span>
        </div>
        <div className="kv">
          <span>{t("stats.longest_pause")}</span>
          <b className="mono">{fmtDuration(stats.longest_pause_us)}</b>
          <span>{t("stats.avg_hold")}</span>
          <b className="mono">{fmtDuration(stats.avg_hold_us)}</b>
          <span>{t("stats.max_hold")}</span>
          <b className="mono">{fmtDuration(stats.max_hold_us)}</b>
        </div>
      </div>

      {stats.top_keys.length > 0 && (
        <div className="insp-section">
          <div className="insp-head">
            <span className="eyebrow grow">{t("stats.top_keys")}</span>
          </div>
          <div className="bars">
            {stats.top_keys.map(([k, n]) => (
              <div key={`${k.scan}:${k.vk}`} className="bar-row">
                <kbd>{keyLabel(k)}</kbd>
                <div className="bar">
                  <span style={{ width: `${(n / topMax) * 100}%`, background: keyColor(k) }} />
                </div>
                <span className="mono faint">{fmtInt(n)}</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {stats.clicks_by_button.length > 0 && (
        <div className="insp-section">
          <div className="insp-head">
            <span className="eyebrow grow">{t("stats.clicks_by_button")}</span>
          </div>
          <div className="bars">
            {stats.clicks_by_button.map(([b, n]) => (
              <div key={b} className="bar-row">
                <span className="bar-label">{t(`button.${b}`)}</span>
                <div className="bar">
                  <span style={{ width: `${(n / btnMax) * 100}%`, background: BUTTON_COLORS[b] }} />
                </div>
                <span className="mono faint">{fmtInt(n)}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function Big({ label, value, accent }: { label: string; value: string; accent?: boolean }) {
  return (
    <div className={`stat ${accent ? "accent" : ""}`}>
      <div className="stat-value mono">{value}</div>
      <div className="stat-label">{label}</div>
    </div>
  );
}
