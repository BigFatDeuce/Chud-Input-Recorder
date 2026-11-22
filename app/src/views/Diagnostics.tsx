// Diagnostics: permission checks with fixes, capabilities, monitors, timer precision.
import { Activity, CircleCheck, CircleX, Copy, FolderOpen, Gauge, Monitor, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { fmtNum, t, tOr, useLang, type TKey } from "../i18n";
import { api } from "../lib/api";
import { useStore } from "../lib/store";
import type { Capabilities, Check, JitterStats, TimerReport } from "../lib/types";

const CAPS: { k: keyof Capabilities; label: TKey; hint: TKey }[] = [
  { k: "absolute_pointer_capture", label: "diag.cap_absolute", hint: "diag.cap_absolute_hint" },
  { k: "raw_mouse_capture", label: "diag.cap_raw", hint: "diag.cap_raw_hint" },
  { k: "hotkey_swallow", label: "diag.cap_swallow", hint: "diag.cap_swallow_hint" },
  { k: "foreground_window", label: "diag.cap_foreground", hint: "diag.cap_foreground_hint" },
  { k: "pixel_read", label: "diag.cap_pixel", hint: "diag.cap_pixel_hint" },
  { k: "gamepad_output", label: "diag.cap_gamepad", hint: "diag.cap_gamepad_hint" },
];

export function DiagnosticsView() {
  useLang();
  const info = useStore((s) => s.info);
  const toast = useStore((s) => s.toast);
  const fail = useStore((s) => s.fail);
  const [checks, setChecks] = useState<Check[] | null>(null);
  const [timer, setTimer] = useState<TimerReport | null>(null);
  const [running, setRunning] = useState(false);

  const loadChecks = useCallback(() => {
    setChecks(null);
    api
      .checks()
      .then(setChecks)
      .catch((e) => {
        fail(e);
        setChecks([]);
      });
  }, [fail]);
  useEffect(loadChecks, [loadChecks]);

  const runTimer = async () => {
    setRunning(true);
    try {
      setTimer(await api.diagTimer());
    } catch (e) {
      fail(e);
    } finally {
      setRunning(false);
    }
  };

  if (!info) return null;
  const caps = info.capabilities;
  const okCount = checks?.filter((c) => c.ok).length ?? 0;

  return (
    <div className="diagnostics">
      <div className="diag-head">
        <h1>{t("nav.diagnostics")}</h1>
        <span className="chip">
          {info.os} · {info.arch} · {caps.session}
        </span>
        <span className="chip gold">v{info.version}</span>
        <div className="spacer" />
        <button className="btn sm" onClick={loadChecks}>
          <RefreshCw size={13} /> {t("diag.recheck")}
        </button>
      </div>

      <div className="diag-grid">
        <section className="panel diag-checks">
          <div className="panel-head">
            <h3>{t("diag.checks")}</h3>
            {checks && (
              <span className={`chip ${okCount === checks.length ? "ok" : "rec"}`}>
                {okCount}/{checks.length}
              </span>
            )}
          </div>
          <div className="panel-body">
            {!checks ? (
              <div className="spinner" />
            ) : (
              checks.map((c) => (
                <div key={c.id} className={`check ${c.ok ? "ok" : "bad"}`}>
                  {c.ok ? <CircleCheck size={17} /> : <CircleX size={17} />}
                  <div className="grow">
                    <div className="check-title">{tOr(`diag.chk_${c.code}`, c.title, Object.fromEntries(c.vars))}</div>
                    <div className="muted">{tOr(`diag.chk_${c.code}_detail`, c.detail, Object.fromEntries(c.vars))}</div>
                    {!c.ok && c.fix && (
                      <div className="fix">
                        <code>{c.fix}</code>
                        <button
                          className="btn ghost sm icon"
                          onClick={() => navigator.clipboard.writeText(c.fix!).then(() => toast("info", t("toast.copied_text")))}
                          title={t("common.copy")}
                        >
                          <Copy size={12} />
                        </button>
                      </div>
                    )}
                  </div>
                </div>
              ))
            )}
          </div>
        </section>

        <section className="panel diag-caps">
          <div className="panel-head">
            <h3>{t("diag.capabilities")}</h3>
          </div>
          <div className="panel-body">
            {CAPS.map((c) => (
              <div key={c.k} className="cap-row">
                <span className={`cap-dot ${caps[c.k] ? "on" : ""}`} />
                <div className="grow">
                  <div>{t(c.label)}</div>
                  <div className="faint">{t(c.hint)}</div>
                </div>
                <span className={`chip ${caps[c.k] ? "ok" : ""}`}>{caps[c.k] ? t("diag.yes") : t("diag.no")}</span>
              </div>
            ))}
          </div>
        </section>

        <section className="panel diag-timer">
          <div className="panel-head">
            <h3>
              <Gauge size={14} /> {t("diag.timer")}
            </h3>
            <button className="btn sm" onClick={runTimer} disabled={running}>
              {running ? <span className="spinner" /> : <Activity size={13} />} {t("diag.run_timer")}
            </button>
          </div>
          <div className="panel-body">
            <p className="muted" style={{ marginBottom: 12 }}>
              {t("diag.timer_body")}
            </p>
            {timer && (
              <div className="timer-results">
                <Jitter title={t("diag.timer_fine")} s={timer.fine} />
                <Jitter title={t("diag.timer_coarse")} s={timer.coarse} />
              </div>
            )}
          </div>
        </section>

        <section className="panel diag-monitors">
          <div className="panel-head">
            <h3>
              <Monitor size={14} /> {t("diag.monitors")}
            </h3>
          </div>
          <div className="panel-body">
            <table className="table">
              <tbody>
                {info.monitors.map((m, i) => (
                  <tr key={i}>
                    <td>
                      {m.name.replace(/^\\\\\.\\/, "")} {m.primary && <span className="chip gold">{t("diag.primary")}</span>}
                    </td>
                    <td className="mono">
                      {m.rect.width}×{m.rect.height}
                    </td>
                    <td className="mono faint">
                      @{m.rect.x},{m.rect.y}
                    </td>
                    <td className="mono faint">×{fmtNum(m.scale, 2)}</td>
                    <td className="mono faint">{m.refresh_hz ? `${m.refresh_hz} Hz` : ""}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>

        <section className="panel diag-paths">
          <div className="panel-head">
            <h3>{t("set.data")}</h3>
          </div>
          <div className="panel-body">
            {(
              [
                [t("set.data_dir"), info.data_dir, "data"],
                [t("set.library_folder"), info.library_dir, "library"],
                [t("set.logs_dir"), info.logs_dir, "logs"],
              ] as const
            ).map(([label, p, which]) => (
              <div key={label} className="path-row">
                <span className="grow">
                  <div>{label}</div>
                  <code className="faint">{p}</code>
                </span>
                <button className="btn ghost sm icon" onClick={() => api.openFolder(which).catch((e) => fail(e))} title={t("set.open")}>
                  <FolderOpen size={13} />
                </button>
              </div>
            ))}
          </div>
        </section>
      </div>
    </div>
  );
}

function Jitter({ title, s }: { title: string; s: JitterStats }) {
  const max = Math.max(1, ...s.histogram.map(([, n]) => n));
  const grade = s.p99_us < 250 ? "ok" : s.p99_us < 1000 ? "gold" : "rec";
  return (
    <div className="jitter">
      <div className="row" style={{ justifyContent: "space-between" }}>
        <b>{title}</b>
        <span className={`chip ${grade}`}>p99 {fmtNum(s.p99_us, 0)} µs</span>
      </div>
      <div className="kv">
        <span>{t("diag.median")}</span>
        <b className="mono">{fmtNum(s.median_us, 1)} µs</b>
        <span>p95</span>
        <b className="mono">{fmtNum(s.p95_us, 1)} µs</b>
        <span>{t("diag.max")}</span>
        <b className="mono">{fmtNum(s.max_us, 0)} µs</b>
        <span>{t("diag.samples")}</span>
        <b className="mono">{s.samples}</b>
      </div>
      <div className="histo">
        {s.histogram.map(([edge, n]) => (
          <div key={edge} className="histo-bar" title={`≤ ${edge} µs · ${n}`}>
            <span style={{ height: `${(n / max) * 100}%` }} />
          </div>
        ))}
      </div>
    </div>
  );
}
