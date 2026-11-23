// Floating click-through indicator shown while recording (always) and
// while playing (optional). Lives in its own transparent window.
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import { hotkeyText } from "./lib/actions";
import { api } from "./lib/api";
import { useAutomation, useAutomationEvents } from "./lib/automation";
import { fmtTime } from "./lib/format";
import { setKeyTable } from "./lib/keys";
import type { EngineStatus } from "./lib/types";
import { setLanguage, t, useLang } from "./i18n";

export function Overlay() {
  useLang();
  const [s, setS] = useState<EngineStatus | null>(null);
  const [stopKey, setStopKey] = useState("");
  const pending = useAutomation((a) => a.pending);
  useAutomationEvents();
  useEffect(() => {
    document.documentElement.classList.add("overlay-root");
    api.getSettings().then((st) => setLanguage(st.language)).catch(() => {});
    api.engineStatus().then(setS).catch(() => {});
    const un = listen<EngineStatus>("engine-status", (e) => setS(e.payload));
    return () => {
      un.then((f) => f());
    };
  }, []);
  // Read for each announcement: the language and shortcut may have changed.
  const announced = pending?.rule;
  useEffect(() => {
    if (!announced) return;
    Promise.all([api.getSettings(), api.keyTable()])
      .then(([st, keys]) => {
        setKeyTable(keys);
        setLanguage(st.language);
        setStopKey(hotkeyText(st.hotkeys.find((h) => h.action === "emergency_stop")));
      })
      .catch(() => {});
  }, [announced]);
  if (pending && (!s || s.mode === "idle"))
    return (
      <div className="overlay-pill play auto" title={t("auto.announce")}>
        <span className="ov-dot" />
        <span className="ov-name">{pending.macro_name}</span>
        <span className="ov-value">{pending.remaining_s} s</span>
        {stopKey && <span className="ov-sub">■ {stopKey}</span>}
      </div>
    );
  if (!s || s.mode === "idle") return null;

  const rec = s.mode === "recording" || s.mode === "countdown";
  let label: string;
  let value: string;
  if (s.mode === "countdown") {
    label = t("overlay.starting");
    value = String(Math.ceil(s.countdown_ms / 1000));
  } else if (s.mode === "recording") {
    label = s.paused ? t("overlay.paused") : s.waiting_first_input ? t("overlay.armed") : "REC";
    value = fmtTime(s.elapsed_us, 1);
  } else {
    label = s.paused ? t("overlay.paused") : t("overlay.playing");
    value = fmtTime(s.play_position_us, 1);
  }
  return (
    <div className={`overlay-pill ${rec ? "rec" : "play"} ${s.paused ? "paused" : ""}`}>
      <span className="ov-dot" />
      <span className="ov-label">{label}</span>
      <span className="ov-value">{value}</span>
      {s.mode === "recording" && <span className="ov-sub">{s.event_count}</span>}
      {s.mode === "playing" && s.iterations !== 1 && (
        <span className="ov-sub">
          {s.iteration}/{s.iterations === 0 ? "∞" : s.iterations}
        </span>
      )}
    </div>
  );
}
