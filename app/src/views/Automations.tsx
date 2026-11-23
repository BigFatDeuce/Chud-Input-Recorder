// Settings › Automatic triggers: macros that start by themselves.
import { Crosshair, Play, Plus, Trash2, TriangleAlert } from "lucide-react";
import { useState, type ReactNode } from "react";
import { FieldRow, NumberInput, Toggle } from "../components/ui";
import { fmtRelative, t, tOr, useLang, type TKey } from "../i18n";
import { hotkeyFor } from "../lib/actions";
import { api } from "../lib/api";
import { defaultWhen, newRule, parseTime, ruleProblem, timeText, useAutomation, type WhenType } from "../lib/automation";
import { useStore } from "../lib/store";
import type { AppSettings, Automation, AutomationSettings, AutomationWhen } from "../lib/types";

const WHEN_TYPES: WhenType[] = ["window", "daily", "interval", "startup"];
/** Time left to switch to the window to pick. */
const PICK_DELAY_S = 4;

type CardProps = { title: string; hint?: string; children: ReactNode };

export function Automations({
  s,
  set,
  Card,
}: {
  s: AppSettings;
  set: (p: Partial<AppSettings>) => void;
  Card: (p: CardProps) => ReactNode;
}) {
  useLang();
  const listing = useStore((st) => st.listing);
  const status = useAutomation();
  const [newMacro, setNewMacro] = useState("");
  const a = s.automation;
  const A = (patch: Partial<AutomationSettings>) => set({ automation: { ...a, ...patch } });
  const setRule = (id: string, patch: Partial<Automation>) => A({ rules: a.rules.map((r) => (r.id === id ? { ...r, ...patch } : r)) });

  return (
    <>
      <Card title={t("auto.title")} hint={t("auto.hint")}>
        <FieldRow label={t("auto.enabled")} hint={t("auto.enabled_hint")}>
          <Toggle value={a.enabled} onChange={(enabled) => A({ enabled })} label={t("auto.enabled")} />
        </FieldRow>
        <FieldRow label={t("auto.warning")} hint={t("auto.warning_hint", { hk: hotkeyFor("emergency_stop") })}>
          <NumberInput value={a.warning_s} min={3} max={60} step={1} suffix="s" width={90} onChange={(v) => A({ warning_s: Math.round(v) })} />
        </FieldRow>
        {status.suspended && (
          <FieldRow label={t("auto.suspended")} hint={t("auto.suspended_hint")}>
            <button className="btn primary sm" onClick={() => api.automationSuspend(false)}>
              <Play size={13} /> {t("auto.resume")}
            </button>
          </FieldRow>
        )}
        <ul className="privacy-list auto-safety">
          <li>{t("auto.safe_running")}</li>
          <li>{t("auto.safe_excluded")}</li>
          <li>{t("auto.safe_emergency")}</li>
          <li>{t("auto.safe_window")}</li>
        </ul>
        {!s.close_to_tray && (
          <FieldRow label={t("set.close_to_tray")} hint={t("auto.tray_hint")}>
            <Toggle value={s.close_to_tray} onChange={(close_to_tray) => set({ close_to_tray })} label={t("set.close_to_tray")} />
          </FieldRow>
        )}
      </Card>

      <Card title={t("auto.rules")} hint={t("auto.rules_hint")}>
        {a.rules.map((r) => (
          <Rule
            key={r.id}
            r={r}
            onChange={(patch) => setRule(r.id, patch)}
            onDelete={() => A({ rules: a.rules.filter((x) => x.id !== r.id) })}
          />
        ))}
        <div className="row" style={{ gap: 8, marginTop: a.rules.length ? 12 : 0 }}>
          <select className="select grow" value={newMacro} onChange={(e) => setNewMacro(e.target.value)} aria-label={t("flow.pick_macro")}>
            <option value="">{t("flow.pick_macro")}</option>
            {listing?.entries.map((e) => (
              <option key={e.id} value={e.id}>
                {e.folder ? `${e.folder} / ` : ""}
                {e.manifest.meta.name}
              </option>
            ))}
          </select>
          <button
            className="btn primary"
            disabled={!newMacro}
            onClick={() => {
              const entry = listing?.entries.find((e) => e.id === newMacro);
              A({ rules: [...a.rules, newRule(newMacro, Boolean(entry?.manifest.summary.has_flow))] });
              setNewMacro("");
            }}
          >
            <Plus size={14} /> {t("auto.add")}
          </button>
        </div>
      </Card>
    </>
  );
}

