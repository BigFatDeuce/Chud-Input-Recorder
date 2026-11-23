// Bottom transport bar: record / play controls and the live status readout.
import { Check, OctagonX, Pause, Play, Radio, Repeat, Square, Timer, Volume2, VolumeX, X, Zap } from "lucide-react";
import { useMemo } from "react";
import { useContextMenu, type MenuEntry } from "./ui";
import { tone } from "../lib/sound";
import { t, useLang, fmtInt } from "../i18n";
import { emergencyStop, hotkeyFor, playTarget, toggleRecording, togglePause, togglePlay } from "../lib/actions";
import { api } from "../lib/api";
import { useAutomation } from "../lib/automation";
import { fmtTime } from "../lib/format";
import { useStore } from "../lib/store";

export function RecordButton({ size = 54 }: { size?: number }) {
  useLang();
  const mode = useStore((s) => s.status.mode);
  const paused = useStore((s) => s.status.paused);
  const recording = mode === "recording" || mode === "countdown";
  const disabled = mode === "playing";
  const hk = hotkeyFor("toggle_record");
  return (
    <button
      className={`rec-btn ${recording ? "recording" : ""} ${paused ? "paused" : ""}`}
      style={{ width: size, height: size }}
      onClick={toggleRecording}
      disabled={disabled}
      title={`${recording ? t("transport.stop_recording") : t("transport.record")}${hk ? ` (${hk})` : ""}`}
      aria-label={recording ? t("transport.stop_recording") : t("transport.record")}
    >
      <span className="rec-orbit" />
      <span className="rec-ring" />
      <span className="rec-core">{recording && <span className="rec-square" />}</span>
    </button>
  );
}

const COUNTDOWNS = [0, 3, 5, 10, 15];

/** Countdown before recording, chosen right next to the record button. */
function CountdownPicker() {
  useLang();
  const settings = useStore((s) => s.settings);
  const save = useStore((s) => s.saveSettings);
  const mode = useStore((s) => s.status.mode);
  const menu = useContextMenu();
  if (!settings) return null;
  const ms = settings.record.countdown_ms;
  const secs = Math.round(ms / 1000);
  const choose = (s: number) => save({ ...settings, record: { ...settings.record, countdown_ms: s * 1000 } });
  const mark = (on: boolean) => (on ? <Check size={14} /> : <span style={{ width: 14 }} />);
  const open = (e: React.MouseEvent<HTMLButtonElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    const items: MenuEntry[] = [
      { title: t("countdown.menu") },
      ...COUNTDOWNS.map((s) => ({
        label: s === 0 ? t("countdown.off") : t("countdown.seconds", { n: s }),
        icon: mark(secs === s),
        onClick: () => choose(s),
      })),
      ...(COUNTDOWNS.includes(secs) ? [] : [{ label: t("countdown.seconds", { n: secs }), icon: mark(true) }]),
      { separator: true },
      {
        label: t("countdown.sound"),
        icon: settings.countdown_sound ? <Volume2 size={14} /> : <VolumeX size={14} />,
        shortcut: settings.countdown_sound ? t("countdown.sound_on") : t("countdown.sound_off"),
        onClick: () => {
          const on = !settings.countdown_sound;
          if (on) tone(880, 110);
          save({ ...settings, countdown_sound: on });
        },
      },
    ];
    menu.open(r.left, r.top - 8, items, true);
  };
  return (
    <>
      <button
        className={`countdown-pick ${secs > 0 ? "on" : ""}`}
        onClick={open}
        disabled={mode !== "idle"}
        title={secs > 0 ? t("countdown.title_on", { n: secs }) : t("countdown.title_off")}
        aria-label={t("countdown.menu")}
      >
        <Timer size={14} />
        {secs > 0 && <span className="mono">{secs}s</span>}
      </button>
      {menu.element}
    </>
  );
}

/** Automatic triggers: on, suspended, or about to start a macro. */
function AutomationChip() {
  useLang();
  const { enabled, suspended, pending } = useAutomation();
  const setView = useStore((s) => s.setView);
  if (pending)
    return (
      <button className="btn sm auto-cancel" onClick={() => api.automationCancel()}>
        <X size={14} /> {t("common.cancel")}
      </button>
    );
  if (!enabled) return null;
  return suspended ? (
    <button className="chip rec" onClick={() => api.automationSuspend(false)} title={t("auto.suspended_hint")}>
      <Zap size={11} /> {t("auto.resume_long")}
    </button>
  ) : (
    <button className="chip gold" onClick={() => setView("settings")} title={t("auto.on_hint")}>
      <Zap size={11} /> {t("auto.on")}
    </button>
  );
}

