// Inspector: edits the selected event(s), the range, markers and cursor.
import {
  ArrowLeftRight,
  Copy,
  Crosshair,
  Flag,
  Gauge,
  Grid3x3,
  Play,
  Plus,
  Scissors,
  Spline,
  Trash2,
  Waypoints,
  Timer,
  MoveHorizontal,
} from "lucide-react";
import { useMemo, useState, type ReactNode } from "react";
import { KeyPicker } from "../components/KeyPicker";
import { NumberInput, Segmented, Toggle } from "../components/ui";
import { fmtInt, t, tn, useLang, type TKey } from "../i18n";
import { playMacro } from "../lib/actions";
import { category, controlOf, describe, kindOf, withPartners, type Category } from "../lib/events";
import { fmtDuration, fmtTime, msToUs, usToMs } from "../lib/format";
import { keyLabel, PAD_AXES, PAD_BUTTONS } from "../lib/keys";
import { useStore, type DocState } from "../lib/store";
import type { ChudEvent, EventKind, Marker, MouseButton } from "../lib/types";
import { MOUSE_BUTTONS } from "../lib/types";

const CAT_LABEL: Record<Category, TKey> = {
  keyboard: "lane.keyboard",
  mouse_button: "lane.buttons",
  mouse_move: "lane.moves",
  mouse_raw: "lane.raw",
  wheel: "lane.wheel",
  pad_button: "lane.gamepad",
  pad_axis: "lane.axes",
};

const MARKER_COLORS = ["#38d6ff", "#f9c83a", "#8fe8c2", "#ff86b0", "#a99bff", "#ffad5c", "#e0342c"];

export function Inspector({
  doc,
  onExtract,
  onMarkers,
}: {
  doc: DocState;
  onExtract: () => void;
  onMarkers: (m: Marker[]) => void;
}) {
  useLang();
  const selected = useMemo(() => doc.events.filter((e) => doc.selection.has(e.id)), [doc.events, doc.selection]);

  // A single event, or a press with its release, is edited as one item.
  const single = useMemo(() => {
    if (selected.length === 1) return { main: selected[0], partner: partnerOf(doc.events, selected[0]) };
    if (selected.length === 2) {
      const [a, b] = selected;
      const ca = controlOf(a);
      const cb = controlOf(b);
      if (ca && cb && ca.id === cb.id && ca.down && !cb.down) return { main: a, partner: b };
    }
    return null;
  }, [selected, doc.events]);

  if (single) return <SingleEditor doc={doc} ev={single.main} partner={single.partner} />;
  if (selected.length > 1) return <MultiEditor doc={doc} events={selected} onExtract={onExtract} />;
  return <DocEditor doc={doc} onMarkers={onMarkers} />;
}

function partnerOf(events: ChudEvent[], e: ChudEvent): ChudEvent | null {
  if (!controlOf(e)) return null;
  const ids = withPartners(events, [e.id]);
  ids.delete(e.id);
  if (!ids.size) return null;
  const pid = [...ids][0];
  return events.find((x) => x.id === pid) ?? null;
}

function Section({ title, icon, children, right }: { title: string; icon?: ReactNode; children: ReactNode; right?: ReactNode }) {
  return (
    <div className="insp-section">
      <div className="insp-head">
        {icon}
        <span className="eyebrow grow">{title}</span>
        {right}
      </div>
      {children}
    </div>
  );
}

function Line({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="insp-line">
      <span className="insp-label">{label}</span>
      <div className="insp-value">{children}</div>
    </div>
  );
}

// ───────────────────────────── one event ─────────────────────────────

