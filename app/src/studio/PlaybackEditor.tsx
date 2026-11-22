// Playback settings editor (per macro in the studio, defaults in settings).
import { Infinity as InfinityIcon } from "lucide-react";
import { FieldRow, NumberInput, Segmented, Slider, Toggle } from "../components/ui";
import { t, useLang } from "../i18n";
import type { CoordMode, KeyMode, MouseMode, PlaybackSettings, TimingMode } from "../lib/types";

const SPEEDS = [0.25, 0.5, 1, 2, 4, 10];

export function PlaybackEditor({
  value,
  onChange,
  compact,
  canGamepad = true,
}: {
  value: PlaybackSettings;
  onChange: (v: PlaybackSettings) => void;
  compact?: boolean;
  canGamepad?: boolean;
}) {
  useLang();
  const set = <K extends keyof PlaybackSettings>(k: K, v: PlaybackSettings[K]) => onChange({ ...value, [k]: v });
  const timingHint: Record<TimingMode, string> = {
    exact: t("pb.timing_exact_hint"),
    relative: t("pb.timing_relative_hint"),
    adaptive: t("pb.timing_adaptive_hint"),
    fast: t("pb.timing_fast_hint"),
    custom: t("pb.timing_custom_hint"),
  };
  const mouseHint: Record<MouseMode, string> = {
    absolute: t("pb.mouse_absolute_hint"),
    relative: t("pb.mouse_relative_hint"),
    off: t("pb.mouse_off_hint"),
  };
  return (
    <div className={`pb-editor ${compact ? "compact" : ""}`}>
      <div className="pb-section">
        <div className="eyebrow">{t("pb.speed_repeat")}</div>
        <FieldRow label={t("pb.speed")} hint={`×${value.speed}`}>
          <div className="col" style={{ gap: 6, alignItems: "stretch", width: "100%" }}>
            <Slider
              value={Math.log2(value.speed)}
              min={-3}
              max={4}
              step={0.05}
              onChange={(l) => set("speed", +Math.pow(2, l).toFixed(2))}
            />
            <div className="row" style={{ gap: 4, justifyContent: "flex-end" }}>
              {SPEEDS.map((s) => (
                <button key={s} className={`chip ${value.speed === s ? "active" : ""}`} onClick={() => set("speed", s)}>
                  ×{s}
                </button>
              ))}
              <NumberInput value={value.speed} min={0.05} max={50} step={0.1} width={70} onChange={(v) => set("speed", v)} />
            </div>
          </div>
        </FieldRow>
        <FieldRow label={t("pb.repeat")} hint={t("pb.repeat_hint")}>
          <NumberInput value={value.repeat} min={0} max={1_000_000} step={1} width={80} onChange={(v) => set("repeat", Math.round(v))} />
          <button className={`btn sm icon ${value.repeat === 0 ? "on" : ""}`} onClick={() => set("repeat", value.repeat === 0 ? 1 : 0)} title={t("pb.forever")}>
            <InfinityIcon size={14} />
          </button>
        </FieldRow>
        <FieldRow label={t("pb.repeat_delay")}>
          <NumberInput value={value.repeat_delay_ms} min={0} max={3_600_000} step={50} suffix="ms" width={110} onChange={(v) => set("repeat_delay_ms", v)} />
        </FieldRow>
      </div>

      <div className="pb-section">
        <div className="eyebrow">{t("pb.timing")}</div>
        <Segmented<TimingMode>
          value={value.timing}
          onChange={(v) => set("timing", v)}
          options={(["exact", "adaptive", "relative", "fast", "custom"] as TimingMode[]).map((m) => ({
            value: m,
            label: t(`pb.timing_${m}`),
            title: timingHint[m],
          }))}
        />
        <p className="pb-hint">{timingHint[value.timing]}</p>
        {value.timing === "adaptive" && (
          <FieldRow label={t("pb.max_lag")}>
            <NumberInput value={value.adaptive_max_lag_ms} min={1} max={10_000} step={5} suffix="ms" width={100} onChange={(v) => set("adaptive_max_lag_ms", v)} />
          </FieldRow>
        )}
        {value.timing === "fast" && (
          <FieldRow label={t("pb.fast_gap")}>
            <NumberInput value={value.fast_gap_us / 1000} min={0} max={1000} step={0.5} suffix="ms" width={100} onChange={(v) => set("fast_gap_us", Math.round(v * 1000))} />
          </FieldRow>
        )}
        {value.timing === "custom" && (
          <>
            <FieldRow label={t("pb.fixed_gap")} hint={t("pb.fixed_gap_hint")}>
              <Toggle value={value.custom_fixed_gap_ms !== null} onChange={(on) => set("custom_fixed_gap_ms", on ? 50 : null)} />
              {value.custom_fixed_gap_ms !== null && (
                <NumberInput value={value.custom_fixed_gap_ms} min={0} max={60_000} step={5} suffix="ms" width={100} onChange={(v) => set("custom_fixed_gap_ms", v)} />
              )}
            </FieldRow>
            {value.custom_fixed_gap_ms === null && (
              <>
                <FieldRow label={t("pb.min_gap")}>
                  <NumberInput value={value.custom_min_gap_ms} min={0} max={60_000} step={5} suffix="ms" width={100} onChange={(v) => set("custom_min_gap_ms", v)} />
                </FieldRow>
                <FieldRow label={t("pb.max_gap")}>
                  <NumberInput value={value.custom_max_gap_ms} min={0} max={3_600_000} step={50} suffix="ms" width={100} onChange={(v) => set("custom_max_gap_ms", v)} />
                </FieldRow>
              </>
            )}
          </>
        )}
      </div>

      <div className="pb-section">
        <div className="eyebrow">{t("pb.inputs")}</div>
        <FieldRow label={t("pb.mouse")} hint={mouseHint[value.mouse]}>
          <Segmented<MouseMode>
            value={value.mouse}
            onChange={(v) => set("mouse", v)}
            options={[
              { value: "absolute", label: t("pb.mouse_absolute") },
              { value: "relative", label: t("pb.mouse_relative") },
              { value: "off", label: t("pb.mouse_off") },
            ]}
          />
        </FieldRow>
        <FieldRow label={t("pb.keys")} hint={value.keys === "physical" ? t("pb.keys_physical_hint") : t("pb.keys_logical_hint")}>
          <Segmented<KeyMode>
            value={value.keys}
            onChange={(v) => set("keys", v)}
            options={[
              { value: "physical", label: t("pb.keys_physical") },
              { value: "logical", label: t("pb.keys_logical") },
            ]}
          />
        </FieldRow>
        <FieldRow label={t("pb.coords")} hint={value.coords === "as_recorded" ? t("pb.coords_recorded_hint") : t("pb.coords_adapt_hint")}>
          <Segmented<CoordMode>
            value={value.coords}
            onChange={(v) => set("coords", v)}
            options={[
              { value: "as_recorded", label: t("pb.coords_recorded") },
              { value: "adapt_to_screens", label: t("pb.coords_adapt") },
            ]}
          />
        </FieldRow>
        <FieldRow label={t("pb.gamepad")} hint={canGamepad ? t("pb.gamepad_hint") : t("pb.gamepad_unavailable")}>
          <Toggle value={value.gamepad} onChange={(v) => set("gamepad", v)} />
        </FieldRow>
      </div>

      <div className="pb-section">
        <div className="eyebrow">{t("pb.safety")}</div>
        <FieldRow label={t("pb.restore_cursor")} hint={t("pb.restore_cursor_hint")}>
          <Toggle value={value.restore_cursor} onChange={(v) => set("restore_cursor", v)} />
        </FieldRow>
        <FieldRow label={t("pb.abort_on_input")} hint={t("pb.abort_on_input_hint")}>
          <Toggle value={value.abort_on_user_input} onChange={(v) => set("abort_on_user_input", v)} />
        </FieldRow>
      </div>
    </div>
  );
}
