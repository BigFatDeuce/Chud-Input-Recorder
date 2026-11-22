import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useEffect, useState } from "react";
import { Starfield } from "./components/Starfield";
import { NavRail, TitleBar, Toasts } from "./components/Shell";
import { Transport } from "./components/Transport";
import { ViewBoundary } from "./components/ViewBoundary";
import { Logo } from "./components/Logo";
import { setLanguage, t, useLang, type TKey } from "./i18n";
import { api, errorText } from "./lib/api";
import { hasFlow, playMacro, playTarget } from "./lib/actions";
import { setKeyTable } from "./lib/keys";
import { useStore } from "./lib/store";
import type { EngineEventMsg, EngineStatus, LibraryEntry, LiveInput, Saved } from "./lib/types";
import { liveBus } from "./lib/live";
import { useCountdownSound } from "./lib/sound";
import { useZoomShortcuts } from "./lib/zoom";
import { useLocalKeyBridge } from "./lib/localKeys";
import { LibraryView } from "./views/Library";
import { StudioView } from "./views/Studio";
import { FlowView } from "./views/Flow";
import { ObservatoryView } from "./views/Observatory";
import { SettingsView } from "./views/Settings";
import { DiagnosticsView } from "./views/Diagnostics";
import { AboutView } from "./views/About";

const NOTICE_KEYS: Record<string, TKey> = {
  gamepad_output_unavailable: "notice.gamepad_output_unavailable",
  input_blocked: "notice.input_blocked",
  recording_empty: "notice.recording_empty",
  linux_input_permission: "notice.linux_input_permission",
  wayland_limited_capture: "notice.wayland_limited_capture",
  stopped_by_user_input: "notice.stopped_by_user_input",
  emergency_stop: "notice.emergency_stop",
};

function noticeText(message: string): string {
  const k = NOTICE_KEYS[message];
  if (k) return t(k);
  if (message.startsWith("save_failed")) return `${t("error.save")} — ${message.slice(13)}`;
  return message;
}

export function App() {
  useLang();
  const [ready, setReady] = useState(false);
  const [bootError, setBootError] = useState<string | null>(null);
  const view = useStore((s) => s.view);
  const settings = useStore((s) => s.settings);
  useCountdownSound();
  useZoomShortcuts();
  useLocalKeyBridge();

  // Boot.
  useEffect(() => {
    (async () => {
      try {
        const [info, settings, keys, status, listing] = await Promise.all([
          api.appInfo(),
          api.getSettings(),
          api.keyTable(),
          api.engineStatus(),
          api.list(),
        ]);
        setKeyTable(keys);
        setLanguage(settings.language);
        document.documentElement.style.setProperty("--glow", String(settings.glow));
        document.documentElement.dataset.reduceMotion = String(settings.reduce_motion);
        useStore.setState({ info, settings, status, listing });
        setReady(true);
      } catch (e) {
        setBootError(errorText(e));
      }
    })();
  }, []);

  // Engine and backend events.
  useEffect(() => {
    const st = useStore.getState;
    const subs = [
      listen<EngineStatus>("engine-status", (e) => useStore.setState({ status: e.payload })),
      listen<LiveInput[]>("engine-live", (e) => liveBus.emit(e.payload)),
      listen<EngineEventMsg>("engine-event", (e) => {
        const m = e.payload;
        if (m.type === "notice") {
          st().toast(m.level === "error" ? "error" : m.level === "warning" ? "warning" : "info", noticeText(m.message));
        } else if (m.type === "foreground") {
          useStore.setState({ foreground: m.window });
        } else if (m.type === "playback_finished") {
          if (m.end.kind === "failed") st().toast("error", `${t("error.play")} — ${m.end.detail}`);
        } else if (m.type === "hotkey") {
          if (m.action === "toggle_play" && !st().status.armed) {
            const id = playTarget();
            if (id) playMacro(id);
          }
        }
      }),
      listen<LibraryEntry>("recording-saved", (e) => {
        const entry = e.payload;
        st().refreshLibrary();
        const name = entry.manifest.meta.name;
        st().toast("success", t("toast.recording_saved", { name }), {
          label: t("common.open"),
          run: () => st().openDoc(entry.id),
        });
        // Jump straight into the studio with the new take.
        st().openDoc(entry.id);
      }),
      listen("library-changed", () => st().refreshLibrary()),
      listen<Saved>("doc-saved", (e) => {
        const d = st().docs[e.payload.id];
        if (d) st().patchDoc(d.id, { dirty: false, history: e.payload.history, meta: { ...d.meta, modified_ms: e.payload.modified_ms } });
      }),
      listen("tray-play", () => {
        const id = playTarget();
        if (id) playMacro(id);
      }),
      getCurrentWebview().onDragDropEvent(async (e) => {
        if (e.payload.type !== "drop") return;
        const paths = e.payload.paths.filter((p) => /\.(chud|json)$/i.test(p));
        if (!paths.length) return;
        try {
          const r = await api.importFiles(paths, st().libFolder);
          await st().refreshLibrary();
          if (r.imported.length) st().toast("success", t("toast.imported", { n: r.imported.length }));
          for (const [p, err] of r.failed) st().toast("error", `${p.split(/[\\/]/).pop()} — ${err}`);
        } catch (err) {
          st().fail(err, "error.import");
        }
      }),
    ];
    return () => {
      subs.forEach((p) => p.then((f) => f()));
    };
  }, []);

  // The play hotkey plays what is on screen.
  const activeDoc = useStore((s) => s.activeDoc);
  const libSel = useStore((s) => s.libSelection);
  useEffect(() => {
    if (!ready) return;
    const id = playTarget();
    api.arm(id, Boolean(id && hasFlow(id))).catch(() => {});
  }, [ready, activeDoc, libSel, view]);

  if (bootError)
    return (
      <div className="boot">
        <Logo size={128} />
        <h2>{t("boot.failed")}</h2>
        <code className="boot-error">{bootError}</code>
      </div>
    );

  return (
    <div className="app">
      <Starfield enabled={settings?.starfield ?? true} still={settings?.reduce_motion ?? false} />
      <TitleBar />
      <div className="app-body">
        <NavRail />
        <main className="stage">
          {!ready ? (
            <div className="boot">
              <Logo size={128} className="boot-logo" />
              <div className="spinner" />
            </div>
          ) : (
            <ViewBoundary view={view}>
              {view === "library" && <LibraryView />}
              {view === "studio" && <StudioView />}
              {view === "flow" && <FlowView />}
              {view === "observatory" && <ObservatoryView />}
              {view === "settings" && <SettingsView />}
              {view === "diagnostics" && <DiagnosticsView />}
              {view === "about" && <AboutView />}
            </ViewBoundary>
          )}
        </main>
      </div>
      <Transport />
      <Toasts />
    </div>
  );
}