function SingleEditor({ doc, ev, partner }: { doc: DocState; ev: ChudEvent; partner: ChudEvent | null }) {
  const edit = useStore((s) => s.edit);
  const [picking, setPicking] = useState(false);
  const kind = kindOf(ev);
  const down = partner && partner.t > ev.t ? ev : partner && partner.t < ev.t ? partner : ev;
  const up = down === ev ? partner : ev;
  const isHold = Boolean(partner);

  const update = (changes: { e: ChudEvent; t?: number; kind?: EventKind }[]) =>
    edit(doc.id, {
      op: "update",
      edits: changes.map((c) => ({ id: c.e.id, t: Math.max(0, Math.round(c.t ?? c.e.t)), kind: c.kind ?? kindOf(c.e) })),
    });

  /** Applies the same field change to the press and its release. */
  const patchBoth = (patch: Partial<EventKind>) => {
    const list = [ev, ...(partner ? [partner] : [])];
    update(list.map((e) => ({ e, kind: { ...kindOf(e), ...patch } as EventKind })));
  };

  const setTime = (ms: number) => {
    const nt = msToUs(ms);
    if (isHold && up) {
      // Moving a hold keeps its duration.
      const dt = nt - down.t;
      edit(doc.id, { op: "shift", ids: [down.id, up.id], dt: Math.max(dt, -down.t) });
    } else update([{ e: ev, t: nt }]);
  };

  return (
    <div className="inspector">
      <Section title={t(CAT_LABEL[category(kind)])} icon={<Crosshair size={13} />}>
        <div className="insp-title">{describe(kind)}</div>
        <Line label={t("insp.time")}>
          <NumberInput value={usToMs(isHold ? down.t : ev.t)} step={1} min={0} suffix="ms" width={130} onChange={setTime} />
        </Line>
        {isHold && up && (
          <Line label={t("insp.duration")}>
            <NumberInput
              value={usToMs(up.t - down.t)}
              step={1}
              min={1}
              suffix="ms"
              width={130}
              onChange={(ms) => edit(doc.id, { op: "set_hold_duration", id: down.id, duration: msToUs(ms) })}
            />
          </Line>
        )}
        {(kind.type === "key_down" || kind.type === "key_up") && (
          <>
            <Line label={t("insp.key")}>
              <button className="btn sm" onClick={() => setPicking(true)}>
                <kbd>{keyLabel(kind.key)}</kbd> {t("insp.change")}
              </button>
            </Line>
            {kind.type === "key_down" && !isHold && (
              <Line label={t("insp.repeat")}>
                <Toggle value={Boolean(kind.repeat)} onChange={(v) => update([{ e: ev, kind: { ...kind, repeat: v } }])} />
              </Line>
            )}
          </>
        )}
        {(kind.type === "mouse_down" || kind.type === "mouse_up") && (
          <>
            <Line label={t("insp.button")}>
              <select className="select sm" value={kind.button} onChange={(e) => patchBoth({ button: e.target.value as MouseButton })}>
                {MOUSE_BUTTONS.map((b) => (
                  <option key={b} value={b}>
                    {t(`button.${b}`)}
                  </option>
                ))}
              </select>
            </Line>
            <XY x={kind.x} y={kind.y} onChange={(x, y) => update([{ e: ev, kind: { ...kind, x, y } }])} />
          </>
        )}
        {kind.type === "mouse_move" && <XY x={kind.x} y={kind.y} onChange={(x, y) => update([{ e: ev, kind: { ...kind, x, y } }])} />}
        {kind.type === "mouse_raw" && (
          <>
            <Line label="Δx">
              <NumberInput value={kind.dx} step={1} width={110} onChange={(v) => update([{ e: ev, kind: { ...kind, dx: Math.round(v) } }])} />
            </Line>
            <Line label="Δy">
              <NumberInput value={kind.dy} step={1} width={110} onChange={(v) => update([{ e: ev, kind: { ...kind, dy: Math.round(v) } }])} />
            </Line>
          </>
        )}
        {kind.type === "wheel" && (
          <>
            <Line label={t("insp.notches")}>
              <NumberInput
                value={kind.delta / 120}
                step={1}
                width={110}
                onChange={(v) => update([{ e: ev, kind: { ...kind, delta: Math.round(v * 120) } }])}
              />
            </Line>
            <Line label={t("insp.direction")}>
              <Segmented
                value={kind.horizontal ? "h" : "v"}
                onChange={(v) => update([{ e: ev, kind: { ...kind, horizontal: v === "h" } }])}
                options={[
                  { value: "v", label: t("insp.vertical") },
                  { value: "h", label: t("insp.horizontal") },
                ]}
              />
            </Line>
            <XY x={kind.x} y={kind.y} onChange={(x, y) => update([{ e: ev, kind: { ...kind, x, y } }])} />
          </>
        )}
        {kind.type === "pad_button" && (
          <>
            <Line label={t("insp.pad")}>
              <NumberInput value={kind.pad + 1} min={1} max={4} step={1} width={80} onChange={(v) => patchBoth({ pad: Math.round(v) - 1 })} />
            </Line>
            <Line label={t("insp.button")}>
              <select className="select sm" value={kind.button} onChange={(e) => patchBoth({ button: Number(e.target.value) })}>
                {PAD_BUTTONS.map((b, i) => (
                  <option key={i} value={i}>
                    {b}
                  </option>
                ))}
              </select>
            </Line>
          </>
        )}
        {kind.type === "pad_axis" && (
          <>
            <Line label={t("insp.pad")}>
              <NumberInput value={kind.pad + 1} min={1} max={4} step={1} width={80} onChange={(v) => update([{ e: ev, kind: { ...kind, pad: Math.round(v) - 1 } }])} />
            </Line>
            <Line label={t("insp.axis")}>
              <select className="select sm" value={kind.axis} onChange={(e) => update([{ e: ev, kind: { ...kind, axis: Number(e.target.value) } }])}>
                {PAD_AXES.map((a, i) => (
                  <option key={i} value={i}>
                    {a}
                  </option>
                ))}
              </select>
            </Line>
            <Line label={t("insp.value")}>
              <NumberInput
                value={kind.value}
                min={-32768}
                max={32767}
                step={256}
                width={110}
                onChange={(v) => update([{ e: ev, kind: { ...kind, value: Math.round(v) } }])}
              />
            </Line>
          </>
        )}
      </Section>
      <ActionsSection doc={doc} count={partner ? 2 : 1} />
      {picking && (kind.type === "key_down" || kind.type === "key_up") && (
        <KeyPicker onClose={() => setPicking(false)} onPick={(key) => patchBoth({ key })} />
      )}
    </div>
  );
}

