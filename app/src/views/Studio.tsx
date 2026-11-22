// Studio: timeline editor with smart tools, inspector, playback settings,
// statistics, spatial map and event list.
import {
  AudioWaveform,
  ClipboardPaste,
  Copy,
  Eraser,
  Eye,
  Flag,
  Magnet,
  Maximize2,
  MousePointerClick,
  Play,
  Plus,
  Redo2,
  Repeat,
  RotateCcw,
  Save,
  Scissors,
  SkipForward,
  Sparkles,
  Trash2,
  Undo2,
  Wand,
  ZoomIn,
  ZoomOut,
  Keyboard,
  Timer,
  Rows3,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { KeyPicker } from "../components/KeyPicker";
import { RecordButton } from "../components/Transport";
import { Modal, NumberInput, Segmented, Toggle, useConfirm, useContextMenu, usePrompt, type MenuEntry } from "../components/ui";
import { fmtInt, fmtRelative, t, tn, useLang, type TKey } from "../i18n";
import { playMacro } from "../lib/actions";
import { api } from "../lib/api";
import { buildModel, kindOf, withPartners } from "../lib/events";
import { fmtDuration, fmtTime, msToUs } from "../lib/format";
import { keyLabel } from "../lib/keys";
import { macroIcon } from "../lib/icons";
import { useActiveDoc, useStore, type DocState } from "../lib/store";
import type { ChudEvent, CleanupOptions, EditOp, EventKind, Key, Marker, Micros, MouseButton, Repetition } from "../lib/types";
import { MOUSE_BUTTONS } from "../lib/types";
import { EventList } from "../studio/EventList";
import { Inspector } from "../studio/Inspector";
import { DEFAULT_VISIBILITY, laneIds, layoutLanes, type LaneVisibility } from "../studio/lanes";
import { MapPanel } from "../studio/MapPanel";
import { PlaybackEditor } from "../studio/PlaybackEditor";
import { StatsPanel } from "../studio/StatsPanel";
import { Timeline, type TimelineApi, type TimelineContext } from "../studio/Timeline";

type Tab = "inspector" | "playback" | "stats" | "map" | "events";

function load<T>(k: string, d: T): T {
  try {
    const v = localStorage.getItem(k);
    return v ? { ...d, ...(JSON.parse(v) as T) } : d;
  } catch {
    return d;
  }
}

export function StudioView() {
  useLang();
  const doc = useActiveDoc();
  if (!doc) return <NoDoc />;
  return <Studio key={doc.id} doc={doc} />;
}

function NoDoc() {
  const listing = useStore((s) => s.listing);
  const openDoc = useStore((s) => s.openDoc);
  const recent = [...(listing?.entries ?? [])].sort((a, b) => b.manifest.meta.modified_ms - a.manifest.meta.modified_ms).slice(0, 6);
  return (
    <div className="empty-hero">
      <RecordButton size={96} />
      <h2>{t("studio.no_doc")}</h2>
      <p className="muted">{t("studio.no_doc_body")}</p>
      {recent.length > 0 && (
        <div className="recent-list">
          {recent.map((e) => {
            const I = macroIcon(e.manifest.meta.icon);
            return (
              <button key={e.id} className="recent-item" onClick={() => openDoc(e.id)}>
                <I size={15} color={e.manifest.meta.color} />
                <span className="grow ellipsis">{e.manifest.meta.name}</span>
                <span className="faint">{fmtRelative(e.manifest.meta.modified_ms)}</span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

/** Last pointer position at or before `t` (for inserted clicks). */
function pointerAt(events: ChudEvent[], at: Micros): { x: number; y: number } | null {
  for (let i = events.length - 1; i >= 0; i--) {
    const e = events[i];
    if (e.t > at) continue;
    if ("x" in e && "y" in e) return { x: e.x, y: e.y };
  }
  for (const e of events) if ("x" in e && "y" in e) return { x: e.x, y: e.y };
  return null;
}

function Studio({ doc }: { doc: DocState }) {
  useLang();
  const tl = useRef<TimelineApi>(null);
  const edit = useStore((s) => s.edit);
  const patchDoc = useStore((s) => s.patchDoc);
  const undo = useStore((s) => s.undo);
  const redo = useStore((s) => s.redo);
  const toast = useStore((s) => s.toast);
  const fail = useStore((s) => s.fail);
  const clipboard = useStore((s) => s.clipboard);
  const setClipboard = useStore((s) => s.setClipboard);
  const status = useStore((s) => s.status);
  const view = useStore((s) => s.view);
  const caps = useStore((s) => s.info?.capabilities);
  const [tab, setTab] = useState<Tab>(() => load("studio.tab", { v: "inspector" as Tab }).v);
  const [vis, setVis] = useState<LaneVisibility>(() => load("studio.lanes", DEFAULT_VISIBILITY));
  const [compact, setCompact] = useState<boolean>(() => load("studio.compact", { v: false }).v);
  const [snap, setSnap] = useState<boolean>(() => load("studio.snap", { v: true }).v);
  const [tool, setTool] = useState<ToolId | null>(null);
  const [inserting, setInserting] = useState<InsertKind | null>(null);
  const [reps, setReps] = useState(false);
  const [name, setName] = useState(doc.meta.name);
  const menu = useContextMenu();
  const confirm = useConfirm();
  const prompt = usePrompt();

  useEffect(() => setName(doc.meta.name), [doc.meta.name]);
  useEffect(() => {
    try {
      localStorage.setItem("studio.tab", JSON.stringify({ v: tab }));
      localStorage.setItem("studio.lanes", JSON.stringify(vis));
      localStorage.setItem("studio.compact", JSON.stringify({ v: compact }));
      localStorage.setItem("studio.snap", JSON.stringify({ v: snap }));
    } catch {
      /* ignore */
    }
  }, [tab, vis, compact, snap]);

  const model = useMemo(() => buildModel(doc.events), [doc.events]);
  const { lanes, height } = useMemo(() => layoutLanes(model, vis, compact), [model, vis, compact]);
  const playhead = status.mode === "playing" && status.macro_id === doc.id ? status.play_position_us : null;
  const selIds = useCallback(() => [...withPartners(doc.events, doc.selection)], [doc.events, doc.selection]);

  // ── document actions ──
  const save = async () => {
    try {
      const r = await api.docSave(doc.id);
      if (r) {
        patchDoc(doc.id, { dirty: false, history: r.history, meta: { ...doc.meta, modified_ms: r.modified_ms } });
        useStore.getState().refreshLibrary();
      }
    } catch (e) {
      fail(e, "error.save");
    }
  };

  const rename = async (n: string) => {
    const v = n.trim();
    if (!v || v === doc.meta.name) {
      setName(doc.meta.name);
      return;
    }
    try {
      const meta = await api.setMeta(doc.id, { name: v });
      patchDoc(doc.id, { meta });
      useStore.getState().refreshLibrary();
    } catch (e) {
      fail(e, "error.library");
      setName(doc.meta.name);
    }
  };

  const setMarkers = (markers: Marker[]) => {
    const sorted = [...markers].sort((a, b) => a.t - b.t);
    patchDoc(doc.id, { markers: sorted, dirty: true });
    api.docSetMarkers(doc.id, sorted).catch((e) => fail(e, "error.edit"));
  };

  const addMarker = (at: Micros) =>
    setMarkers([
      ...doc.markers,
      { t: at, end: null, label: `${t("studio.marker")} ${doc.markers.length + 1}`, color: ["#38d6ff", "#f9c83a", "#8fe8c2", "#ff86b0", "#a99bff"][doc.markers.length % 5] },
    ]);

  const copy = () => {
    const ids = new Set(selIds());
    const items = doc.events.filter((e) => ids.has(e.id));
    if (!items.length) return false;
    const t0 = items[0].t;
    setClipboard({ items: items.map((e) => [e.t - t0, kindOf(e)] as [Micros, EventKind]) });
    toast("info", tn("toast.copied_one", "toast.copied_many", items.length));
    return true;
  };
  const paste = (at: Micros) => {
    if (!clipboard) return;
    edit(doc.id, { op: "paste", items: clipboard.items as [Micros, EventKind][], at }, { select: "created" });
  };
  const del = () => {
    if (!doc.selection.size) return;
    edit(doc.id, { op: "delete", ids: [...doc.selection], partners: true }, { select: "none" });
  };
  const duplicate = () => {
    if (!doc.selection.size) return;
    edit(doc.id, { op: "duplicate", ids: selIds(), offset: null }, { select: "created" });
  };
  const nudge = (dt: Micros) => {
    if (!doc.selection.size) return;
    edit(doc.id, { op: "shift", ids: selIds(), dt });
  };

  const extract = async () => {
    const ids = selIds();
    if (!ids.length) return;
    const n = await prompt.ask({
      title: t("studio.extract_title"),
      value: `${doc.meta.name} — ${t("studio.extract_suffix")}`,
      confirm: t("common.create"),
      cancel: t("common.cancel"),
    });
    if (!n?.trim()) return;
    try {
      const id = await api.docExtract(doc.id, ids, n.trim());
      await useStore.getState().refreshLibrary();
      toast("success", t("toast.extracted", { name: n.trim() }), { label: t("common.open"), run: () => useStore.getState().openDoc(id) });
    } catch (e) {
      fail(e, "error.edit");
    }
  };

  const revert = async () => {
    const ok = await confirm.ask({
      title: t("studio.revert_title"),
      body: t("studio.revert_body"),
      confirm: t("studio.revert"),
      cancel: t("common.cancel"),
      danger: true,
    });
    if (ok) edit(doc.id, { op: "revert_original" }, { select: "none" });
  };

  // ── keyboard ──
  useEffect(() => {
    if (view !== "studio") return;
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement;
      if (el?.tagName === "INPUT" || el?.tagName === "TEXTAREA" || el?.tagName === "SELECT") return;
      if (document.querySelector(".modal-backdrop, .menu")) return;
      const ctrl = e.ctrlKey || e.metaKey;
      const k = e.key.toLowerCase();
      const end = doc.events.length ? doc.events[doc.events.length - 1].t : 0;
      let handled = true;
      if (ctrl && k === "z" && !e.shiftKey) undo(doc.id);
      else if (ctrl && (k === "y" || (k === "z" && e.shiftKey))) redo(doc.id);
      else if (ctrl && k === "s") save();
      else if (ctrl && k === "a") patchDoc(doc.id, { selection: new Set(doc.events.map((x) => x.id)) });
      else if (ctrl && k === "c") copy();
      else if (ctrl && k === "x") {
        if (copy()) del();
      } else if (ctrl && k === "v") paste(doc.cursor);
      else if (ctrl && k === "d") duplicate();
      else if (ctrl && k === "e") extract();
      else if (e.key === "Delete" || e.key === "Backspace") del();
      else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
        const step = ctrl ? 100_000 : e.shiftKey ? 10_000 : 1_000;
        const dir = e.key === "ArrowLeft" ? -1 : 1;
        if (doc.selection.size) nudge(dir * step);
        else {
          const c = Math.max(0, doc.cursor + dir * step * 10);
          patchDoc(doc.id, { cursor: c });
          tl.current?.reveal(c);
        }
      } else if (e.key === "Home") {
        patchDoc(doc.id, { cursor: 0 });
        tl.current?.reveal(0);
      } else if (e.key === "End") {
        patchDoc(doc.id, { cursor: end });
        tl.current?.reveal(end);
      } else if (e.key === "+" || e.key === "=") tl.current?.zoomBy(1.5, doc.cursor);
      else if (e.key === "-" || e.key === "_") tl.current?.zoomBy(1 / 1.5, doc.cursor);
      else if (k === "f" || e.key === "0") tl.current?.fit();
      else if (k === "m") addMarker(doc.cursor);
      else if (k === "s" && !ctrl) setSnap((s) => !s);
      else if (e.key === "Escape") patchDoc(doc.id, { selection: new Set(), range: null });
      else handled = false;
      if (handled) e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // ── context menu ──
  const onContext = (c: TimelineContext) => {
    const items: MenuEntry[] = [];
    if (c.marker !== null) {
      const m = doc.markers[c.marker];
      items.push(
        { title: m.label || t("studio.marker") },
        {
          label: t("common.rename"),
          onClick: async () => {
            const v = await prompt.ask({ title: t("studio.rename_marker"), value: m.label, confirm: t("common.rename"), cancel: t("common.cancel") });
            if (v !== null) setMarkers(doc.markers.map((x, i) => (i === c.marker ? { ...x, label: v } : x)));
          },
        },
        {
          label: t("studio.marker_to_range"),
          disabled: !doc.range,
          onClick: () => doc.range && setMarkers(doc.markers.map((x, i) => (i === c.marker ? { ...x, t: doc.range![0], end: doc.range![1] } : x))),
        },
        { label: t("insp.go_to"), onClick: () => patchDoc(doc.id, { cursor: m.t }) },
        { label: t("common.delete"), danger: true, icon: <Trash2 size={14} />, onClick: () => setMarkers(doc.markers.filter((_, i) => i !== c.marker)) },
      );
      menu.open(c.clientX, c.clientY, items);
      return;
    }
    const sel = useStore.getState().docs[doc.id]?.selection ?? doc.selection;
    if (c.hit || sel.size) {
      const n = sel.size;
      items.push(
        { title: tn("insp.n_events_one", "insp.n_events_many", n) },
        {
          label: t("insp.play_selection"),
          icon: <Play size={14} />,
          onClick: () => playMacro(doc.id, { kind: "ids", ids: [...withPartners(doc.events, sel)] }, false),
        },
        { label: t("common.copy"), icon: <Copy size={14} />, shortcut: "Ctrl+C", onClick: copy },
        { label: t("common.cut"), icon: <Scissors size={14} />, shortcut: "Ctrl+X", onClick: () => copy() && del() },
        { label: t("lib.duplicate"), icon: <Copy size={14} />, shortcut: "Ctrl+D", onClick: duplicate },
        { label: t("insp.extract"), icon: <Scissors size={14} />, shortcut: "Ctrl+E", onClick: extract },
      );
      if (c.lane) {
        const lane = c.lane;
        items.push({
          label: t("studio.select_lane"),
          icon: <Rows3 size={14} />,
          onClick: () => patchDoc(doc.id, { selection: new Set(laneIds(doc.events, model, lane)) }),
        });
      }
      if (c.hit?.hold && c.hit.hold.up >= 0) {
        const hold = c.hit.hold;
        items.push({
          label: t("studio.set_duration"),
          icon: <Timer size={14} />,
          onClick: async () => {
            const v = await prompt.ask({
              title: t("studio.set_duration"),
              value: String((hold.end - hold.start) / 1000),
              confirm: t("common.apply"),
              cancel: t("common.cancel"),
            });
            const ms = v ? parseFloat(v.replace(",", ".")) : NaN;
            if (Number.isFinite(ms) && ms > 0) edit(doc.id, { op: "set_hold_duration", id: doc.events[hold.down].id, duration: msToUs(ms) });
          },
        });
      }
      items.push({ label: t("common.delete"), icon: <Trash2 size={14} />, shortcut: "Del", danger: true, onClick: del }, { separator: true });
    }
    items.push(
      { label: t("studio.paste_here"), icon: <ClipboardPaste size={14} />, disabled: !clipboard, onClick: () => paste(c.t) },
      { label: t("studio.cursor_here"), onClick: () => patchDoc(doc.id, { cursor: c.t }) },
      { label: t("insp.play_from_here"), icon: <SkipForward size={14} />, onClick: () => playMacro(doc.id, { kind: "from", t: c.t }, false) },
      { label: t("studio.add_marker_here"), icon: <Flag size={14} />, shortcut: "M", onClick: () => addMarker(c.t) },
      {
        label: t("insp.insert_time"),
        icon: <Timer size={14} />,
        onClick: async () => {
          const v = await prompt.ask({ title: t("insp.insert_time"), value: "500", confirm: t("common.apply"), cancel: t("common.cancel") });
          const ms = v ? parseFloat(v.replace(",", ".")) : NaN;
          if (Number.isFinite(ms) && ms > 0) edit(doc.id, { op: "insert_time", at: c.t, dt: msToUs(ms) });
        },
      },
    );
    // Quick inserts depending on the lane.
    const L = c.lane;
    if (L) {
      items.push({ separator: true }, { title: t("studio.insert") });
      if (L.kind === "key") {
        const key = model.keyRows[L.row].key;
        items.push({
          label: t("studio.insert_key_here", { key: keyLabel(key) }),
          icon: <Keyboard size={14} />,
          onClick: () => insertItems(keyPress(key, c.t, 60_000)),
        });
      } else if (L.kind === "button") {
        const button = model.buttonRows[L.row].button;
        items.push({
          label: t("studio.insert_click_here", { button: t(`button.${button}`) }),
          icon: <MousePointerClick size={14} />,
          onClick: () => {
            const p = pointerAt(doc.events, c.t) ?? { x: 0, y: 0 };
            insertItems(click(button, p.x, p.y, c.t, 1, 50_000, 120_000));
          },
        });
      } else if (L.kind === "wheel") {
        const p = pointerAt(doc.events, c.t) ?? { x: 0, y: 0 };
        items.push(
          { label: t("studio.insert_wheel_up"), onClick: () => insertItems([[c.t, { type: "wheel", delta: 120, horizontal: false, x: p.x, y: p.y }]]) },
          { label: t("studio.insert_wheel_down"), onClick: () => insertItems([[c.t, { type: "wheel", delta: -120, horizontal: false, x: p.x, y: p.y }]]) },
        );
      }
      items.push(
        { label: t("studio.insert_key"), icon: <Keyboard size={14} />, onClick: () => setInserting("key") },
        { label: t("studio.insert_click"), icon: <MousePointerClick size={14} />, onClick: () => setInserting("click") },
      );
    }
    if (doc.range) {
      const [a, b] = doc.range;
      items.push(
        { separator: true },
        { title: `${t("insp.range")} · ${fmtDuration(b - a)}` },
        { label: t("insp.play_range"), icon: <Play size={14} />, onClick: () => playMacro(doc.id, { kind: "range", start: a, end: b }, false) },
        {
          label: t("insp.cut_range"),
          icon: <Scissors size={14} />,
          danger: true,
          onClick: () => {
            edit(doc.id, { op: "delete_range", start: a, end: b, ripple: true }, { select: "none" });
            patchDoc(doc.id, { range: null, cursor: a });
          },
        },
        { label: t("insp.clear_range"), icon: <Eraser size={14} />, onClick: () => edit(doc.id, { op: "delete_range", start: a, end: b, ripple: false }, { select: "none" }) },
      );
    }
    menu.open(c.clientX, c.clientY, items);
  };

  const insertItems = (items: [Micros, EventKind][]) => {
    edit(doc.id, { op: "insert", items }, { select: "created" });
  };

  const undoLabel = doc.history.undo_label ? t(`op.${doc.history.undo_label}` as TKey) : "";
  const redoLabel = doc.history.redo_label ? t(`op.${doc.history.redo_label}` as TKey) : "";
  const Icon = macroIcon(doc.meta.icon);
  const duration = doc.events.length ? doc.events[doc.events.length - 1].t : 0;
  const playingThis = status.mode === "playing" && status.macro_id === doc.id;

  return (
    <div className="studio">
      <div className="studio-bar">
        <div className="studio-name" style={{ ["--accent-c" as string]: doc.meta.color }}>
          <Icon size={17} />
          <input
            className="name-input"
            value={name}
            onChange={(e) => setName(e.target.value)}
            onBlur={() => rename(name)}
            onKeyDown={(e) => {
              if (e.key === "Enter") (e.target as HTMLInputElement).blur();
              if (e.key === "Escape") {
                setName(doc.meta.name);
                (e.target as HTMLInputElement).blur();
              }
            }}
            spellCheck={false}
          />
          <span className="faint mono">
            {fmtDuration(duration)} · {fmtInt(doc.events.length)}
          </span>
          {doc.dirty ? (
            <button className="btn sm ghost save-btn dirty" onClick={save} title={`${t("studio.save")} (Ctrl+S)`}>
              <Save size={13} /> {t("studio.unsaved")}
            </button>
          ) : (
            <span className="saved-chip">{t("studio.saved")}</span>
          )}
        </div>

        <div className="btn-group">
          <button className="btn sm icon" onClick={() => undo(doc.id)} disabled={!doc.history.can_undo} title={`${t("studio.undo")} ${undoLabel} (Ctrl+Z)`}>
            <Undo2 size={14} />
          </button>
          <button className="btn sm icon" onClick={() => redo(doc.id)} disabled={!doc.history.can_redo} title={`${t("studio.redo")} ${redoLabel} (Ctrl+Y)`}>
            <Redo2 size={14} />
          </button>
        </div>

        <div className="btn-group">
          <button className="btn sm icon" onClick={() => tl.current?.zoomBy(1 / 1.6)} title={`${t("studio.zoom_out")} (−)`}>
            <ZoomOut size={14} />
          </button>
          <button className="btn sm icon" onClick={() => tl.current?.fit()} title={`${t("studio.fit")} (F)`}>
            <Maximize2 size={14} />
          </button>
          <button className="btn sm icon" onClick={() => tl.current?.zoomBy(1.6)} title={`${t("studio.zoom_in")} (+)`}>
            <ZoomIn size={14} />
          </button>
        </div>
        <button className={`btn sm ${snap ? "on" : ""}`} onClick={() => setSnap(!snap)} title={`${t("studio.snap_hint")} (S)`}>
          <Magnet size={14} /> <span className="lbl-wide">{t("studio.snap")}</span>
        </button>
        <button
          className="btn sm"
          title={t("studio.lanes")}
          onClick={(e) => {
            const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
            const entry = (k: keyof LaneVisibility, label: TKey): MenuEntry => ({
              label: `${vis[k] ? "✓ " : "   "}${t(label)}`,
              onClick: () => setVis({ ...vis, [k]: !vis[k] }),
            });
            menu.open(r.left, r.bottom + 4, [
              { title: t("studio.lanes") },
              entry("keyboard", "lane.keyboard"),
              entry("buttons", "lane.buttons"),
              entry("wheel", "lane.wheel"),
              entry("pointer", "lane.pointer"),
              entry("raw", "lane.raw"),
              entry("gamepad", "lane.gamepad"),
              { separator: true },
              { label: `${compact ? "✓ " : "   "}${t("studio.compact")}`, onClick: () => setCompact(!compact) },
            ]);
          }}
        >
          <Eye size={14} /> <span className="lbl-wide">{t("studio.lanes")}</span>
        </button>

        <div className="vdivider" />
        <button
          className="btn sm"
          title={t("studio.smart_tools")}
          onClick={(e) => {
            const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
            menu.open(r.left, r.bottom + 4, [
              { title: t("studio.smart_tools") },
              ...TOOLS.map((tid) => ({ label: t(`tool.${tid}`), icon: <Wand size={14} />, onClick: () => setTool(tid) })),
              { separator: true },
              { label: t("studio.detect_repetitions"), icon: <Repeat size={14} />, onClick: () => setReps(true) },
              { label: t("studio.revert"), icon: <RotateCcw size={14} />, disabled: !doc.has_original, danger: true, onClick: revert },
            ]);
          }}
        >
          <Sparkles size={14} /> <span className="lbl-mid">{t("studio.smart_tools")}</span>
        </button>
        <button
          className="btn sm"
          title={t("studio.insert")}
          onClick={(e) => {
            const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
            menu.open(r.left, r.bottom + 4, [
              { title: `${t("studio.insert_at")} ${fmtTime(doc.cursor, 3)}` },
              { label: t("studio.insert_key"), icon: <Keyboard size={14} />, onClick: () => setInserting("key") },
              { label: t("studio.insert_click"), icon: <MousePointerClick size={14} />, onClick: () => setInserting("click") },
              { label: t("studio.insert_wheel"), onClick: () => setInserting("wheel") },
              { label: t("studio.insert_move"), onClick: () => setInserting("move") },
              { label: t("studio.insert_text"), icon: <Keyboard size={14} />, onClick: () => setInserting("text") },
              { separator: true },
              { label: t("studio.add_marker_here"), icon: <Flag size={14} />, shortcut: "M", onClick: () => addMarker(doc.cursor) },
            ]);
          }}
        >
          <Plus size={14} /> <span className="lbl-mid">{t("studio.insert")}</span>
        </button>

        <div className="spacer" />
        {doc.selection.size > 0 && (
          <span className="chip gold">{tn("insp.n_events_one", "insp.n_events_many", doc.selection.size)}</span>
        )}
        <button
          className="btn sm"
          onClick={() => playMacro(doc.id, { kind: "from", t: doc.cursor }, false)}
          disabled={status.mode !== "idle"}
          title={t("insp.play_from_cursor")}
        >
          <SkipForward size={14} />
        </button>
        <button
          className={`btn sm primary ${playingThis ? "on" : ""}`}
          onClick={() => (playingThis ? api.playStop() : playMacro(doc.id))}
          disabled={status.mode === "recording" || status.mode === "countdown"}
        >
          <Play size={14} fill="currentColor" /> {playingThis ? t("transport.stop") : t("transport.play")}
        </button>
      </div>

      <div className="studio-body">
        <div className="studio-timeline panel">
          <Timeline
            ref={tl}
            doc={doc}
            model={model}
            lanes={lanes}
            contentH={height}
            snap={snap}
            playhead={playhead}
            onContext={onContext}
            onOpenEvent={() => setTab("inspector")}
            onAddMarker={addMarker}
            onMarkers={setMarkers}
          />
          <div className="timeline-foot faint">
            <span>{t("studio.hint_zoom")}</span>
            <span>{t("studio.hint_select")}</span>
            <span>{t("studio.hint_range")}</span>
          </div>
        </div>
        <aside className="studio-side panel">
          <div className="tabs">
            {(["inspector", "playback", "stats", "map", "events"] as Tab[]).map((x) => (
              <button key={x} className={`tab ${tab === x ? "active" : ""}`} onClick={() => setTab(x)}>
                {t(`studio.tab_${x}`)}
              </button>
            ))}
          </div>
          <div className="studio-side-body">
            {tab === "inspector" && <Inspector doc={doc} onExtract={extract} onMarkers={setMarkers} />}
            {tab === "playback" && (
              <PlaybackEditor
                compact
                value={doc.settings}
                canGamepad={caps?.gamepad_output ?? false}
                onChange={(settings) => {
                  patchDoc(doc.id, { settings, dirty: true });
                  api.docSetSettings(doc.id, settings).catch((e) => fail(e, "error.edit"));
                }}
              />
            )}
            {tab === "stats" && <StatsPanel doc={doc} />}
            {tab === "map" && <MapPanel doc={doc} />}
            {tab === "events" && <EventList doc={doc} onReveal={(at) => tl.current?.reveal(at, true)} />}
          </div>
        </aside>
      </div>

      {tool && <ToolModal tool={tool} doc={doc} onClose={() => setTool(null)} />}
      {inserting && <InsertModal kind={inserting} doc={doc} onClose={() => setInserting(null)} onInsert={insertItems} />}
      {reps && <RepetitionsModal doc={doc} onClose={() => setReps(false)} onReveal={(at) => tl.current?.reveal(at, true)} />}
      {menu.element}
      {confirm.element}
      {prompt.element}
    </div>
  );
}

// ───────────────────────────── smart tools ─────────────────────────────

const TOOLS = ["dead_time", "simplify", "merge_raw", "merge_wheel", "cleanup", "uniform", "speed"] as const;
type ToolId = (typeof TOOLS)[number];

function ToolModal({ tool, doc, onClose }: { tool: ToolId; doc: DocState; onClose: () => void }) {
  const edit = useStore((s) => s.edit);
  const toast = useStore((s) => s.toast);
  const [threshold, setThreshold] = useState(1000);
  const [keep, setKeep] = useState(200);
  const [respect, setRespect] = useState(true);
  const [eps, setEps] = useState(2);
  const [maxInterval, setMaxInterval] = useState(250);
  const [interval, setInterval_] = useState(8);
  const [win, setWin] = useState(40);
  const [gap, setGap] = useState(50);
  const [simul, setSimul] = useState(2);
  const [factor, setFactor] = useState(2);
  const [clean, setClean] = useState<CleanupOptions>({ orphan_releases: true, close_stuck: true, strip_repeats: false, redundant_moves: true });
  const scope = doc.selection.size ? [...doc.selection] : null;

  const run = async () => {
    let op: EditOp;
    switch (tool) {
      case "dead_time":
        op = { op: "remove_dead_time", threshold: msToUs(threshold), keep: msToUs(keep), respect_holds: respect };
        break;
      case "simplify":
        op = { op: "simplify_mouse", ids: scope, epsilon: eps, max_interval: msToUs(maxInterval) };
        break;
      case "merge_raw":
        op = { op: "merge_raw_motion", ids: scope, interval: msToUs(interval) };
        break;
      case "merge_wheel":
        op = { op: "merge_wheel", window: msToUs(win) };
        break;
      case "cleanup":
        op = { op: "cleanup", options: clean };
        break;
      case "uniform":
        op = { op: "uniform_timing", gap: msToUs(gap), simultaneous: msToUs(simul) };
        break;
      case "speed":
        op = { op: "speed", factor };
        break;
    }
    const before = doc.events.length;
    const dur0 = doc.events.length ? doc.events[doc.events.length - 1].t : 0;
    const r = await edit(doc.id, op, { select: "none" });
    if (!r) return;
    const after = useStore.getState().docs[doc.id];
    const delta = before - (after?.events.length ?? before);
    const dur1 = after?.events.length ? after.events[after.events.length - 1].t : 0;
    if (!r.removed.length && !r.added.length) toast("info", t("tool.no_change"));
    else if (tool === "cleanup" && r.report) {
      const rep = r.report as Record<string, number>;
      toast(
        "success",
        t("tool.cleanup_report", {
          orphans: rep.orphan_releases ?? 0,
          stuck: rep.closed_stuck ?? 0,
          repeats: rep.repeats ?? 0,
          moves: rep.redundant_moves ?? 0,
        }),
      );
    } else if (tool === "dead_time" || tool === "uniform" || tool === "speed")
      toast("success", t("tool.duration_report", { from: fmtDuration(dur0), to: fmtDuration(dur1) }));
    else toast("success", delta > 0 ? t("tool.removed_report", { n: fmtInt(delta) }) : t("tool.changed_report", { n: fmtInt(r.added.length) }));
    onClose();
  };

  return (
    <Modal
      title={t(`tool.${tool}`)}
      icon={<Wand size={18} />}
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>
            {t("common.cancel")}
          </button>
          <button className="btn primary" onClick={run}>
            {t("common.apply")}
          </button>
        </>
      }
    >
      <p className="muted" style={{ marginBottom: 14, lineHeight: 1.6 }}>
        {t(`tool.${tool}_desc`)}
      </p>
      <div className="col" style={{ gap: 2 }}>
        {tool === "dead_time" && (
          <>
            <FieldLine label={t("tool.threshold")} hint={t("tool.threshold_hint")}>
              <NumberInput value={threshold} min={10} step={100} suffix="ms" width={110} onChange={setThreshold} />
            </FieldLine>
            <FieldLine label={t("tool.keep")} hint={t("tool.keep_hint")}>
              <NumberInput value={keep} min={0} step={50} suffix="ms" width={110} onChange={setKeep} />
            </FieldLine>
            <FieldLine label={t("tool.respect_holds")} hint={t("tool.respect_holds_hint")}>
              <Toggle value={respect} onChange={setRespect} />
            </FieldLine>
          </>
        )}
        {tool === "simplify" && (
          <>
            <FieldLine label={t("tool.epsilon")} hint={t("tool.epsilon_hint")}>
              <NumberInput value={eps} min={0.1} max={100} step={0.5} suffix="px" width={100} onChange={setEps} />
            </FieldLine>
            <FieldLine label={t("tool.max_interval")} hint={t("tool.max_interval_hint")}>
              <NumberInput value={maxInterval} min={0} step={50} suffix="ms" width={110} onChange={setMaxInterval} />
            </FieldLine>
          </>
        )}
        {tool === "merge_raw" && (
          <FieldLine label={t("tool.interval")} hint={t("tool.interval_hint")}>
            <NumberInput value={interval} min={1} step={1} suffix="ms" width={100} onChange={setInterval_} />
          </FieldLine>
        )}
        {tool === "merge_wheel" && (
          <FieldLine label={t("tool.window")} hint={t("tool.window_hint")}>
            <NumberInput value={win} min={1} step={5} suffix="ms" width={100} onChange={setWin} />
          </FieldLine>
        )}
        {tool === "cleanup" && (
          <>
            {(Object.keys(clean) as (keyof CleanupOptions)[]).map((k) => (
              <FieldLine key={k} label={t(`tool.clean_${k}`)} hint={t(`tool.clean_${k}_hint`)}>
                <Toggle value={clean[k]} onChange={(v) => setClean({ ...clean, [k]: v })} />
              </FieldLine>
            ))}
          </>
        )}
        {tool === "uniform" && (
          <>
            <FieldLine label={t("tool.gap")} hint={t("tool.gap_hint")}>
              <NumberInput value={gap} min={0} step={5} suffix="ms" width={100} onChange={setGap} />
            </FieldLine>
            <FieldLine label={t("tool.simultaneous")} hint={t("tool.simultaneous_hint")}>
              <NumberInput value={simul} min={0} step={1} suffix="ms" width={100} onChange={setSimul} />
            </FieldLine>
          </>
        )}
        {tool === "speed" && (
          <FieldLine label={t("tool.factor")} hint={t("tool.factor_hint")}>
            <Segmented
              value={String(factor)}
              onChange={(v) => setFactor(Number(v))}
              options={["0.5", "0.75", "1.25", "1.5", "2", "4"].map((v) => ({ value: v, label: `×${v}` }))}
            />
            <NumberInput value={factor} min={0.01} max={100} step={0.1} suffix="×" width={90} onChange={setFactor} />
          </FieldLine>
        )}
        {(tool === "simplify" || tool === "merge_raw") && (
          <p className="faint" style={{ marginTop: 8 }}>
            {scope ? tn("tool.scope_selection_one", "tool.scope_selection_many", scope.length) : t("tool.scope_all")}
          </p>
        )}
      </div>
    </Modal>
  );
}

function FieldLine({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="field-row">
      <div className="label">
        <span>{label}</span>
        {hint && <span>{hint}</span>}
      </div>
      <div className="row" style={{ justifyContent: "flex-end", gap: 6 }}>
        {children}
      </div>
    </div>
  );
}

// ───────────────────────────── inserting ─────────────────────────────

type InsertKind = "key" | "click" | "wheel" | "move" | "text";

function keyPress(key: Key, at: Micros, hold: Micros): [Micros, EventKind][] {
  return [
    [at, { type: "key_down", key }],
    [at + Math.max(1000, hold), { type: "key_up", key }],
  ];
}

function click(button: MouseButton, x: number, y: number, at: Micros, count: number, hold: Micros, interval: Micros): [Micros, EventKind][] {
  const out: [Micros, EventKind][] = [[at, { type: "mouse_move", x, y }]];
  let t0 = at + 1000;
  for (let i = 0; i < count; i++) {
    out.push([t0, { type: "mouse_down", button, x, y }], [t0 + hold, { type: "mouse_up", button, x, y }]);
    t0 += hold + interval;
  }
  return out;
}

/** Characters typeable with the key table: letters, digits, space and a few symbols (US positions). */
function textToKeys(text: string): { items: (Key & { shift: boolean })[]; skipped: string[] } {
  const items: (Key & { shift: boolean })[] = [];
  const skipped: string[] = [];
  const US: Record<string, [number, number, boolean]> = {
    " ": [0x39, 0x20, false],
    "\n": [0x1c, 0x0d, false],
    "\t": [0x0f, 0x09, false],
    "-": [0x0c, 0xbd, false],
    "=": [0x0d, 0xbb, false],
    ",": [0x33, 0xbc, false],
    ".": [0x34, 0xbe, false],
    "/": [0x35, 0xbf, false],
    ";": [0x27, 0xba, false],
    "'": [0x28, 0xde, false],
    "!": [0x02, 0x31, true],
    "?": [0x35, 0xbf, true],
    ":": [0x27, 0xba, true],
    "_": [0x0c, 0xbd, true],
  };
  const LETTER_SCANS: Record<string, number> = {
    q: 0x10, w: 0x11, e: 0x12, r: 0x13, t: 0x14, y: 0x15, u: 0x16, i: 0x17, o: 0x18, p: 0x19,
    a: 0x1e, s: 0x1f, d: 0x20, f: 0x21, g: 0x22, h: 0x23, j: 0x24, k: 0x25, l: 0x26,
    z: 0x2c, x: 0x2d, c: 0x2e, v: 0x2f, b: 0x30, n: 0x31, m: 0x32,
  };
  const DIGIT_SCANS = [0x0b, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a];
  for (const ch of text) {
    const lower = ch.toLowerCase();
    if (LETTER_SCANS[lower]) items.push({ scan: LETTER_SCANS[lower], vk: lower.toUpperCase().charCodeAt(0), shift: ch !== lower });
    else if (ch >= "0" && ch <= "9") items.push({ scan: DIGIT_SCANS[+ch], vk: ch.charCodeAt(0), shift: false });
    else if (US[ch]) items.push({ scan: US[ch][0], vk: US[ch][1], shift: US[ch][2] });
    else skipped.push(ch);
  }
  return { items, skipped };
}

function InsertModal({
  kind,
  doc,
  onClose,
  onInsert,
}: {
  kind: InsertKind;
  doc: DocState;
  onClose: () => void;
  onInsert: (items: [Micros, EventKind][]) => void;
}) {
  const p0 = pointerAt(doc.events, doc.cursor) ?? { x: 0, y: 0 };
  const [key, setKey] = useState<Key | null>(null);
  // The picker calls onPick then onClose synchronously: a ref sees the pick.
  const pickedRef = useRef(false);
  const [picking, setPicking] = useState(kind === "key");
  const [hold, setHold] = useState(60);
  const [button, setButton] = useState<MouseButton>("left");
  const [x, setX] = useState(p0.x);
  const [y, setY] = useState(p0.y);
  const [count, setCount] = useState(1);
  const [notches, setNotches] = useState(-1);
  const [horizontal, setHorizontal] = useState(false);
  const [text, setText] = useState("");
  const [interval, setInterval_] = useState(40);
  const at = doc.cursor;

  const typed = kind === "text" ? textToKeys(text) : null;
  const insert = () => {
    let items: [Micros, EventKind][] = [];
    if (kind === "key" && key) items = keyPress(key, at, msToUs(hold));
    else if (kind === "click") items = click(button, x, y, at, count, msToUs(hold), msToUs(80));
    else if (kind === "wheel") items = [[at, { type: "wheel", delta: Math.round(notches * 120), horizontal, x: p0.x, y: p0.y }]];
    else if (kind === "move") items = [[at, { type: "mouse_move", x, y }]];
    else if (kind === "text" && typed) {
      const SHIFT: Key = { scan: 0x2a, vk: 0xa0 };
      let tt = at;
      for (const k of typed.items) {
        if (k.shift) items.push([tt, { type: "key_down", key: SHIFT }]);
        items.push([tt + 500, { type: "key_down", key: { scan: k.scan, vk: k.vk } }]);
        items.push([tt + 500 + msToUs(hold), { type: "key_up", key: { scan: k.scan, vk: k.vk } }]);
        if (k.shift) items.push([tt + 1000 + msToUs(hold), { type: "key_up", key: SHIFT }]);
        tt += msToUs(hold) + msToUs(interval);
      }
    }
    if (items.length) onInsert(items);
    onClose();
  };

  if (kind === "key" && picking)
    return (
      <KeyPicker
        onClose={() => (key || pickedRef.current ? setPicking(false) : onClose())}
        onPick={(k) => {
          pickedRef.current = true;
          setKey(k);
          setPicking(false);
        }}
      />
    );

  return (
    <Modal
      title={`${t(`studio.insert_${kind}`)} · ${fmtTime(at, 3)}`}
      icon={<Plus size={18} />}
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>
            {t("common.cancel")}
          </button>
          <button className="btn primary" onClick={insert} disabled={(kind === "key" && !key) || (kind === "text" && !typed?.items.length)}>
            {t("studio.insert")}
          </button>
        </>
      }
    >
      <div className="col" style={{ gap: 2 }}>
        {kind === "key" && (
          <>
            <FieldLine label={t("insp.key")}>
              <button className="btn sm" onClick={() => setPicking(true)}>
                <kbd>{key ? keyLabel(key) : "?"}</kbd> {t("insp.change")}
              </button>
            </FieldLine>
            <FieldLine label={t("insp.duration")}>
              <NumberInput value={hold} min={1} step={10} suffix="ms" width={100} onChange={setHold} />
            </FieldLine>
          </>
        )}
        {kind === "click" && (
          <>
            <FieldLine label={t("insp.button")}>
              <Segmented value={button} onChange={setButton} options={MOUSE_BUTTONS.map((b) => ({ value: b, label: t(`button.${b}`) }))} />
            </FieldLine>
            <FieldLine label="X · Y" hint={t("studio.position_hint")}>
              <NumberInput value={x} step={1} width={80} onChange={(v) => setX(Math.round(v))} />
              <NumberInput value={y} step={1} width={80} onChange={(v) => setY(Math.round(v))} />
            </FieldLine>
            <FieldLine label={t("studio.click_count")}>
              <Segmented
                value={String(count)}
                onChange={(v) => setCount(Number(v))}
                options={[
                  { value: "1", label: t("studio.single") },
                  { value: "2", label: t("studio.double") },
                  { value: "3", label: t("studio.triple") },
                ]}
              />
            </FieldLine>
            <FieldLine label={t("insp.duration")}>
              <NumberInput value={hold} min={1} step={10} suffix="ms" width={100} onChange={setHold} />
            </FieldLine>
          </>
        )}
        {kind === "wheel" && (
          <>
            <FieldLine label={t("insp.notches")} hint={t("studio.notches_hint")}>
              <NumberInput value={notches} step={1} width={90} onChange={setNotches} />
            </FieldLine>
            <FieldLine label={t("insp.direction")}>
              <Toggle value={horizontal} onChange={setHorizontal} label={t("insp.horizontal")} />
              <span className="muted">{horizontal ? t("insp.horizontal") : t("insp.vertical")}</span>
            </FieldLine>
          </>
        )}
        {kind === "move" && (
          <FieldLine label="X · Y">
            <NumberInput value={x} step={1} width={80} onChange={(v) => setX(Math.round(v))} />
            <NumberInput value={y} step={1} width={80} onChange={(v) => setY(Math.round(v))} />
          </FieldLine>
        )}
        {kind === "text" && (
          <>
            <textarea className="textarea" rows={3} value={text} onChange={(e) => setText(e.target.value)} placeholder={t("studio.text_ph")} autoFocus />
            <FieldLine label={t("insp.duration")}>
              <NumberInput value={hold} min={1} step={5} suffix="ms" width={100} onChange={setHold} />
            </FieldLine>
            <FieldLine label={t("tool.interval")}>
              <NumberInput value={interval} min={0} step={5} suffix="ms" width={100} onChange={setInterval_} />
            </FieldLine>
            <p className="faint" style={{ marginTop: 8 }}>
              {t("studio.text_hint")}
              {typed && typed.skipped.length > 0 && (
                <>
                  {" "}
                  <span style={{ color: "var(--orange)" }}>{t("studio.text_skipped", { chars: [...new Set(typed.skipped)].join(" ") })}</span>
                </>
              )}
            </p>
          </>
        )}
      </div>
    </Modal>
  );
}

// ───────────────────────────── repetitions ─────────────────────────────

function RepetitionsModal({ doc, onClose, onReveal }: { doc: DocState; onClose: () => void; onReveal: (t: Micros) => void }) {
  const [list, setList] = useState<Repetition[] | null>(null);
  const [minLen, setMinLen] = useState(3);
  const patchDoc = useStore((s) => s.patchDoc);
  const fail = useStore((s) => s.fail);
  const toast = useStore((s) => s.toast);

  useEffect(() => {
    setList(null);
    api
      .docRepetitions(doc.id, minLen, 12)
      .then(setList)
      .catch((e) => {
        fail(e);
        setList([]);
      });
  }, [doc.id, minLen, doc.version, fail]);

  const selectAll = (r: Repetition) => {
    const ids = new Set<number>();
    for (const [a, b] of r.occurrences) for (const e of doc.events) if (e.t >= a && e.t <= b) ids.add(e.id);
    patchDoc(doc.id, { selection: ids });
    onClose();
  };

  return (
    <Modal title={t("studio.detect_repetitions")} icon={<Repeat size={18} />} onClose={onClose} wide>
      <div className="row" style={{ gap: 10, marginBottom: 12 }}>
        <span className="muted grow">{t("rep.body")}</span>
        <span className="faint">{t("rep.min_len")}</span>
        <NumberInput value={minLen} min={2} max={200} step={1} width={70} onChange={(v) => setMinLen(Math.round(v))} />
      </div>
      {!list ? (
        <div className="empty">
          <div className="spinner" />
        </div>
      ) : list.length === 0 ? (
        <div className="empty">
          <AudioWaveform size={26} className="faint" />
          <h3>{t("rep.none")}</h3>
        </div>
      ) : (
        <table className="table">
          <thead>
            <tr>
              <th>{t("rep.pattern")}</th>
              <th>{t("rep.length")}</th>
              <th>{t("rep.times")}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {list.map((r, i) => (
              <tr key={i}>
                <td className="ellipsis mono" style={{ maxWidth: 360 }} title={r.summary}>
                  {r.summary}
                </td>
                <td className="mono">{r.length}</td>
                <td className="mono">×{r.occurrences.length}</td>
                <td style={{ textAlign: "right", whiteSpace: "nowrap" }}>
                  <button
                    className="btn sm ghost"
                    onClick={() => {
                      const [a, b] = r.occurrences[0];
                      patchDoc(doc.id, { range: [a, b], cursor: a, selection: new Set(r.first_ids) });
                      onReveal(a);
                      onClose();
                    }}
                  >
                    {t("rep.show")}
                  </button>
                  <button className="btn sm ghost" onClick={() => selectAll(r)}>
                    {t("rep.select_all")}
                  </button>
                  <button
                    className="btn sm"
                    onClick={async () => {
                      try {
                        const name = `${doc.meta.name} — ${t("rep.pattern")} ${i + 1}`;
                        const id = await api.docExtract(doc.id, r.first_ids, name);
                        await useStore.getState().refreshLibrary();
                        toast("success", t("toast.extracted", { name }), { label: t("common.open"), run: () => useStore.getState().openDoc(id) });
                      } catch (e) {
                        fail(e, "error.edit");
                      }
                    }}
                  >
                    <Scissors size={12} /> {t("insp.extract")}
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </Modal>
  );
}
