// Settings: language, recording, playback defaults, hotkeys, privacy,
// overlay, appearance, library and data.
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  Clapperboard,
  FolderOpen,
  Globe,
  Keyboard,
  Layers,
  Palette,
  Plus,
  RotateCcw,
  ShieldCheck,
  SlidersHorizontal,
  Trash2,
  TriangleAlert,
  CircleDot,
  X,
} from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";
import { HotkeyCapture, KeyPicker } from "../components/KeyPicker";
import { FieldRow, NumberInput, Segmented, Slider, Toggle, useConfirm } from "../components/ui";
import { UI_SCALES } from "../lib/zoom";
import { t, useLang, type TKey } from "../i18n";
import { hotkeyText } from "../lib/actions";
import { api } from "../lib/api";
import { keyLabel } from "../lib/keys";
import { useStore } from "../lib/store";
import type { AppSettings, Hotkey, HotkeyAction, KeyFilter, RecordOptions } from "../lib/types";
import { PlaybackEditor } from "../studio/PlaybackEditor";

type Section = "general" | "recording" | "playback" | "hotkeys" | "privacy" | "overlay" | "appearance" | "data";

const SECTIONS: { id: Section; icon: typeof Globe; label: TKey }[] = [
  { id: "general", icon: Globe, label: "set.general" },
  { id: "recording", icon: CircleDot, label: "set.recording" },
  { id: "playback", icon: Clapperboard, label: "set.playback" },
  { id: "hotkeys", icon: Keyboard, label: "set.hotkeys" },
  { id: "privacy", icon: ShieldCheck, label: "set.privacy" },
  { id: "overlay", icon: Layers, label: "set.overlay" },
  { id: "appearance", icon: Palette, label: "set.appearance" },
  { id: "data", icon: FolderOpen, label: "set.data" },
];

const SUGGESTED_EXCLUSIONS = ["KeePass.exe", "KeePassXC.exe", "1Password.exe", "Bitwarden.exe", "keepassxc", "bitwarden"];

function actionKey(a: HotkeyAction): string {
  return typeof a === "string" ? a : `trigger:${a.trigger}`;
}

export function SettingsView() {
  useLang();
  const settings = useStore((s) => s.settings);
  const save = useStore((s) => s.saveSettings);
  const [section, setSection] = useState<Section>("general");
  const confirm = useConfirm();
  if (!settings) return null;
  const set = (patch: Partial<AppSettings>) => save({ ...settings, ...patch });

  return (
    <div className="settings">
      <nav className="settings-nav panel">
        <div className="eyebrow" style={{ padding: "14px 14px 6px" }}>
          {t("nav.settings")}
        </div>
        {SECTIONS.map((s) => (
          <button key={s.id} className={`side-item ${section === s.id ? "active" : ""}`} onClick={() => setSection(s.id)}>
            <s.icon size={15} />
            <span className="grow">{t(s.label)}</span>
          </button>
        ))}
        <div className="spacer" />
        <button
          className="side-item"
          onClick={async () => {
            const ok = await confirm.ask({
              title: t("set.reset_title"),
              body: t("set.reset_body"),
              confirm: t("set.reset"),
              cancel: t("common.cancel"),
              danger: true,
            });
            if (ok) {
              const d = await api.defaultSettings();
              await save({ ...d, library_dir: settings.library_dir, triggers: settings.triggers });
            }
          }}
        >
          <RotateCcw size={15} />
          <span className="grow">{t("set.reset")}</span>
        </button>
      </nav>
      <div className="settings-body">
        {section === "general" && <General s={settings} set={set} />}
        {section === "recording" && <Recording s={settings} set={set} />}
        {section === "playback" && (
          <Card title={t("set.playback_defaults")} hint={t("set.playback_defaults_hint")}>
            <PlaybackEditor value={settings.playback} onChange={(playback) => set({ playback })} canGamepad={useStore.getState().info?.capabilities.gamepad_output ?? false} />
          </Card>
        )}
        {section === "hotkeys" && <Hotkeys s={settings} set={set} />}
        {section === "privacy" && <Privacy s={settings} set={set} />}
        {section === "overlay" && <Overlay s={settings} set={set} />}
        {section === "appearance" && <Appearance s={settings} set={set} />}
        {section === "data" && <Data s={settings} set={set} />}
      </div>
      {confirm.element}
    </div>
  );
}