function XY({ x, y, onChange }: { x: number; y: number; onChange: (x: number, y: number) => void }) {
  return (
    <Line label="X · Y">
      <div className="row" style={{ gap: 6 }}>
        <NumberInput value={x} step={1} width={78} onChange={(v) => onChange(Math.round(v), y)} />
        <NumberInput value={y} step={1} width={78} onChange={(v) => onChange(x, Math.round(v))} />
      </div>
    </Line>
  );
}

function ActionsSection({ doc, count, onExtract }: { doc: DocState; count: number; onExtract?: () => void }) {
  const edit = useStore((s) => s.edit);
  const ids = [...doc.selection];
  return (
    <Section title={t("insp.actions")}>
      <div className="insp-actions">
        <button className="btn sm" onClick={() => playMacro(doc.id, { kind: "ids", ids: [...withPartners(doc.events, ids)] }, false)}>
          <Play size={13} /> {t("insp.play_selection")}
        </button>
        <button className="btn sm" onClick={() => edit(doc.id, { op: "duplicate", ids: [...withPartners(doc.events, ids)], offset: null }, { select: "created" })}>
          <Copy size={13} /> {t("lib.duplicate")}
        </button>
        {onExtract && (
          <button className="btn sm" onClick={onExtract}>
            <Scissors size={13} /> {t("insp.extract")}
          </button>
        )}
        <button
          className="btn sm danger"
          onClick={() => edit(doc.id, { op: "delete", ids, partners: true }, { select: "none" })}
          title={tn("insp.delete_one", "insp.delete_many", count)}
        >
          <Trash2 size={13} /> {t("common.delete")}
        </button>
      </div>
    </Section>
  );
}

// ───────────────────────────── many events ─────────────────────────────

