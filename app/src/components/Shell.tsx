// Window chrome: title bar with document tabs, navigation rail, toasts.
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Activity,
  AudioWaveform,
  Info,
  Library,
  Minus,
  Radar,
  Settings,
  Square,
  Workflow,
  X,
  Copy as Restore,
} from "lucide-react";
import { useEffect, useState } from "react";
import { t, useLang, type TKey } from "../i18n";
import { useStore, type View } from "../lib/store";
import { Logo } from "./Logo";

export function TitleBar() {
  useLang();
  const docs = useStore((s) => s.docs);
  const activeDoc = useStore((s) => s.activeDoc);
  const view = useStore((s) => s.view);
  const openDoc = useStore((s) => s.openDoc);
  const closeDoc = useStore((s) => s.closeDoc);
  const [maximized, setMaximized] = useState(false);
  const win = getCurrentWindow();

  useEffect(() => {
    let un: (() => void) | undefined;
    win.isMaximized().then(setMaximized);
    win.onResized(() => win.isMaximized().then(setMaximized)).then((u) => (un = u));
    return () => un?.();
  }, [win]);

  return (
    <header className="titlebar" data-tauri-drag-region onDoubleClick={() => win.toggleMaximize()}>
      <div className="titlebar-brand" data-tauri-drag-region>
        <Logo size={22} />
        <span data-tauri-drag-region>
          Chud <b>Input Recorder</b>
        </span>
      </div>
      <div className="doc-tabs" data-tauri-drag-region>
        {Object.values(docs).map((d) => (
          <div
            key={d.id}
            className={`doc-tab ${d.id === activeDoc && (view === "studio" || view === "flow") ? "active" : ""}`}
            onMouseDown={(e) => {
              if (e.button === 1) {
                e.preventDefault();
                closeDoc(d.id);
              }
            }}
            onClick={() => openDoc(d.id)}
            onDoubleClick={(e) => e.stopPropagation()}
            title={d.meta.name}
          >
            <span className="doc-dot" style={{ background: d.meta.color }} />
            <span className="ellipsis">{d.meta.name}</span>
            {d.dirty && <span className="dirty" title={t("doc.unsaved")} />}
            <button
              className="doc-close"
              onClick={(e) => {
                e.stopPropagation();
                closeDoc(d.id);
              }}
              aria-label={t("common.close")}
            >
              <X size={12} />
            </button>
          </div>
        ))}
      </div>
      <div className="win-controls">
        <button onClick={() => win.minimize()} aria-label={t("window.minimize")}>
          <Minus size={15} />
        </button>
        <button onClick={() => win.toggleMaximize()} aria-label={t("window.maximize")}>
          {maximized ? <Restore size={13} /> : <Square size={12} />}
        </button>
        <button className="close" onClick={() => win.close()} aria-label={t("common.close")}>
          <X size={16} />
        </button>
      </div>
    </header>
  );
}

const NAV: { view: View; icon: typeof Library; label: TKey }[] = [
  { view: "library", icon: Library, label: "nav.library" },
  { view: "studio", icon: AudioWaveform, label: "nav.studio" },
  { view: "flow", icon: Workflow, label: "nav.flow" },
  { view: "observatory", icon: Radar, label: "nav.observatory" },
];

const NAV_BOTTOM: { view: View; icon: typeof Library; label: TKey }[] = [
  { view: "diagnostics", icon: Activity, label: "nav.diagnostics" },
  { view: "settings", icon: Settings, label: "nav.settings" },
  { view: "about", icon: Info, label: "nav.about" },
];

export function NavRail() {
  useLang();
  const view = useStore((s) => s.view);
  const setView = useStore((s) => s.setView);
  const activeDoc = useStore((s) => s.activeDoc);
  const btn = (n: (typeof NAV)[number]) => {
    const needsDoc = n.view === "studio" && !activeDoc;
    return (
      <button
        key={n.view}
        className={`rail-btn ${view === n.view ? "active" : ""}`}
        onClick={() => setView(n.view)}
        disabled={needsDoc}
        title={needsDoc ? t("nav.open_first") : t(n.label)}
      >
        <n.icon size={20} strokeWidth={1.7} />
        <span>{t(n.label)}</span>
      </button>
    );
  };
  return (
    <nav className="rail">
      {NAV.map(btn)}
      <div className="spacer" />
      {NAV_BOTTOM.map(btn)}
    </nav>
  );
}

export function Toasts() {
  const toasts = useStore((s) => s.toasts);
  const dismiss = useStore((s) => s.dismiss);
  return (
    <div className="toasts">
      {toasts.map((x) => (
        <div key={x.id} className={`toast ${x.level}`}>
          <span className={`toast-dot ${x.level}`} />
          <div className="grow" style={{ lineHeight: 1.45 }}>
            {x.text}
          </div>
          {x.action && (
            <button
              className="btn sm"
              onClick={() => {
                x.action!.run();
                dismiss(x.id);
              }}
            >
              {x.action.label}
            </button>
          )}
          <button className="btn ghost sm icon" onClick={() => dismiss(x.id)} aria-label={t("common.close")}>
            <X size={13} />
          </button>
        </div>
      ))}
    </div>
  );
}