type P = { s: AppSettings; set: (p: Partial<AppSettings>) => void };

function Card({ title, hint, children }: { title: string; hint?: string; children: ReactNode }) {
  return (
    <section className="settings-card panel">
      <div className="settings-card-head">
        <h2>{title}</h2>
        {hint && <p className="muted">{hint}</p>}
      </div>
      {children}
    </section>
  );
}

function General({ s, set }: P) {
  return (
    <>
      <Card title={t("set.general")}>
        <FieldRow label={t("set.language")} hint={t("set.language_hint")}>
          <Segmented
            value={s.language}
            onChange={(language) => set({ language })}
            options={[
              { value: "auto", label: t("set.lang_auto") },
              { value: "fr", label: "Français" },
              { value: "en", label: "English" },
            ]}
          />
        </FieldRow>
        <FieldRow label={t("set.minimize_on_play")} hint={t("set.minimize_on_play_hint")}>
          <Toggle value={s.minimize_on_play} onChange={(minimize_on_play) => set({ minimize_on_play })} />
        </FieldRow>
        <FieldRow label={t("set.minimize_on_record")} hint={t("set.minimize_on_record_hint")}>
          <Toggle value={s.minimize_on_record} onChange={(minimize_on_record) => set({ minimize_on_record })} />
        </FieldRow>
        <FieldRow label={t("set.close_to_tray")} hint={t("set.close_to_tray_hint")}>
          <Toggle value={s.close_to_tray} onChange={(close_to_tray) => set({ close_to_tray })} />
        </FieldRow>
        <FieldRow label={t("set.update_check")} hint={t("set.update_check_hint")}>
          <Toggle value={s.update_check === true} onChange={(on) => set({ update_check: on })} />
        </FieldRow>
        <FieldRow label={t("set.confirm_delete")} hint={t("set.confirm_delete_hint")}>
          <Toggle value={s.confirm_delete} onChange={(confirm_delete) => set({ confirm_delete })} />
        </FieldRow>
      </Card>
      <Card title={t("set.saving")}>
        <FieldRow label={t("set.autosave")} hint={t("set.autosave_hint")}>
          <Toggle value={s.autosave} onChange={(autosave) => set({ autosave })} />
          {s.autosave && (
            <NumberInput value={s.autosave_delay_ms / 1000} min={0.2} max={60} step={0.5} suffix="s" width={90} onChange={(v) => set({ autosave_delay_ms: Math.round(v * 1000) })} />
          )}
        </FieldRow>
        <FieldRow label={t("set.versions_kept")} hint={t("set.versions_kept_hint")}>
          <NumberInput value={s.versions_kept} min={1} max={500} step={1} width={90} onChange={(v) => set({ versions_kept: Math.round(v) })} />
        </FieldRow>
      </Card>
    </>
  );
}

