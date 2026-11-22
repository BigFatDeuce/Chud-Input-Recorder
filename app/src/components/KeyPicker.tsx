// Picks a key: press it, or search the key table.
import { Keyboard, Search } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { t, type TKey } from "../i18n";
import { GROUP_COLORS, keyFromDom, keyLabel, keyTable } from "../lib/keys";
import type { Key, Modifiers } from "../lib/types";
import { Modal } from "./ui";

const GROUP_ORDER = ["letter", "digit", "modifier", "whitespace", "function", "navigation", "symbol", "numpad", "media", "system", "international"];

export function KeyPicker({ title, onPick, onClose }: { title?: string; onPick: (k: Key) => void; onClose: () => void }) {
  const [q, setQ] = useState("");
  const [captured, setCaptured] = useState<Key | null>(null);
  const [listening, setListening] = useState(true);

  useEffect(() => {
    if (!listening) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement)?.tagName === "INPUT") return;
      if (e.key === "Escape" && !e.shiftKey) return; // Modal closes
      e.preventDefault();
      e.stopPropagation();
      const k = keyFromDom(e);
      if (k) setCaptured(k);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [listening]);

  const groups = useMemo(() => {
    const words = q.toLowerCase().split(/\s+/).filter(Boolean);
    const rows = keyTable().filter((r) => {
      if (!words.length) return true;
      const hay = `${r.label} ${r.code} ${r.group}`.toLowerCase();
      return words.every((w) => hay.includes(w));
    });
    const map = new Map<string, typeof rows>();
    for (const r of rows) {
      const g = map.get(r.group) ?? [];
      g.push(r);
      map.set(r.group, g);
    }
    return [...map.entries()].sort((a, b) => GROUP_ORDER.indexOf(a[0]) - GROUP_ORDER.indexOf(b[0]));
  }, [q]);

  return (
    <Modal
      title={title ?? t("keys.pick")}
      icon={<Keyboard size={18} />}
      onClose={onClose}
      wide
      footer={
        <>
          <button className="btn" onClick={onClose}>
            {t("common.cancel")}
          </button>
          <button
            className="btn primary"
            disabled={!captured}
            onClick={() => {
              if (captured) onPick(captured);
              onClose();
            }}
          >
            {t("common.ok")}
          </button>
        </>
      }
    >
      <div className={`key-capture ${listening ? "listening" : ""}`} onClick={() => setListening(true)}>
        {captured ? (
          <>
            <kbd className="big">{keyLabel(captured)}</kbd>
            <span className="faint mono">
              scan {captured.scan.toString(16).toUpperCase()} · vk {captured.vk.toString(16).toUpperCase()}
            </span>
          </>
        ) : (
          <span className="muted">{t("keys.press")}</span>
        )}
      </div>
      <div className="search" style={{ margin: "12px 0" }}>
        <Search size={14} />
        <input
          className="input"
          placeholder={t("keys.search")}
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onFocus={() => setListening(false)}
          onBlur={() => setListening(true)}
        />
      </div>
      <div className="key-groups">
        {groups.map(([g, rows]) => (
          <div key={g} className="key-group">
            <div className="eyebrow" style={{ color: GROUP_COLORS[g] }}>
              {GROUP_ORDER.includes(g) ? t(`keygroup.${g}` as TKey) : g}
            </div>
            <div className="key-grid">
              {rows.map((r) => {
                const k = { scan: r.scan, vk: r.vk };
                const on = captured && captured.scan === r.scan && captured.vk === r.vk;
                return (
                  <button
                    key={`${r.code}`}
                    className={`key-cell ${on ? "on" : ""}`}
                    title={r.code}
                    onClick={() => setCaptured(k)}
                    onDoubleClick={() => {
                      onPick(k);
                      onClose();
                    }}
                  >
                    {r.label}
                  </button>
                );
              })}
            </div>
          </div>
        ))}
      </div>
    </Modal>
  );
}

/** Captures a hotkey (modifiers + key) from the keyboard. */
export function HotkeyCapture({
  onCapture,
  onClose,
}: {
  onCapture: (m: Modifiers, k: Key) => void;
  onClose: () => void;
}) {
  const [mods, setMods] = useState<Modifiers>({ ctrl: false, shift: false, alt: false, meta: false });
  const [key, setKey] = useState<Key | null>(null);
  useEffect(() => {
    const isMod = (c: string) => /^(Control|Shift|Alt|Meta|OS)(Left|Right)?$/.test(c);
    const down = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape" && !e.ctrlKey && !e.altKey && !e.shiftKey) {
        onClose();
        return;
      }
      const m = { ctrl: e.ctrlKey, shift: e.shiftKey, alt: e.altKey, meta: e.metaKey };
      setMods(m);
      if (!isMod(e.code)) {
        const k = keyFromDom(e);
        if (k) setKey(k);
      }
    };
    window.addEventListener("keydown", down, true);
    return () => window.removeEventListener("keydown", down, true);
  }, [onClose]);
  const text = [mods.ctrl && "Ctrl", mods.shift && "Shift", mods.alt && "Alt", mods.meta && "Win", key && keyLabel(key)]
    .filter(Boolean)
    .join(" + ");
  return (
    <Modal
      title={t("keys.hotkey_title")}
      icon={<Keyboard size={18} />}
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>
            {t("common.cancel")}
          </button>
          <button
            className="btn primary"
            disabled={!key}
            onClick={() => {
              if (key) onCapture(mods, key);
              onClose();
            }}
          >
            {t("common.ok")}
          </button>
        </>
      }
    >
      <div className="key-capture listening">
        {text ? <kbd className="big">{text}</kbd> : <span className="muted">{t("keys.hotkey_press")}</span>}
      </div>
      <p className="faint" style={{ marginTop: 12 }}>
        {t("keys.hotkey_hint")}
      </p>
    </Modal>
  );
}
