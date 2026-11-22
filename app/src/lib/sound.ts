// Countdown beeps, synthesized with Web Audio (no audio files, nothing
// leaves the machine). Chromium only lets a page start audio after a user
// gesture, so the context is unlocked on the first click or key press in
// Chud's window; after that the beeps also play while another window has
// the focus (the usual case: the user switched to the target application).
import { useEffect } from "react";
import { useStore } from "./store";
import type { EngineStatus } from "./types";

let ctx: AudioContext | null = null;

function audio(): AudioContext | null {
  try {
    ctx ??= new AudioContext();
    if (ctx.state === "suspended") void ctx.resume().catch(() => {});
    return ctx;
  } catch {
    return null;
  }
}

/** Short sine blip with a soft attack and an exponential release. */
export function tone(freq: number, ms: number, gain = 0.07) {
  const a = audio();
  if (!a) return;
  const t0 = a.currentTime + 0.01;
  const end = t0 + ms / 1000;
  const o = a.createOscillator();
  const g = a.createGain();
  o.type = "sine";
  o.frequency.setValueAtTime(freq, t0);
  g.gain.setValueAtTime(0, t0);
  g.gain.linearRampToValueAtTime(gain, t0 + 0.006);
  g.gain.exponentialRampToValueAtTime(0.0001, end);
  o.connect(g).connect(a.destination);
  o.start(t0);
  o.stop(end + 0.02);
}

/** What the countdown should sound like when the status moves from `prev` to `next`. */
export function countdownCue(prev: EngineStatus, next: EngineStatus): "tick" | "soft" | "go" | null {
  if (next.mode === "countdown") {
    const sec = Math.ceil(next.countdown_ms / 1000);
    const was = prev.mode === "countdown" ? Math.ceil(prev.countdown_ms / 1000) : null;
    if (sec === was || sec <= 0) return null;
    return sec <= 3 ? "tick" : "soft";
  }
  if (prev.mode === "countdown" && next.mode === "recording") return "go";
  return null;
}

export function useCountdownSound() {
  useEffect(() => {
    const unlock = () => {
      audio();
      window.removeEventListener("pointerdown", unlock, true);
      window.removeEventListener("keydown", unlock, true);
    };
    window.addEventListener("pointerdown", unlock, true);
    window.addEventListener("keydown", unlock, true);
    const unsub = useStore.subscribe((s, p) => {
      if (s.status === p.status || !s.settings?.countdown_sound) return;
      const cue = countdownCue(p.status, s.status);
      if (cue === "tick") tone(880, 110);
      else if (cue === "soft") tone(660, 70, 0.035);
      else if (cue === "go") tone(1320, 260, 0.08);
    });
    return () => {
      unsub();
      window.removeEventListener("pointerdown", unlock, true);
      window.removeEventListener("keydown", unlock, true);
    };
  }, []);
}