function Recording({ s, set }: P) {
  const r = s.record;
  const R = (patch: Partial<RecordOptions>) => set({ record: { ...r, ...patch } });
  const caps = useStore((st) => st.info?.capabilities);
  const [picking, setPicking] = useState(false);
  const filterKeys = r.key_filter.mode === "all" ? [] : r.key_filter.keys;
  return (
    <>
      <Card title={t("set.capture")} hint={t("set.capture_hint")}>
        <FieldRow label={t("lane.keyboard")}>
          <Toggle value={r.keyboard} onChange={(keyboard) => R({ keyboard })} />
        </FieldRow>
        <FieldRow label={t("lane.buttons")}>
          <Toggle value={r.mouse_buttons} onChange={(mouse_buttons) => R({ mouse_buttons })} />
        </FieldRow>
        <FieldRow label={t("lane.moves")} hint={caps && !caps.absolute_pointer_capture ? t("set.no_absolute") : t("set.moves_hint")}>
          <Toggle value={r.mouse_move} onChange={(mouse_move) => R({ mouse_move })} />
        </FieldRow>
        <FieldRow label={t("lane.raw")} hint={t("set.raw_hint")}>
          <Toggle value={r.mouse_raw} onChange={(mouse_raw) => R({ mouse_raw })} />
        </FieldRow>
        <FieldRow label={t("lane.wheel")}>
          <Toggle value={r.wheel} onChange={(wheel) => R({ wheel })} />
        </FieldRow>
        <FieldRow label={t("lane.gamepad")} hint={t(caps?.os === "windows" ? "set.gamepad_capture_hint_win" : "set.gamepad_capture_hint")}>
          <Toggle value={r.gamepad} onChange={(gamepad) => R({ gamepad })} />
        </FieldRow>
        <FieldRow label={t("set.include_injected")} hint={t("set.include_injected_hint")}>
          <Toggle value={r.include_injected} onChange={(include_injected) => R({ include_injected })} />
        </FieldRow>
      </Card>
      <Card title={t("set.start_stop")}>
        <FieldRow label={t("set.countdown")} hint={t("set.countdown_hint")}>
          <NumberInput value={r.countdown_ms / 1000} min={0} max={30} step={1} suffix="s" width={90} onChange={(v) => R({ countdown_ms: Math.round(v * 1000) })} />
        </FieldRow>
        <FieldRow label={t("set.countdown_sound")} hint={t("set.countdown_sound_hint")}>
          <Toggle value={s.countdown_sound} onChange={(countdown_sound) => set({ countdown_sound })} />
        </FieldRow>
        <FieldRow label={t("set.first_input")} hint={t("set.first_input_hint")}>
          <Toggle value={r.start_on_first_input} onChange={(start_on_first_input) => R({ start_on_first_input })} />
        </FieldRow>
        <FieldRow label={t("set.max_duration")} hint={t("set.max_duration_hint")}>
          <Toggle value={r.max_duration_ms !== null} onChange={(on) => R({ max_duration_ms: on ? 60_000 : null })} />
          {r.max_duration_ms !== null && (
            <NumberInput value={r.max_duration_ms / 1000} min={1} step={10} suffix="s" width={100} onChange={(v) => R({ max_duration_ms: Math.round(v * 1000) })} />
          )}
        </FieldRow>
      </Card>
      <Card title={t("set.refine")}>
        <FieldRow label={t("set.uniform_gap")} hint={t("set.uniform_gap_hint")}>
          <Toggle value={r.uniform_gap_ms !== null} onChange={(on) => R({ uniform_gap_ms: on ? 50 : null })} />
          {r.uniform_gap_ms !== null && (
            <NumberInput value={r.uniform_gap_ms} min={0} step={5} suffix="ms" width={100} onChange={(v) => R({ uniform_gap_ms: v })} />
          )}
        </FieldRow>
        <FieldRow label={t("set.mouse_hz")} hint={t("set.mouse_hz_hint")}>
          <Toggle value={r.mouse_max_hz !== null} onChange={(on) => R({ mouse_max_hz: on ? 125 : null })} />
          {r.mouse_max_hz !== null && (
            <NumberInput value={r.mouse_max_hz} min={1} max={8000} step={25} suffix="Hz" width={100} onChange={(v) => R({ mouse_max_hz: Math.round(v) })} />
          )}
        </FieldRow>
        <FieldRow label={t("set.analog_threshold")} hint={t("set.analog_threshold_hint")}>
          <NumberInput value={r.analog_threshold} min={0} max={32767} step={64} width={100} onChange={(v) => R({ analog_threshold: Math.round(v) })} />
        </FieldRow>
        <FieldRow label={t("set.key_filter")} hint={t("set.key_filter_hint")}>
          <Segmented<KeyFilter["mode"]>
            value={r.key_filter.mode}
            onChange={(mode) => R({ key_filter: mode === "all" ? { mode } : { mode, keys: filterKeys } })}
            options={[
              { value: "all", label: t("set.filter_all") },
              { value: "only", label: t("set.filter_only") },
              { value: "except", label: t("set.filter_except") },
            ]}
          />
        </FieldRow>
        {r.key_filter.mode !== "all" && (
          <div className="filter-keys">
            {filterKeys.map((vk) => (
              <span key={vk} className="key-token">
                <kbd>{keyLabel({ scan: 0, vk })}</kbd>
                <button
                  onClick={() => R({ key_filter: { mode: r.key_filter.mode as "only", keys: filterKeys.filter((x) => x !== vk) } })}
                  aria-label={t("common.remove")}
                >
                  ×
                </button>
              </span>
            ))}
            <button className="btn sm" onClick={() => setPicking(true)}>
              <Plus size={12} /> {t("flow.add_key")}
            </button>
          </div>
        )}
        {picking && (
          <KeyPicker
            onClose={() => setPicking(false)}
            onPick={(k) => {
              if (!filterKeys.includes(k.vk) && r.key_filter.mode !== "all")
                R({ key_filter: { mode: r.key_filter.mode, keys: [...filterKeys, k.vk] } });
            }}
          />
        )}
      </Card>
    </>
  );
}

