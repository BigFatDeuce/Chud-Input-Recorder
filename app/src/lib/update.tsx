// Updates from inside the app: check, download (with progress), install and
// restart. The backend verifies every download against Chud's signing key.
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { CheckCircle2, Download, ExternalLink, RefreshCw, Sparkles } from "lucide-react";
import { useEffect } from "react";
import { create } from "zustand";
import { Modal } from "../components/ui";
import { t } from "../i18n";
import { api, errorText } from "./api";
import { useStore } from "./store";
import type { UpdateInfo } from "./types";

interface UpdateState {
  info: UpdateInfo | null;
  checking: boolean;
  installing: boolean;
  progress: { done: number; total: number | null } | null;
  error: string | null;
}

export const useUpdate = create<UpdateState>(() => ({ info: null, checking: false, installing: false, progress: null, error: null }));

export async function checkForUpdate(): Promise<UpdateInfo | null> {
  useUpdate.setState({ checking: true, error: null });
  try {
    const info = await api.updateCheck();
    useUpdate.setState({ info, checking: false });
    return info;
  } catch (e) {
    useUpdate.setState({ checking: false, error: errorText(e) });
    return null;
  }
}

export async function installUpdate() {
  const { info } = useUpdate.getState();
  if (info?.mode === "manual") {
    openUrl(info.page).catch(() => {});
    return;
  }
  useUpdate.setState({ installing: true, progress: { done: 0, total: null }, error: null });
  try {
    await api.updateInstall(); // Chud restarts when it succeeds.
  } catch (e) {
    const text = errorText(e);
    // A refused signature gets a plain explanation; the technical text follows.
    const bad = /signature|signed for version/i.test(text);
    useUpdate.setState({ installing: false, progress: null, error: bad ? `${t("update.bad_signature")} (${text})` : text });
  }
}

/** Backend notifications: a newer version found at startup, download progress. */
/** Tells the user a new version exists, with a shortcut to About. */
function announceUpdate(info: UpdateInfo) {
  if (!info.version) return;
  const st = useStore.getState();
  st.toast("info", t("update.available", { v: info.version }), {
    label: t("update.see"),
    run: () => st.setView("about"),
  });
}

export function useUpdateEvents() {
  useEffect(() => {
    const subs = [
      listen<UpdateInfo>("update-available", (e) => {
        useUpdate.setState({ info: e.payload });
        announceUpdate(e.payload);
      }),
      listen<{ done: number; total: number | null }>("update-progress", (e) => useUpdate.setState({ progress: e.payload })),
    ];
    return () => subs.forEach((p) => p.then((f) => f()));
  }, []);
}

function fmtMb(n: number) {
  return (n / 1024 / 1024).toFixed(1);
}

/** The "Updates" card of the About page. */
export function UpdatePanel() {
  const { info, checking, installing, progress, error } = useUpdate();
  const current = useStore((s) => s.info?.version);
  const newer = info?.version;
  const pct = progress?.total ? Math.min(100, Math.round((progress.done / progress.total) * 100)) : null;
  return (
    <div className="update-panel panel">
      <div className="update-head">
        <Sparkles size={18} className="gold-icon" />
        <h3 className="grow">{t("update.title")}</h3>
        <span className="chip">v{current}</span>
      </div>
      {newer ? (
        <>
          <p>
            <b>{t("update.new_version", { v: newer })}</b>
          </p>
          {info?.notes && <pre className="update-notes">{info.notes}</pre>}
          {installing ? (
            <div className="update-progress">
              <div className="update-bar">
                <div style={{ width: `${pct ?? 15}%` }} className={pct === null ? "indeterminate" : ""} />
              </div>
              <span className="faint mono">
                {progress && progress.done > 0 ? `${fmtMb(progress.done)}${progress.total ? ` / ${fmtMb(progress.total)}` : ""} MB` : t("update.starting")}
              </span>
            </div>
          ) : (
            <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
              <button className="btn primary" onClick={installUpdate}>
                {info?.mode === "manual" ? <ExternalLink size={14} /> : <Download size={14} />}
                {info?.mode === "manual" ? t("update.open_page") : t("update.install")}
              </button>
              <button className="btn ghost" onClick={() => info && openUrl(info.page).catch(() => {})}>
                <ExternalLink size={14} /> {t("update.whats_new")}
              </button>
            </div>
          )}
          <p className="faint update-hint">{t(`update.hint_${info?.mode ?? "installer"}` as "update.hint_installer")}</p>
        </>
      ) : (
        <div className="row" style={{ gap: 10, flexWrap: "wrap" }}>
          <button className="btn" onClick={checkForUpdate} disabled={checking}>
            <RefreshCw size={14} className={checking ? "spin" : ""} /> {checking ? t("update.checking") : t("update.check")}
          </button>
          {info && !checking && !error && (
            <span className="muted row" style={{ gap: 6 }}>
              <CheckCircle2 size={14} className="mint-icon" /> {t("update.up_to_date")}
            </span>
          )}
        </div>
      )}
      {error && <p className="error-text">{t("update.failed")} — {error}</p>}
      <p className="faint update-hint">{t("update.privacy")}</p>
    </div>
  );
}

/** Asked once: may Chud look for new versions by itself? */
export function UpdateQuestion() {
  const settings = useStore((s) => s.settings);
  const saveSettings = useStore((s) => s.saveSettings);
  if (!settings || settings.update_check !== null) return null;
  const answer = (yes: boolean) => {
    void saveSettings({ ...settings, update_check: yes }).then(() => {
      if (yes) void checkForUpdate().then((info) => info && announceUpdate(info));
    });
  };
  return (
    <Modal
      title={t("update.ask_title")}
      icon={<Sparkles size={18} className="gold-icon" />}
      onClose={() => answer(false)}
      footer={
        <>
          <button className="btn ghost" onClick={() => answer(false)}>
            {t("update.ask_no")}
          </button>
          <button className="btn primary" onClick={() => answer(true)}>
            {t("update.ask_yes")}
          </button>
        </>
      }
    >
      <p>{t("update.ask_body")}</p>
      <p className="faint">{t("update.ask_note")}</p>
    </Modal>
  );
}
