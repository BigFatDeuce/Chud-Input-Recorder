// About: brand, version, principles, links.
import { openUrl } from "@tauri-apps/plugin-opener";
import { ExternalLink, Lock, ShieldCheck, OctagonX, Cpu } from "lucide-react";
import { Logo } from "../components/Logo";
import { t, useLang } from "../i18n";
import { hotkeyFor } from "../lib/actions";
import { useStore } from "../lib/store";

const REPO = "https://github.com/BigFatDeuce/Chud-Input-Recorder";

export function AboutView() {
  useLang();
  const info = useStore((s) => s.info);
  return (
    <div className="about">
      <div className="about-hero">
        <div className="about-logo">
          <Logo size={160} />
        </div>
        <h1>
          Chud <span className="gold-text">Input Recorder</span>
        </h1>
        <p className="muted about-tagline">{t("about.tagline")}</p>
        <div className="row" style={{ gap: 8, justifyContent: "center" }}>
          <span className="chip gold">v{info?.version}</span>
          <span className="chip">
            {info?.os} · {info?.arch}
          </span>
          {info?.portable && <span className="chip cyan">{t("set.portable")}</span>}
        </div>
      </div>
      <div className="about-grid">
        <div className="about-card panel">
          <Lock size={18} />
          <h3>{t("about.local_title")}</h3>
          <p className="muted">{t("about.local_body")}</p>
        </div>
        <div className="about-card panel">
          <ShieldCheck size={18} />
          <h3>{t("about.visible_title")}</h3>
          <p className="muted">{t("about.visible_body")}</p>
        </div>
        <div className="about-card panel">
          <OctagonX size={18} />
          <h3>{t("about.stop_title")}</h3>
          <p className="muted">{t("about.stop_body", { hk: hotkeyFor("emergency_stop") || "—" })}</p>
        </div>
        <div className="about-card panel">
          <Cpu size={18} />
          <h3>{t("about.open_title")}</h3>
          <p className="muted">{t("about.open_body")}</p>
        </div>
      </div>
      <div className="row" style={{ gap: 10, justifyContent: "center", marginTop: 18 }}>
        <button className="btn" onClick={() => openUrl(REPO).catch(() => {})}>
          <ExternalLink size={14} /> GitHub
        </button>
      </div>
      <p className="faint about-credits">{t("about.credits")}</p>
    </div>
  );
}