function Hotkeys({ s, set }: P) {
  const listing = useStore((st) => st.listing);
  const caps = useStore((st) => st.info?.capabilities);
  const [capturing, setCapturing] = useState<number | "new" | null>(null);
  const [newMacro, setNewMacro] = useState("");
  const conflicts = useMemo(() => {
    const seen = new Map<string, number>();
    const out = new Set<number>();
    s.hotkeys.forEach((h, i) => {
      if (!h.enabled) return;
      const k = `${h.modifiers.ctrl}${h.modifiers.shift}${h.modifiers.alt}${h.modifiers.meta}:${h.key.scan}:${h.key.vk}`;
      if (seen.has(k)) {
        out.add(i);
        out.add(seen.get(k)!);
      } else seen.set(k, i);
    });
    return out;
  }, [s.hotkeys]);

  const label = (a: HotkeyAction) => {
    if (typeof a !== "string") {
      const tr = s.triggers.find((x) => x.slot === a.trigger);
      const m = listing?.entries.find((e) => e.id === tr?.macro_id);
      return `${t("set.hk_play_macro")} · ${m?.manifest.meta.name ?? t("set.hk_missing_macro")}`;
    }
    return t(`set.hk_${a}` as TKey);
  };

  const setHk = (i: number, h: Hotkey) => set({ hotkeys: s.hotkeys.map((x, j) => (j === i ? h : x)) });
  const removeHk = (i: number) => {
    const h = s.hotkeys[i];
    const triggers = typeof h.action === "string" ? s.triggers : s.triggers.filter((x) => x.slot !== (h.action as { trigger: number }).trigger);
    set({ hotkeys: s.hotkeys.filter((_, j) => j !== i), triggers });
  };

  return (
    <>
      <Card title={t("set.hotkeys")} hint={caps?.hotkey_swallow === false ? t("set.hk_no_swallow") : t("set.hotkeys_hint")}>
        <div className="hk-table">
          {s.hotkeys.map((h, i) => (
            <div key={actionKey(h.action) + i} className={`hk-row ${conflicts.has(i) ? "conflict" : ""}`}>
              <Toggle value={h.enabled} onChange={(enabled) => setHk(i, { ...h, enabled })} />
              <span className={`grow ${h.action === "emergency_stop" ? "danger-text" : ""}`}>{label(h.action)}</span>
              {conflicts.has(i) && (
                <span className="chip rec" title={t("set.hk_conflict")}>
                  <TriangleAlert size={11} /> {t("set.hk_conflict")}
                </span>
              )}
              <button className="btn sm hk-combo" onClick={() => setCapturing(i)}>
                <kbd>{hotkeyText({ ...h, enabled: true })}</kbd>
              </button>
              {typeof h.action !== "string" ? (
                <button className="btn ghost sm icon" onClick={() => removeHk(i)} aria-label={t("common.delete")}>
                  <Trash2 size={13} />
                </button>
              ) : (
                <span style={{ width: 28 }} />
              )}
            </div>
          ))}
        </div>
        <p className="faint" style={{ marginTop: 10 }}>
          {t("set.hk_emergency_note")}
        </p>
      </Card>
      <Card title={t("set.hk_macros")} hint={t("set.hk_macros_hint")}>
        <div className="row" style={{ gap: 8 }}>
          <select className="select grow" value={newMacro} onChange={(e) => setNewMacro(e.target.value)}>
            <option value="">{t("flow.pick_macro")}</option>
            {listing?.entries.map((e) => (
              <option key={e.id} value={e.id}>
                {e.folder ? `${e.folder} / ` : ""}
                {e.manifest.meta.name}
              </option>
            ))}
          </select>
          <button className="btn primary" disabled={!newMacro} onClick={() => setCapturing("new")}>
            <Plus size={14} /> {t("set.hk_assign")}
          </button>
        </div>
      </Card>
      {capturing !== null && (
        <HotkeyCapture
          onClose={() => setCapturing(null)}
          onCapture={(modifiers, key) => {
            if (capturing === "new") {
              const slot = Math.max(0, ...s.triggers.map((x) => x.slot + 1));
              const entry = listing?.entries.find((e) => e.id === newMacro);
              set({
                hotkeys: [...s.hotkeys, { action: { trigger: slot }, modifiers, key, enabled: true }],
                triggers: [...s.triggers, { slot, macro_id: newMacro, use_flow: Boolean(entry?.manifest.summary.has_flow) }],
              });
              setNewMacro("");
            } else setHk(capturing, { ...s.hotkeys[capturing], modifiers, key });
          }}
        />
      )}
    </>
  );
}