function MultiEditor({ doc, events, onExtract }: { doc: DocState; events: ChudEvent[]; onExtract: () => void }) {
  const edit = useStore((s) => s.edit);
  const [shiftMs, setShiftMs] = useState(10);
  const [factor, setFactor] = useState(0.5);
  const [gapMs, setGapMs] = useState(50);
  const [gridMs, setGridMs] = useState(50);
  const [strength, setStrength] = useState(100);
  const [dx, setDx] = useState(0);
  const [dy, setDy] = useState(0);
  const [eps, setEps] = useState(2);

  const ids = events.map((e) => e.id);
  const withP = () => [...withPartners(doc.events, ids)];
  const counts = useMemo(() => {
    const m = new Map<Category, number>();
    for (const e of events) m.set(category(e), (m.get(category(e)) ?? 0) + 1);
    return [...m.entries()];
  }, [events]);
  const t0 = events[0].t;
  const t1 = events[events.length - 1].t;
  const positional = events.some((e) => "x" in e);
  const moves = events.filter((e) => e.type === "mouse_move").length;
  const raw = events.filter((e) => e.type === "mouse_raw").length;

  return (
    <div className="inspector">
      <Section title={t("insp.selection")} icon={<Crosshair size={13} />}>
        <div className="insp-title">{tn("insp.n_events_one", "insp.n_events_many", events.length)}</div>
        <div className="insp-counts">
          {counts.map(([c, n]) => (
            <span key={c} className="chip">
              {t(CAT_LABEL[c])} <b>{fmtInt(n)}</b>
            </span>
          ))}
        </div>
        <Line label={t("insp.span")}>
          <span className="mono">
            {fmtTime(t0)} → {fmtTime(t1)} <span className="faint">({fmtDuration(t1 - t0)})</span>
          </span>
        </Line>
      </Section>

      <Section title={t("insp.timing")} icon={<Timer size={13} />}>
        <div className="tool-row">
          <MoveHorizontal size={14} className="faint" />
          <span className="grow">{t("insp.shift")}</span>
          <NumberInput value={shiftMs} step={1} suffix="ms" width={92} onChange={setShiftMs} />
          <div className="btn-group">
            <button className="btn sm" onClick={() => edit(doc.id, { op: "shift", ids: withP(), dt: -msToUs(shiftMs) })}>
              −
            </button>
            <button className="btn sm" onClick={() => edit(doc.id, { op: "shift", ids: withP(), dt: msToUs(shiftMs) })}>
              +
            </button>
          </div>
        </div>
        <div className="tool-row">
          <Gauge size={14} className="faint" />
          <span className="grow" title={t("insp.scale_hint")}>
            {t("insp.scale")}
          </span>
          <NumberInput value={factor} step={0.05} min={0.01} max={100} suffix="×" width={92} onChange={setFactor} />
          <button className="btn sm" onClick={() => edit(doc.id, { op: "scale", ids: withP(), anchor: t0, factor })}>
            {t("common.apply")}
          </button>
        </div>
        <div className="tool-row">
          <ArrowLeftRight size={14} className="faint" />
          <span className="grow" title={t("insp.gaps_hint")}>
            {t("insp.gaps")}
          </span>
          <NumberInput value={gapMs} step={5} min={0} suffix="ms" width={92} onChange={setGapMs} />
          <button className="btn sm" onClick={() => edit(doc.id, { op: "set_gaps", ids: withP(), gap: msToUs(gapMs), simultaneous: 2000 })}>
            {t("common.apply")}
          </button>
        </div>
        <div className="tool-row">
          <Grid3x3 size={14} className="faint" />
          <span className="grow" title={t("insp.quantize_hint")}>
            {t("insp.quantize")}
          </span>
          <NumberInput value={gridMs} step={5} min={0.1} suffix="ms" width={80} onChange={setGridMs} />
          <NumberInput value={strength} step={10} min={0} max={100} suffix="%" width={70} onChange={setStrength} />
          <button
            className="btn sm"
            onClick={() => edit(doc.id, { op: "quantize", ids: withP(), grid: msToUs(gridMs), strength: strength / 100 })}
          >
            {t("common.apply")}
          </button>
        </div>
      </Section>

      {positional && (
        <Section title={t("insp.pointer")} icon={<Waypoints size={13} />}>
          <div className="tool-row">
            <span className="grow">{t("insp.translate")}</span>
            <NumberInput value={dx} step={1} suffix="x" width={74} onChange={(v) => setDx(Math.round(v))} />
            <NumberInput value={dy} step={1} suffix="y" width={74} onChange={(v) => setDy(Math.round(v))} />
            <button className="btn sm" disabled={!dx && !dy} onClick={() => edit(doc.id, { op: "translate", ids, dx, dy })}>
              {t("common.apply")}
            </button>
          </div>
          {moves > 2 && (
            <div className="tool-row">
              <Spline size={14} className="faint" />
              <span className="grow" title={t("insp.simplify_hint")}>
                {t("insp.simplify")}
              </span>
              <NumberInput value={eps} step={0.5} min={0.1} max={100} suffix="px" width={80} onChange={setEps} />
              <button className="btn sm" onClick={() => edit(doc.id, { op: "simplify_mouse", ids, epsilon: eps, max_interval: 250_000 })}>
                {t("common.apply")}
              </button>
            </div>
          )}
        </Section>
      )}
      {raw > 2 && (
        <Section title={t("lane.raw")} icon={<Waypoints size={13} />}>
          <div className="tool-row">
            <span className="grow">{t("insp.merge_raw")}</span>
            <button className="btn sm" onClick={() => edit(doc.id, { op: "merge_raw_motion", ids, interval: 8000 })}>
              {t("common.apply")}
            </button>
          </div>
        </Section>
      )}
      <ActionsSection doc={doc} count={events.length} onExtract={onExtract} />
    </div>
  );
}