function Rule({ r, onChange, onDelete }: { r: Automation; onChange: (p: Partial<Automation>) => void; onDelete: () => void }) {
  const listing = useStore((st) => st.listing);
  const caps = useStore((st) => st.info?.capabilities);
  const last = useAutomation((st) => st.last[r.id]);
  const entry = listing?.entries.find((e) => e.id === r.macro_id);
  const problem = ruleProblem(r, Boolean(entry) || !listing, caps?.foreground_window ?? true);
  const hasFlow = Boolean(entry?.manifest.summary.has_flow);
  const when = (w: AutomationWhen) => onChange({ when: w });

  return (
    <div className="auto-rule">
      <div className="row" style={{ gap: 10 }}>
        <Toggle value={r.enabled} onChange={(enabled) => onChange({ enabled })} label={entry?.manifest.meta.name ?? t("set.hk_missing_macro")} />
        <strong className="grow ellipsis">{entry?.manifest.meta.name ?? t("set.hk_missing_macro")}</strong>
        {problem && (
          <span className="chip rec">
            <TriangleAlert size={11} /> {t(`auto.problem_${problem}` as TKey)}
          </span>
        )}
        {last && (
          <span className="faint" title={fmtRelative(last.at_ms)}>
            {tOr(`auto.last_${last.what}`, last.what)} · {fmtRelative(last.at_ms)}
          </span>
        )}
        <button className="btn ghost sm icon" onClick={onDelete} aria-label={t("common.delete")} title={t("common.delete")}>
          <Trash2 size={13} />
        </button>
      </div>
      <FieldRow label={t("auto.when")}>
        <select className="select sm" value={r.when.type} onChange={(e) => when(defaultWhen(e.target.value as WhenType))} aria-label={t("auto.when")}>
          {WHEN_TYPES.map((w) => (
            <option key={w} value={w}>
              {t(`auto.when_${w}` as TKey)}
            </option>
          ))}
        </select>
      </FieldRow>
      <WhenFields w={r.when} onChange={when} />
      {(hasFlow || r.use_flow) && (
        <FieldRow label={t("auto.use_flow")} hint={t("auto.use_flow_hint")}>
          <Toggle value={r.use_flow} onChange={(use_flow) => onChange({ use_flow })} label={t("auto.use_flow")} />
        </FieldRow>
      )}
      <FieldRow label={t("auto.stop_on_input")} hint={t("auto.stop_on_input_hint")}>
        <Toggle value={r.stop_on_input} onChange={(stop_on_input) => onChange({ stop_on_input })} label={t("auto.stop_on_input")} />
      </FieldRow>
    </div>
  );
}

function WhenFields({ w, onChange }: { w: AutomationWhen; onChange: (w: AutomationWhen) => void }) {
  const [left, setLeft] = useState(0);
  const toast = useStore((st) => st.toast);
  const fail = useStore((st) => st.fail);

  if (w.type === "startup")
    return (
      <FieldRow label={t("auto.delay")} hint={t("auto.delay_hint")}>
        <NumberInput value={w.delay_s} min={0} max={3600} step={5} suffix="s" width={90} onChange={(v) => onChange({ ...w, delay_s: Math.round(v) })} />
      </FieldRow>
    );
  if (w.type === "interval")
    return (
      <FieldRow label={t("auto.every")} hint={t("auto.every_hint")}>
        <NumberInput value={w.minutes} min={1} max={10080} step={5} suffix="min" width={100} onChange={(v) => onChange({ ...w, minutes: Math.round(v) })} />
      </FieldRow>
    );
  if (w.type === "daily")
    return (
      <>
        <FieldRow label={t("auto.time")} hint={t("auto.time_hint")}>
          <input
            className="input sm mono"
            type="time"
            style={{ width: 110 }}
            aria-label={t("auto.time")}
            key={timeText(w.hour, w.minute)}
            defaultValue={timeText(w.hour, w.minute)}
            onBlur={(e) => {
              const v = parseTime(e.target.value);
              if (v && (v.hour !== w.hour || v.minute !== w.minute)) onChange({ ...w, ...v });
            }}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          />
        </FieldRow>
        <FieldRow label={t("auto.days")}>
          <div className="auto-days">
            {w.days.map((on, i) => (
              <button
                key={i}
                type="button"
                className={`chip ${on ? "gold" : ""}`}
                aria-pressed={on}
                onClick={() => onChange({ ...w, days: w.days.map((d, j) => (j === i ? !d : d)) })}
              >
                {t(`auto.day_${i}` as TKey)}
              </button>
            ))}
          </div>
        </FieldRow>
      </>
    );

  const pick = async () => {
    setLeft(PICK_DELAY_S);
    const timer = window.setInterval(() => setLeft((n) => Math.max(1, n - 1)), 1000);
    try {
      const win = await api.foregroundAfter(PICK_DELAY_S * 1000);
      if (win) onChange({ ...w, process: win.process, title: "" });
      else toast("warning", t("auto.pick_none"));
    } catch (e) {
      fail(e);
    } finally {
      window.clearInterval(timer);
      setLeft(0);
    }
  };
  const text = (field: "process" | "title", ph: string) => (
    <input
      className="input sm"
      style={{ width: 220 }}
      placeholder={ph}
      aria-label={t(field === "process" ? "auto.process" : "auto.window_title")}
      key={w[field]}
      defaultValue={w[field]}
      onBlur={(e) => e.target.value.trim() !== w[field] && onChange({ ...w, [field]: e.target.value.trim() })}
      onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
    />
  );
  return (
    <>
      <FieldRow label={t("auto.process")} hint={t("auto.process_hint")}>
        {text("process", "notepad.exe")}
      </FieldRow>
      <FieldRow label={t("auto.window_title")} hint={t("auto.window_title_hint")}>
        {text("title", "")}
      </FieldRow>
      <FieldRow label={t("auto.pick")} hint={t("auto.pick_hint", { n: PICK_DELAY_S })}>
        <button className="btn sm" onClick={pick} disabled={left > 0}>
          <Crosshair size={13} /> {left > 0 ? t("auto.picking", { n: left }) : t("auto.pick_do")}
        </button>
      </FieldRow>
      <FieldRow label={t("auto.cooldown")} hint={t("auto.cooldown_hint")}>
        <NumberInput value={w.cooldown_s} min={3} max={86400} step={5} suffix="s" width={100} onChange={(v) => onChange({ ...w, cooldown_s: Math.round(v) })} />
      </FieldRow>
    </>
  );
}