function Privacy({ s, set }: P) {
  const r = s.record;
  const [name, setName] = useState("");
  const caps = useStore((st) => st.info?.capabilities);
  const add = (n: string) => {
    const v = n.trim();
    if (!v || r.excluded_processes.some((x) => x.toLowerCase() === v.toLowerCase())) return;
    set({ record: { ...r, excluded_processes: [...r.excluded_processes, v] } });
    setName("");
  };
  return (
    <>
      <Card title={t("set.privacy")} hint={t("set.privacy_hint")}>
        <ul className="privacy-list">
          <li>{t("set.privacy_local")}</li>
          <li>{t("set.privacy_indicator")}</li>
          <li>{t("set.privacy_passwords")}</li>
          <li>{t("set.privacy_release")}</li>
        </ul>
      </Card>
      <Card title={t("set.exclusions")} hint={caps?.foreground_window === false ? t("set.exclusions_unavailable") : t("set.exclusions_hint")}>
        <div className="row" style={{ gap: 8 }}>
          <input
            className="input grow"
            placeholder={t("set.exclusion_ph")}
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && add(name)}
          />
          <button className="btn" onClick={() => add(name)} disabled={!name.trim()}>
            <Plus size={14} /> {t("common.add")}
          </button>
        </div>
        <div className="row wrap" style={{ gap: 6, marginTop: 10 }}>
          {r.excluded_processes.map((p) => (
            <span key={p} className="tag big">
              {p}
              <button onClick={() => set({ record: { ...r, excluded_processes: r.excluded_processes.filter((x) => x !== p) } })} aria-label={t("common.remove")}>
                <X size={11} />
              </button>
            </span>
          ))}
          {!r.excluded_processes.length && <span className="faint">{t("set.no_exclusions")}</span>}
        </div>
        <div className="row wrap" style={{ gap: 6, marginTop: 12 }}>
          <span className="faint">{t("set.suggestions")}</span>
          {SUGGESTED_EXCLUSIONS.filter((x) => !r.excluded_processes.includes(x)).map((x) => (
            <button key={x} className="chip" onClick={() => add(x)}>
              + {x}
            </button>
          ))}
        </div>
      </Card>
      <Card title={t("set.only_process")} hint={t("set.only_process_hint")}>
        <FieldRow label={t("set.only_process_on")}>
          <Toggle value={r.only_process !== null} onChange={(on) => set({ record: { ...r, only_process: on ? "" : null } })} />
          {r.only_process !== null && (
            <input
              className="input sm"
              style={{ width: 200 }}
              placeholder="game.exe"
              defaultValue={r.only_process}
              onBlur={(e) => set({ record: { ...r, only_process: e.target.value.trim() } })}
            />
          )}
        </FieldRow>
      </Card>
    </>
  );
}