export function Transport() {
  useLang();
  const status = useStore((s) => s.status);
  const fg = useStore((s) => s.foreground);
  const pending = useAutomation((s) => s.pending);
  const docs = useStore((s) => s.docs);
  const listing = useStore((s) => s.listing);
  useStore((s) => s.activeDoc);
  useStore((s) => s.view);
  useStore((s) => s.libSelection);
  const target = playTarget();
  const targetName = useMemo(() => {
    if (!target) return null;
    return docs[target]?.meta.name ?? listing?.entries.find((e) => e.id === target)?.manifest.meta.name ?? null;
  }, [target, docs, listing]);

  const { mode, paused } = status;
  const recording = mode === "recording" || mode === "countdown";
  const playing = mode === "playing";

  let title: string;
  let clock: string;
  let sub: string;
  if (mode === "countdown") {
    title = t("transport.countdown");
    clock = `${Math.ceil(status.countdown_ms / 1000)}`;
    sub = t("transport.countdown_hint");
  } else if (mode === "recording") {
    title = paused ? t("transport.rec_paused") : status.waiting_first_input ? t("transport.armed") : t("transport.recording");
    clock = fmtTime(status.elapsed_us, 1);
    sub = status.waiting_first_input
      ? t("transport.waiting_first")
      : `${fmtInt(status.event_count)} ${t("transport.events")}${fg ? ` · ${fg.process || fg.title}` : ""}`;
  } else if (playing) {
    title = paused ? t("transport.play_paused") : t("transport.playing");
    clock = `${fmtTime(status.play_position_us, 1)} / ${fmtTime(status.play_duration_us, 1)}`;
    const it = status.iterations === 0 ? "∞" : String(status.iterations);
    sub = `${status.macro_name}${status.iterations !== 1 ? ` · ${t("transport.iteration")} ${status.iteration}/${it}` : ""}${status.block ? ` · ${status.block}` : ""}`;
  } else if (pending) {
    title = t("auto.announce");
    clock = `${pending.remaining_s}`;
    sub = pending.macro_name;
  } else {
    title = t("transport.ready");
    clock = "0:00.0";
    sub = targetName ? `${t("transport.target")} ${targetName}` : t("transport.no_target");
  }

  const progress = playing && status.play_duration_us > 0 ? Math.min(1, status.play_position_us / status.play_duration_us) : 0;

  return (
    <footer className={`transport ${recording ? "is-rec" : ""} ${playing ? "is-play" : ""}`}>
      <div className="transport-left">
        <RecordButton />
        <CountdownPicker />
        <button
          className="btn icon"
          onClick={togglePause}
          disabled={mode === "idle" || mode === "countdown"}
          title={`${t("transport.pause")} (${hotkeyFor("toggle_pause")})`}
        >
          <Pause size={16} />
        </button>
        <button
          className={`play-btn ${playing ? "playing" : ""}`}
          onClick={togglePlay}
          disabled={recording || (!playing && !target)}
          title={`${playing ? t("transport.stop") : t("transport.play")} (${hotkeyFor("toggle_play")})`}
        >
          {playing ? <Square size={16} fill="currentColor" /> : <Play size={18} fill="currentColor" />}
        </button>
      </div>

      <div className="transport-center">
        <div className={`status-orb ${mode} ${paused ? "paused" : ""}`} />
        <div className="status-text">
          <div className="status-title">
            {title}
            {status.live && (
              <span className="chip cyan" style={{ marginLeft: 8 }}>
                <Radio size={11} /> {t("transport.live")}
              </span>
            )}
          </div>
          <div className="status-sub ellipsis">{sub}</div>
        </div>
        {mode === "countdown" ? (
          <div key={clock} className="clock mono countdown-num">
            {clock}
          </div>
        ) : (
          <div className="clock mono">{clock}</div>
        )}
        {playing && (
          <div className="transport-progress">
            <div style={{ width: `${progress * 100}%` }} />
          </div>
        )}
      </div>

      <div className="transport-right">
        <AutomationChip />
        {playing && status.iterations !== 1 && (
          <span className="chip gold">
            <Repeat size={11} /> {status.iteration}/{status.iterations === 0 ? "∞" : status.iterations}
          </span>
        )}
        <button className="btn danger" onClick={emergencyStop} title={t("transport.emergency_hint")}>
          <OctagonX size={16} />
          <span className="hide-narrow">{t("transport.emergency")}</span>
          <kbd>{hotkeyFor("emergency_stop")}</kbd>
        </button>
      </div>
    </footer>
  );
}