// ───────────────────────────── document ─────────────────────────────

function DocEditor({ doc, onMarkers }: { doc: DocState; onMarkers: (m: Marker[]) => void }) {
  const patchDoc = useStore((s) => s.patchDoc);
  const edit = useStore((s) => s.edit);
  const [insertMs, setInsertMs] = useState(500);
  const duration = doc.events.length ? doc.events[doc.events.length - 1].t : 0;
  const range = doc.range;
  return (
    <div className="inspector">
      <Section title={t("insp.cursor")} icon={<Crosshair size={13} />}>
        <Line label={t("insp.time")}>
          <NumberInput
            value={usToMs(doc.cursor)}
            step={10}
            min={0}
            suffix="ms"
            width={130}
            onChange={(ms) => patchDoc(doc.id, { cursor: msToUs(ms) })}
          />
        </Line>
        <div className="tool-row">
          <Timer size={14} className="faint" />
          <span className="grow" title={t("insp.insert_time_hint")}>
            {t("insp.insert_time")}
          </span>
          <NumberInput value={insertMs} step={50} min={1} suffix="ms" width={92} onChange={setInsertMs} />
          <button className="btn sm" onClick={() => edit(doc.id, { op: "insert_time", at: doc.cursor, dt: msToUs(insertMs) })}>
            <Plus size={13} />
          </button>
        </div>
        <div className="insp-actions">
          <button className="btn sm" onClick={() => playMacro(doc.id, { kind: "from", t: doc.cursor }, false)}>
            <Play size={13} /> {t("insp.play_from_cursor")}
          </button>
        </div>
      </Section>

      <Section title={t("insp.range")} icon={<MoveHorizontal size={13} />}>
        {range ? (
          <>
            <Line label={t("insp.start")}>
              <NumberInput
                value={usToMs(range[0])}
                step={10}
                min={0}
                suffix="ms"
                width={130}
                onChange={(ms) => patchDoc(doc.id, { range: [Math.min(msToUs(ms), range[1]), range[1]] })}
              />
            </Line>
            <Line label={t("insp.end")}>
              <NumberInput
                value={usToMs(range[1])}
                step={10}
                min={0}
                suffix="ms"
                width={130}
                onChange={(ms) => patchDoc(doc.id, { range: [range[0], Math.max(range[0], msToUs(ms))] })}
              />
            </Line>
            <Line label={t("insp.duration")}>
              <span className="mono">{fmtDuration(range[1] - range[0])}</span>
            </Line>
            <div className="insp-actions">
              <button className="btn sm" onClick={() => playMacro(doc.id, { kind: "range", start: range[0], end: range[1] }, false)}>
                <Play size={13} /> {t("insp.play_range")}
              </button>
              <button
                className="btn sm"
                onClick={() => {
                  const sel = new Set(doc.events.filter((e) => e.t >= range[0] && e.t <= range[1]).map((e) => e.id));
                  patchDoc(doc.id, { selection: sel });
                }}
              >
                {t("insp.select_range")}
              </button>
              <button
                className="btn sm danger"
                title={t("insp.delete_range_ripple_hint")}
                onClick={() => {
                  edit(doc.id, { op: "delete_range", start: range[0], end: range[1], ripple: true }, { select: "none" });
                  patchDoc(doc.id, { range: null, cursor: range[0] });
                }}
              >
                <Scissors size={13} /> {t("insp.cut_range")}
              </button>
              <button
                className="btn sm"
                title={t("insp.delete_range_hint")}
                onClick={() => edit(doc.id, { op: "delete_range", start: range[0], end: range[1], ripple: false }, { select: "none" })}
              >
                <Trash2 size={13} /> {t("insp.clear_range")}
              </button>
              <button className="btn sm ghost" onClick={() => patchDoc(doc.id, { range: null })}>
                {t("common.clear")}
              </button>
            </div>
          </>
        ) : (
          <p className="faint insp-hint">{t("insp.range_hint")}</p>
        )}
      </Section>

      <Section
        title={t("insp.markers")}
        icon={<Flag size={13} />}
        right={
          <button
            className="btn ghost sm icon"
            title={t("insp.add_marker")}
            onClick={() =>
              onMarkers(
                [...doc.markers, { t: doc.cursor, end: null, label: `${t("studio.marker")} ${doc.markers.length + 1}`, color: MARKER_COLORS[doc.markers.length % MARKER_COLORS.length] }].sort(
                  (a, b) => a.t - b.t,
                ),
              )
            }
          >
            <Plus size={14} />
          </button>
        }
      >
        {doc.markers.length === 0 ? (
          <p className="faint insp-hint">{t("insp.markers_hint")}</p>
        ) : (
          <div className="marker-list">
            {doc.markers.map((m, i) => (
              <div key={i} className="marker-row">
                <button
                  className="marker-swatch"
                  style={{ background: m.color }}
                  onClick={() => {
                    const c = MARKER_COLORS[(MARKER_COLORS.indexOf(m.color) + 1) % MARKER_COLORS.length];
                    onMarkers(doc.markers.map((x, j) => (j === i ? { ...x, color: c } : x)));
                  }}
                  title={t("lib.color")}
                />
                <input
                  className="input sm grow"
                  defaultValue={m.label}
                  onBlur={(e) => e.target.value !== m.label && onMarkers(doc.markers.map((x, j) => (j === i ? { ...x, label: e.target.value } : x)))}
                  onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
                />
                <button className="btn ghost sm mono" onClick={() => patchDoc(doc.id, { cursor: m.t })} title={t("insp.go_to")}>
                  {fmtTime(m.t, 2)}
                </button>
                <button
                  className="btn ghost sm icon"
                  onClick={() => onMarkers(doc.markers.filter((_, j) => j !== i))}
                  aria-label={t("common.delete")}
                >
                  <Trash2 size={12} />
                </button>
              </div>
            ))}
          </div>
        )}
      </Section>

      <Section title={t("insp.document")}>
        <Line label={t("lib.col_events")}>
          <span className="mono">{fmtInt(doc.events.length)}</span>
        </Line>
        <Line label={t("lib.col_duration")}>
          <span className="mono">{fmtDuration(duration)}</span>
        </Line>
        <Line label={t("insp.revision")}>
          <span className="mono">{doc.history.revision}</span>
        </Line>
        {doc.has_original && (
          <p className="faint insp-hint">{t("insp.original_kept")}</p>
        )}
      </Section>
    </div>
  );
}