function Overlay({ s, set }: P) {
  return (
    <Card title={t("set.overlay")} hint={t("set.overlay_hint")}>
      <FieldRow label={t("set.overlay_recording")} hint={t("set.overlay_recording_hint")}>
        <Toggle value disabled onChange={() => {}} />
      </FieldRow>
      <FieldRow label={t("set.overlay_playing")}>
        <Toggle value={s.overlay.show_when_playing} onChange={(v) => set({ overlay: { ...s.overlay, show_when_playing: v } })} />
      </FieldRow>
      <FieldRow label={t("set.overlay_position")}>
        <Segmented
          value={s.overlay.position}
          onChange={(position) => set({ overlay: { ...s.overlay, position } })}
          options={[
            { value: "top", label: t("set.top") },
            { value: "bottom", label: t("set.bottom") },
          ]}
        />
      </FieldRow>
    </Card>
  );
}

function Appearance({ s, set }: P) {
  return (
    <Card title={t("set.appearance")}>
      <FieldRow label={t("set.ui_scale")} hint={t("set.ui_scale_hint")}>
        <Segmented
          value={String(s.ui_scale)}
          onChange={(v) => set({ ui_scale: Number(v) })}
          options={UI_SCALES.map((v) => ({ value: String(v), label: `${Math.round(v * 100)} %` }))}
        />
      </FieldRow>
      <FieldRow label={t("set.starfield")} hint={t("set.starfield_hint")}>
        <Toggle value={s.starfield} onChange={(starfield) => set({ starfield })} />
      </FieldRow>
      <FieldRow label={t("set.glow")} hint={`${Math.round(s.glow * 100)} %`}>
        <div style={{ width: 220 }}>
          <Slider value={s.glow} min={0} max={1} step={0.05} onChange={(glow) => set({ glow })} />
        </div>
      </FieldRow>
      <FieldRow label={t("set.reduce_motion")} hint={t("set.reduce_motion_hint")}>
        <Toggle value={s.reduce_motion} onChange={(reduce_motion) => set({ reduce_motion })} />
      </FieldRow>
    </Card>
  );
}

function Data({ s, set }: P) {
  const info = useStore((st) => st.info);
  const listing = useStore((st) => st.listing);
  const refresh = useStore((st) => st.refreshLibrary);
  const fail = useStore((st) => st.fail);
  const open = (which: "data" | "logs" | "library") => api.openFolder(which).catch((e) => fail(e));
  return (
    <>
      <Card title={t("set.library")} hint={t("set.library_hint")}>
        <FieldRow label={t("set.library_folder")} hint={listing?.root ?? info?.library_dir}>
          <button className="btn sm" onClick={() => open("library")}>
            <FolderOpen size={13} /> {t("set.open")}
          </button>
          <button
            className="btn sm"
            onClick={async () => {
              const dir = await openDialog({ directory: true, multiple: false });
              if (typeof dir === "string") {
                set({ library_dir: dir });
                window.setTimeout(refresh, 200);
              }
            }}
          >
            {t("set.change")}
          </button>
          {s.library_dir && (
            <button
              className="btn sm ghost"
              onClick={() => {
                set({ library_dir: null });
                window.setTimeout(refresh, 200);
              }}
            >
              {t("set.default")}
            </button>
          )}
        </FieldRow>
      </Card>
      {info && (
        <Card title={t("set.data")}>
          <FieldRow label={t("set.mode")} hint={info.portable ? t("set.portable_on") : t("set.portable_off")}>
            <span className={`chip ${info.portable ? "gold" : ""}`}>{info.portable ? t("set.portable") : t("set.installed")}</span>
          </FieldRow>
          <FieldRow label={t("set.data_dir")} hint={info.data_dir}>
            <button className="btn sm" onClick={() => open("data")}>
              <FolderOpen size={13} /> {t("set.open")}
            </button>
          </FieldRow>
          <FieldRow label={t("set.logs_dir")} hint={info.logs_dir}>
            <button className="btn sm" onClick={() => open("logs")}>
              <FolderOpen size={13} /> {t("set.open")}
            </button>
          </FieldRow>
          <FieldRow label={t("set.settings_file")} hint={info.settings_file}>
            <SlidersHorizontal size={14} className="faint" />
          </FieldRow>
        </Card>
      )}
    </>
  );
}
