// Flow: visual automation blocks (loops, conditions, variables, nested macros).
import {
  ArrowDown,
  ArrowUp,
  Ban,
  Bell,
  Braces,
  Clapperboard,
  Copy,
  CornerDownRight,
  Dices,
  Eye,
  Focus,
  GitBranch,
  GripVertical,
  Hourglass,
  Keyboard,
  Layers,
  ListRestart,
  MousePointer2,
  MousePointerClick,
  OctagonX,
  Pipette,
  Play,
  Plus,
  Power,
  Repeat,
  Save,
  LayoutList,
  ScanSearch,
  ScanText,
  ScrollText,
  SkipForward,
  Timer,
  Trash2,
  Type,
  Variable,
  Workflow,
  ArrowDownToLine,
  ArrowUpFromLine,
  TriangleAlert,
  type LucideIcon,
} from "lucide-react";
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { KeyPicker } from "../components/KeyPicker";
import { NumberInput, Segmented, Toggle, useConfirm, useContextMenu, usePrompt } from "../components/ui";
import { fmtRelative, t, tn, useLang, type TKey } from "../i18n";
import { playMacro } from "../lib/actions";
import { api } from "../lib/api";
import { fmtTime, msToUs, usToMs } from "../lib/format";
import { keyLabel } from "../lib/keys";
import { useActiveDoc, useStore, type DocState } from "../lib/store";
import type { BlockType, Condition, Flow, FlowNode, FoundAction, IfMissing, Key, Motion, MouseButton, Target } from "../lib/types";
import { MOUSE_BUTTONS } from "../lib/types";
import {
  childList,
  cloneNode,
  countNodes,
  exprFields,
  find,
  insert,
  makeNode,
  makePreset,
  move,
  placeOf,
  remove,
  slotsOf,
  targetIssue,
  targetsOf,
  update,
  variables,
  type Place,
  type Preset,
  type Slot,
} from "../flow/tree";
import { PickButton, RegionEditor, ScreenTestButton, TargetEditor } from "../flow/ScreenTarget";

/** The document being edited, for editors deep in the tree. */
const FlowDoc = createContext<DocState | null>(null);

const PALETTE: { group: TKey; items: { type: BlockType; icon: LucideIcon; preset?: Preset }[] }[] = [
  {
    group: "flow.group_play",
    items: [
      { type: "play_recording", icon: Clapperboard },
      { type: "play_macro", icon: Layers },
    ],
  },
  {
    group: "flow.group_input",
    items: [
      { type: "keys", icon: Keyboard },
      { type: "type_text", icon: Type },
      { type: "click", icon: MousePointerClick },
      { type: "move_mouse", icon: MousePointer2 },
      { type: "scroll", icon: ScrollText },
      { type: "key_down", icon: ArrowDownToLine },
      { type: "key_up", icon: ArrowUpFromLine },
      { type: "mouse_down", icon: ArrowDownToLine },
      { type: "mouse_up", icon: ArrowUpFromLine },
    ],
  },
  {
    group: "flow.group_screen",
    items: [
      { type: "find", icon: ScanSearch, preset: "click_image" },
      { type: "find", icon: ScanText, preset: "click_text" },
      { type: "wait_until", icon: Eye, preset: "wait_image" },
      { type: "if", icon: Focus, preset: "if_image" },
      { type: "find", icon: Pipette, preset: "find_color" },
    ],
  },
  {
    group: "flow.group_time",
    items: [
      { type: "wait", icon: Timer },
      { type: "wait_until", icon: Hourglass },
    ],
  },
  {
    group: "flow.group_logic",
    items: [
      { type: "repeat", icon: Repeat },
      { type: "while", icon: ListRestart },
      { type: "if", icon: GitBranch },
      { type: "break", icon: Ban },
      { type: "continue", icon: SkipForward },
      { type: "stop", icon: OctagonX },
    ],
  },
  {
    group: "flow.group_data",
    items: [
      { type: "set_var", icon: Variable },
      { type: "notify", icon: Bell },
    ],
  },
];

const ICONS: Record<BlockType, LucideIcon> = Object.fromEntries(
  PALETTE.flatMap((g) => g.items.filter((i) => !i.preset).map((i) => [i.type, i.icon])).concat([["find", ScanSearch]]),
) as Record<BlockType, LucideIcon>;

const TARGET_ICONS: Record<Target["kind"], LucideIcon> = { image: ScanSearch, text: ScanText, color: Pipette };

function iconOf(n: FlowNode): LucideIcon {
  return n.type === "find" ? TARGET_ICONS[n.target.kind] : ICONS[n.type];
}

function titleOf(n: FlowNode): string {
  return n.type === "find" ? t(`block.find_${n.target.kind}` as TKey) : blockTitle(n.type);
}

/** The image a block looks for, to show on its card. */
function thumbOf(n: FlowNode, doc: DocState): string | null {
  const tg = n.type === "find" ? n.target : (n.type === "if" || n.type === "while" || n.type === "wait_until") && n.cond.type === "on_screen" ? n.cond.target : null;
  if (!tg || tg.kind !== "image") return null;
  return doc.images.find((i) => i.id === tg.image)?.url ?? null;
}

const FOUND_ACTIONS: FoundAction[] = ["click", "double_click", "right_click", "middle_click", "move", "none"];

const TONE: Record<BlockType, string> = {
  play_recording: "gold",
  play_macro: "gold",
  keys: "sky",
  type_text: "sky",
  click: "cyan",
  move_mouse: "cyan",
  scroll: "cyan",
  key_down: "sky",
  key_up: "sky",
  mouse_down: "cyan",
  mouse_up: "cyan",
  wait: "mint",
  wait_until: "mint",
  repeat: "violet",
  while: "violet",
  if: "pink",
  break: "pink",
  continue: "pink",
  stop: "rec",
  set_var: "orange",
  notify: "orange",
  find: "lime",
};

function blockTitle(type: BlockType): string {
  return t(`block.${type}` as TKey);
}

export function FlowView() {
  useLang();
  const doc = useActiveDoc();
  if (!doc) return <FlowHub />;
  return <FlowEditor key={doc.id} doc={doc} />;
}

/** Library macros that have a flow, most recently modified first. */
function useSavedFlows(exceptId?: string) {
  const listing = useStore((s) => s.listing);
  return useMemo(
    () =>
      (listing?.entries ?? [])
        .filter((e) => e.manifest.summary.has_flow && e.id !== exceptId)
        .sort((a, b) => b.manifest.meta.modified_ms - a.manifest.meta.modified_ms),
    [listing, exceptId],
  );
}

/** Opens a macro straight in the Flow view. */
async function openInFlow(id: string) {
  const s = useStore.getState();
  await s.openDoc(id, false);
  s.setView("flow");
}

/** Creates a new macro holding an empty flow and opens it. */
async function createFlow(name: string) {
  const s = useStore.getState();
  try {
    const id = await api.docNew(name, s.libFolder);
    await s.refreshLibrary();
    await openInFlow(id);
    const flow: Flow = { blocks: [] };
    await api.docSetFlow(id, flow);
    s.patchDoc(id, { flow, dirty: true });
  } catch (e) {
    s.fail(e, "error.library");
  }
}

/** Flow home when no macro is open: the saved flows and a way to start one. */
function FlowHub() {
  const flows = useSavedFlows();
  const prompt = usePrompt();
  useEffect(() => {
    useStore.getState().refreshLibrary();
  }, []);
  const newFlow = async () => {
    const name = await prompt.ask({ title: t("flow.new"), value: t("flow.new_name"), confirm: t("common.create"), cancel: t("common.cancel") });
    if (name?.trim()) await createFlow(name.trim());
  };
  return (
    <div className="flow-hub">
      <div className="flow-hub-head">
        <Workflow size={36} className="gold-icon" />
        <div className="grow">
          <h2>{t("flow.hub_title")}</h2>
          <p className="muted">{t("flow.hub_body")}</p>
        </div>
        <button className="btn primary" onClick={newFlow}>
          <Plus size={14} /> {t("flow.new")}
        </button>
      </div>
      {flows.length ? (
        <div className="flow-hub-list">
          {flows.map((e) => (
            <button key={e.id} className="flow-hub-item panel" onClick={() => openInFlow(e.id)} title={t("flow.open")}>
              <Workflow size={16} style={{ color: e.manifest.meta.color || undefined }} />
              <span className="grow ellipsis">
                <b>{e.manifest.meta.name}</b>
                {e.manifest.meta.description && <span className="faint ellipsis flow-hub-desc">{e.manifest.meta.description}</span>}
              </span>
              {e.folder && <span className="chip">{e.folder}</span>}
              <span className="faint">{fmtRelative(e.manifest.meta.modified_ms)}</span>
            </button>
          ))}
        </div>
      ) : (
        <p className="muted flow-hub-empty">{t("flow.hub_empty")}</p>
      )}
      {prompt.element}
    </div>
  );
}

function FlowEditor({ doc }: { doc: DocState }) {
  const patchDoc = useStore((s) => s.patchDoc);
  const fail = useStore((s) => s.fail);
  const status = useStore((s) => s.status);
  const caps = useStore((s) => s.info?.capabilities);
  const [selected, setSelected] = useState<string | null>(null);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const confirm = useConfirm();
  const blocks = doc.flow?.blocks ?? [];
  const running = status.mode === "playing" && status.macro_id === doc.id ? status.block : "";

  const setFlow = useCallback(
    (next: FlowNode[] | null) => {
      const flow: Flow | null = next === null ? null : { blocks: next };
      patchDoc(doc.id, { flow, dirty: true });
      api.docSetFlow(doc.id, flow).catch((e) => fail(e, "error.edit"));
    },
    [doc.id, patchDoc, fail],
  );

  // ── saving and reusing flows ──
  const toast = useStore((s) => s.toast);
  const menu = useContextMenu();
  const others = useSavedFlows(doc.id);
  const save = useCallback(async () => {
    try {
      const r = await api.docSave(doc.id);
      if (r) {
        const d = useStore.getState().docs[doc.id];
        patchDoc(doc.id, { dirty: false, history: r.history, ...(d ? { meta: { ...d.meta, modified_ms: r.modified_ms } } : {}) });
      } else {
        // Already saved (autosave got there first): just clear the flag.
        patchDoc(doc.id, { dirty: false });
      }
      useStore.getState().refreshLibrary();
    } catch (e) {
      fail(e, "error.save");
    }
  }, [doc.id, patchDoc, fail]);
  useEffect(() => {
    const k = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void save();
      }
    };
    window.addEventListener("keydown", k);
    return () => window.removeEventListener("keydown", k);
  }, [save]);
  const reuseFrom = async (fromId: string, name: string) => {
    if ((useStore.getState().docs[doc.id]?.flow?.blocks.length ?? 0) > 0) {
      const ok = await confirm.ask({
        title: t("flow.reuse_title"),
        body: t("flow.reuse_body", { name }),
        confirm: t("flow.reuse_confirm"),
        cancel: t("common.cancel"),
      });
      if (!ok) return;
    }
    try {
      const p = await api.docImportFlow(doc.id, fromId);
      patchDoc(doc.id, { flow: p.flow, images: p.images, dirty: true });
      setSelected(null);
      toast("success", t("flow.reused", { name }));
    } catch (e) {
      fail(e, "error.edit");
    }
  };
  const openReuseMenu = (el: HTMLElement) => {
    const r = el.getBoundingClientRect();
    menu.open(r.left, r.bottom + 4, [
      { title: t("flow.reuse_menu") },
      ...(others.length
        ? others.slice(0, 30).map((e) => ({
            label: e.folder ? `${e.manifest.meta.name} · ${e.folder}` : e.manifest.meta.name,
            icon: <Workflow size={13} />,
            onClick: () => void reuseFrom(e.id, e.manifest.meta.name),
          }))
        : [{ label: t("flow.reuse_none"), disabled: true }]),
    ]);
  };

  // Validate expressions (debounced, through the core parser).
  useEffect(() => {
    const id = window.setTimeout(async () => {
      const out: Record<string, string> = {};
      const images = new Set(doc.images.map((i) => i.id));
      const walk = async (l: FlowNode[]) => {
        for (const n of l) {
          for (const tg of targetsOf(n)) {
            const issue = targetIssue(tg, images);
            if (issue) {
              out[n.id] = t(`screen.issue_${issue}` as TKey);
              break;
            }
          }
          for (const src of out[n.id] ? [] : exprFields(n)) {
            const err = src.trim() ? await api.exprCheck(src).catch(() => null) : t("flow.empty_expr");
            if (err) {
              out[n.id] = err;
              break;
            }
          }
          for (const s of slotsOf(n)) await walk(childList(n, s));
        }
      };
      await walk(blocks);
      setErrors(out);
    }, 250);
    return () => window.clearTimeout(id);
  }, [blocks, doc.images]);

  const add = (type: BlockType, preset?: Preset) => {
    const node = preset ? makePreset(preset) : makeNode(type);
    let place: Place = { parent: null, slot: "root", index: blocks.length };
    if (selected) {
      const sel = find(blocks, selected);
      if (sel && slotsOf(sel).length) place = { parent: sel.id, slot: slotsOf(sel)[0], index: childList(sel, slotsOf(sel)[0]).length };
      else {
        const p = placeOf(blocks, selected);
        if (p) place = { ...p, index: p.index + 1 };
      }
    }
    setFlow(insert(blocks, place, [node]));
    setSelected(node.id);
  };

  const patch = (id: string, p: Partial<FlowNode>) => setFlow(update(blocks, id, p));
  const del = (id: string) => {
    setFlow(remove(blocks, id));
    if (selected === id) setSelected(null);
  };
  const dup = (id: string) => {
    const n = find(blocks, id);
    const p = placeOf(blocks, id);
    if (!n || !p) return;
    const c = cloneNode(n);
    setFlow(insert(blocks, { ...p, index: p.index + 1 }, [c]));
    setSelected(c.id);
  };
  const shiftBy = (id: string, d: -1 | 1) => {
    const p = placeOf(blocks, id);
    if (!p) return;
    const target = p.index + (d > 0 ? 2 : -1);
    if (target < 0) return;
    setFlow(move(blocks, id, { ...p, index: target }));
  };

  // ── pointer drag & drop ──
  const [drag, setDrag] = useState<{ id: string; x: number; y: number; label: string } | null>(null);
  const [drop, setDrop] = useState<Place | null>(null);
  const dropRef = useRef<Place | null>(null);
  const startDrag = (e: React.MouseEvent, n: FlowNode) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const x0 = e.clientX;
    const y0 = e.clientY;
    let active = false;
    const mv = (ev: MouseEvent) => {
      if (!active && Math.hypot(ev.clientX - x0, ev.clientY - y0) < 5) return;
      active = true;
      setDrag({ id: n.id, x: ev.clientX, y: ev.clientY, label: blockTitle(n.type) });
      const el = document.elementFromPoint(ev.clientX, ev.clientY) as HTMLElement | null;
      const zone = el?.closest<HTMLElement>("[data-drop-parent]") ?? null;
      const blockEl = el?.closest<HTMLElement>("[data-block-id]") ?? null;
      let place: Place | null = null;
      if (zone) {
        const parent = zone.dataset.dropParent || null;
        const slot = zone.dataset.dropSlot as Slot;
        if (blockEl && zone.contains(blockEl)) {
          // Over a block of this list: before it, after it, or into its first slot.
          const head = blockEl.querySelector<HTMLElement>(":scope > .block-head") ?? blockEl;
          const r = head.getBoundingClientRect();
          const idx = Number(blockEl.dataset.index);
          const inner = blockEl.querySelector<HTMLElement>(":scope > .block-slot > [data-drop-parent]");
          if (ev.clientY < r.top + r.height / 2) place = { parent, slot, index: idx };
          else if (inner) place = { parent: inner.dataset.dropParent || null, slot: inner.dataset.dropSlot as Slot, index: 0 };
          else place = { parent, slot, index: idx + 1 };
        } else {
          place = { parent, slot, index: Number(zone.dataset.count ?? 0) };
        }
      }
      dropRef.current = place;
      setDrop(place);
    };
    const up = () => {
      window.removeEventListener("mousemove", mv);
      window.removeEventListener("mouseup", up);
      if (active && dropRef.current) setFlow(move(useStore.getState().docs[doc.id]?.flow?.blocks ?? [], n.id, dropRef.current));
      setDrag(null);
      setDrop(null);
      dropRef.current = null;
    };
    window.addEventListener("mousemove", mv);
    window.addEventListener("mouseup", up);
  };

  // Keyboard on selected block.
  useEffect(() => {
    const k = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
      if (document.querySelector(".modal-backdrop, .menu, .picker")) return;
      if (!selected) return;
      if (e.key === "Delete") del(selected);
      else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "d") {
        e.preventDefault();
        dup(selected);
      } else if (e.altKey && e.key === "ArrowUp") shiftBy(selected, -1);
      else if (e.altKey && e.key === "ArrowDown") shiftBy(selected, 1);
      else if (e.key === "Escape") setSelected(null);
    };
    window.addEventListener("keydown", k);
    return () => window.removeEventListener("keydown", k);
  });

  const sel = selected ? find(blocks, selected) : null;
  const vars = useMemo(() => variables(blocks), [blocks]);
  const errCount = Object.keys(errors).length;
  const playing = status.mode === "playing" && status.macro_id === doc.id;

  if (!doc.flow)
    return (
      <div className="flow-intro">
        <div className="flow-intro-card panel">
          <Workflow size={40} className="gold-icon" />
          <h2>{t("flow.intro_title")}</h2>
          <p className="muted">{t("flow.intro_body")}</p>
          <ul className="flow-intro-list">
            <li>
              <Repeat size={14} /> {t("flow.intro_loops")}
            </li>
            <li>
              <ScanSearch size={14} /> {t("flow.intro_screen")}
            </li>
            <li>
              <GitBranch size={14} /> {t("flow.intro_conditions")}
            </li>
            <li>
              <Braces size={14} /> {t("flow.intro_vars")}
            </li>
            <li>
              <Layers size={14} /> {t("flow.intro_nested")}
            </li>
          </ul>
          <div className="row" style={{ gap: 10, justifyContent: "center" }}>
            <button
              className="btn primary"
              onClick={() => {
                const rep = makeNode("repeat") as Extract<FlowNode, { type: "repeat" }>;
                setFlow([{ ...rep, body: [makeNode("play_recording"), { ...makeNode("wait"), ms: "random(300, 800)" } as FlowNode] }]);
              }}
            >
              <Plus size={14} /> {t("flow.create")}
            </button>
            <button className="btn" onClick={() => setFlow([])}>
              {t("flow.create_empty")}
            </button>
            <button className="btn" onClick={(e) => openReuseMenu(e.currentTarget)} disabled={!others.length} title={t("flow.reuse_hint")}>
              <Copy size={14} /> {t("flow.reuse")}
            </button>
          </div>
        </div>
        {menu.element}
        {confirm.element}
      </div>
    );

  return (
    <div className="flow">
      <aside className="flow-palette panel">
        <div className="eyebrow" style={{ padding: "12px 14px 4px" }}>
          {t("flow.blocks")}
        </div>
        <div className="palette-scroll">
          {PALETTE.map((g) => (
            <div key={g.group} className="palette-group">
              <div className="palette-title">{t(g.group)}</div>
              {g.items.map((it) => {
                const label = it.preset ? t(`preset.${it.preset}` as TKey) : blockTitle(it.type);
                const hint = it.preset ? t(`preset.${it.preset}_hint` as TKey) : t(`block.${it.type}_hint` as TKey);
                return (
                <button key={it.preset ?? it.type} className={`palette-item tone-${it.preset ? "lime" : TONE[it.type]}`} onClick={() => add(it.type, it.preset)} title={`${label} — ${hint}`} aria-label={label}>
                  <it.icon size={14} />
                  <span>{label}</span>
                  <Plus size={12} className="palette-plus" />
                </button>
                );
              })}
            </div>
          ))}
        </div>
        <p className="faint palette-hint">{selected ? t("flow.palette_hint_selected") : t("flow.palette_hint")}</p>
      </aside>

      <section className="flow-main">
        <div className="flow-bar">
          <button className="btn sm icon ghost" onClick={() => useStore.setState({ activeDoc: null })} title={t("flow.hub_title")} aria-label={t("flow.hub_title")}>
            <LayoutList size={15} />
          </button>
          <h2 className="grow flow-title" title={doc.meta.name}>
            {t("nav.flow")} <span className="faint">· {doc.meta.name}</span>
          </h2>
          {doc.dirty ? (
            <button className="btn sm ghost save-btn dirty" onClick={save} title={`${t("studio.save")} (Ctrl+S)`}>
              <Save size={13} /> {t("studio.unsaved")}
            </button>
          ) : (
            <span className="saved-chip" title={t("flow.saved_hint")}>
              {t("studio.saved")}
            </span>
          )}
          <span className="chip">{tn("flow.n_blocks_one", "flow.n_blocks_many", countNodes(blocks))}</span>
          {errCount > 0 && (
            <span className="chip rec">
              <TriangleAlert size={11} /> {tn("flow.n_errors_one", "flow.n_errors_many", errCount)}
            </span>
          )}
          <button className="btn sm ghost" onClick={(e) => openReuseMenu(e.currentTarget)} title={t("flow.reuse_hint")}>
            <Copy size={13} /> <span className="lbl-mid">{t("flow.reuse")}</span>
          </button>
          <button
            className="btn sm ghost"
            title={t("flow.remove")}
            onClick={async () => {
              const ok = await confirm.ask({
                title: t("flow.remove_title"),
                body: t("flow.remove_body"),
                confirm: t("flow.remove"),
                cancel: t("common.cancel"),
                danger: true,
              });
              if (ok) {
                setFlow(null);
                setSelected(null);
              }
            }}
          >
            <Trash2 size={13} /> <span className="lbl-mid">{t("flow.remove")}</span>
          </button>
          <button
            className={`btn sm primary ${playing ? "on" : ""}`}
            disabled={errCount > 0 || !blocks.length || status.mode === "recording"}
            onClick={() => (playing ? api.playStop() : playMacro(doc.id, undefined, true))}
            title={errCount ? t("flow.fix_errors") : ""}
          >
            <Play size={14} fill="currentColor" /> {playing ? t("transport.stop") : t("flow.run")}
          </button>
        </div>
        <div className="flow-canvas" onMouseDown={(e) => e.target === e.currentTarget && setSelected(null)}>
          <div className="flow-start">
            <Power size={13} /> {t("flow.start")}
            <span className="faint">
              · {doc.settings.repeat === 0 ? t("flow.runs_forever") : tn("flow.runs_one", "flow.runs_many", doc.settings.repeat)}
            </span>
          </div>
          <BlockList
            list={blocks}
            parent={null}
            slot="root"
            depth={0}
            ctx={{ selected, setSelected, errors, running, patch, del, dup, shiftBy, startDrag, drop, dragId: drag?.id ?? null, doc }}
          />
          <div className="flow-end">
            <OctagonX size={13} /> {t("flow.end")}
          </div>
        </div>
      </section>

      <aside className="flow-props panel">
        <FlowDoc.Provider value={doc}>
        {sel ? (
          <BlockProps node={sel} doc={doc} error={errors[sel.id]} onPatch={(p) => patch(sel.id, p)} vars={vars} canPixel={caps?.pixel_read ?? false} canWindow={caps?.foreground_window ?? false} />
        ) : (
          <div className="flow-help">
            <div className="eyebrow">{t("flow.help")}</div>
            <p className="muted">{t("flow.help_body")}</p>
            <div className="eyebrow" style={{ marginTop: 16 }}>
              {t("flow.variables")}
            </div>
            {vars.length ? (
              <div className="wrap row" style={{ gap: 6 }}>
                {vars.map((v) => (
                  <code key={v} className="var-chip">
                    {v}
                  </code>
                ))}
              </div>
            ) : (
              <p className="faint">{t("flow.no_vars")}</p>
            )}
            <div className="eyebrow" style={{ marginTop: 16 }}>
              {t("flow.expressions")}
            </div>
            <div className="expr-help mono">
              <div>random(100, 300)</div>
              <div>i * 50 + 10</div>
              <div>i % 2 == 0 && !done</div>
              <div>min(a, b) · max · abs · round · floor · ceil · clamp(v, lo, hi) · rand()</div>
              <div>"run " + i</div>
            </div>
          </div>
        )}
        </FlowDoc.Provider>
      </aside>
      {drag && (
        <div className="drag-ghost" style={{ left: drag.x + 14, top: drag.y + 10 }}>
          <GripVertical size={13} /> {drag.label}
        </div>
      )}
      {confirm.element}
      {menu.element}
    </div>
  );
}

interface Ctx {
  selected: string | null;
  setSelected: (id: string | null) => void;
  errors: Record<string, string>;
  running: string;
  patch: (id: string, p: Partial<FlowNode>) => void;
  del: (id: string) => void;
  dup: (id: string) => void;
  shiftBy: (id: string, d: -1 | 1) => void;
  startDrag: (e: React.MouseEvent, n: FlowNode) => void;
  drop: Place | null;
  dragId: string | null;
  doc: DocState;
}

function BlockList({ list, parent, slot, depth, ctx }: { list: FlowNode[]; parent: string | null; slot: Slot; depth: number; ctx: Ctx }) {
  const isDrop = (i: number) => ctx.drop && ctx.drop.parent === parent && ctx.drop.slot === slot && ctx.drop.index === i;
  return (
    <div className={`block-list ${list.length ? "" : "empty"}`} data-drop-parent={parent ?? ""} data-drop-slot={slot} data-count={list.length}>
      {list.map((n, i) => (
        <div key={n.id}>
          {isDrop(i) && <div className="drop-line" />}
          <BlockCard node={n} index={i} depth={depth} ctx={ctx} />
        </div>
      ))}
      {isDrop(list.length) && <div className="drop-line" />}
      {!list.length && <div className="block-empty faint">{t("flow.drop_here")}</div>}
    </div>
  );
}

function BlockCard({ node, index, depth, ctx }: { node: FlowNode; index: number; depth: number; ctx: Ctx }) {
  const Icon = iconOf(node);
  const selected = ctx.selected === node.id;
  const err = ctx.errors[node.id];
  const slots = slotsOf(node);
  const thumb = thumbOf(node, ctx.doc);
  return (
    <div
      className={`block tone-${TONE[node.type]} ${selected ? "selected" : ""} ${node.enabled ? "" : "disabled"} ${ctx.running === node.id ? "running" : ""} ${
        ctx.dragId === node.id ? "dragging" : ""
      }`}
      data-block-id={node.id}
      data-index={index}
      onMouseDown={(e) => {
        e.stopPropagation();
        ctx.setSelected(node.id);
      }}
    >
      <div className={`block-head ${thumb ? "has-thumb" : ""}`}>
        <span className="grip" onMouseDown={(e) => ctx.startDrag(e, node)} title={t("flow.drag")}>
          <GripVertical size={13} />
        </span>
        <span className="block-icon">
          <Icon size={14} />
        </span>
        <span className="block-title">{titleOf(node)}</span>
        {thumb && <img className="block-thumb" src={thumb} alt="" />}
        {node.type === "find" && node.target.kind === "color" && <span className="block-swatch" style={{ background: node.target.color }} />}
        <span className="block-summary ellipsis">{summary(node, ctx.doc)}</span>
        {err && (
          <span className="block-err" title={err}>
            <TriangleAlert size={13} />
          </span>
        )}
        <div className="block-tools">
          <button onClick={() => ctx.shiftBy(node.id, -1)} title={`${t("flow.move_up")} (Alt+↑)`}>
            <ArrowUp size={12} />
          </button>
          <button onClick={() => ctx.shiftBy(node.id, 1)} title={`${t("flow.move_down")} (Alt+↓)`}>
            <ArrowDown size={12} />
          </button>
          <button onClick={() => ctx.dup(node.id)} title={`${t("lib.duplicate")} (Ctrl+D)`}>
            <Copy size={12} />
          </button>
          <button onClick={() => ctx.patch(node.id, { enabled: !node.enabled })} title={node.enabled ? t("flow.disable") : t("flow.enable")}>
            <Power size={12} />
          </button>
          <button className="danger" onClick={() => ctx.del(node.id)} title={`${t("common.delete")} (Del)`}>
            <Trash2 size={12} />
          </button>
        </div>
      </div>
      {slots.map((s) => (
        <div key={s} className="block-slot">
          {node.type === "if" && (
            <div className="slot-label">
              <CornerDownRight size={11} /> {s === "then" ? t("flow.then") : t("flow.otherwise")}
            </div>
          )}
          <BlockList list={childList(node, s)} parent={node.id} slot={s} depth={depth + 1} ctx={ctx} />
        </div>
      ))}
    </div>
  );
}

function condSummary(c: Condition): string {
  switch (c.type) {
    case "expr":
      return c.expr || "…";
    case "window_active":
      return `${t("cond.window_active")} ${c.process || c.title || "…"}`;
    case "window_exists":
      return `${t("cond.window_exists")} ${c.process || c.title || "…"}`;
    case "pixel_color":
      return `${t("cond.pixel")} (${c.x}, ${c.y}) ≈ ${c.color}`;
    case "key_held":
      return `${keyLabel(c.key)} ${t("cond.held")}`;
    case "on_screen":
      return `${targetText(c.target)} ${t("cond.visible")}`;
    case "chance":
      return `${c.percent}%`;
    case "not":
      return `${t("cond.not")} (${condSummary(c.cond)})`;
    case "all":
      return c.conds.map(condSummary).join(` ${t("cond.and")} `) || "…";
    case "any":
      return c.conds.map(condSummary).join(` ${t("cond.or")} `) || "…";
  }
}

function targetText(tg: Target): string {
  switch (tg.kind) {
    case "image":
      return tg.image ? t("screen.kind_image").toLowerCase() : `${t("screen.kind_image").toLowerCase()} …`;
    case "text":
      return tg.text ? `“${tg.text.length > 30 ? `${tg.text.slice(0, 30)}…` : tg.text}”` : "“…”";
    case "color":
      return tg.color;
  }
}

function summary(n: FlowNode, doc: DocState): string {
  switch (n.type) {
    case "play_recording":
      return n.start === null && n.end === null
        ? `${t("flow.whole_recording")}${n.speed !== 1 ? ` ×${n.speed}` : ""}`
        : `${fmtTime(n.start ?? 0, 2)} → ${n.end === null ? t("flow.end_short") : fmtTime(n.end, 2)}${n.speed !== 1 ? ` ×${n.speed}` : ""}`;
    case "play_macro":
      return n.name || t("flow.pick_macro");
    case "wait":
      return `${n.ms} ms`;
    case "keys":
      return n.keys.length ? n.keys.map(keyLabel).join(" + ") : t("flow.no_keys");
    case "key_down":
    case "key_up":
      return keyLabel(n.key);
    case "type_text":
      return n.text ? `“${n.text.length > 40 ? `${n.text.slice(0, 40)}…` : n.text}”` : "…";
    case "move_mouse":
      return `${n.relative ? "Δ " : ""}(${n.x}, ${n.y}) · ${n.duration_ms} ms`;
    case "click":
      return `${t(`button.${n.button}`)}${n.count > 1 ? ` ×${n.count}` : ""}`;
    case "mouse_down":
    case "mouse_up":
      return t(`button.${n.button}`);
    case "scroll":
      return `${n.notches > 0 ? "+" : ""}${n.notches} ${n.horizontal ? "⇆" : "⇅"}`;
    case "repeat":
      return n.count === null ? t("flow.forever") : `${n.count} × · ${n.counter}`;
    case "while":
      return condSummary(n.cond);
    case "if":
      return condSummary(n.cond);
    case "wait_until":
      return `${condSummary(n.cond)}${n.timeout_ms ? ` · ≤ ${n.timeout_ms} ms` : ""}`;
    case "find": {
      const parts = [t(`found.${n.action}`)];
      if (n.target.kind === "text") parts.push(targetText(n.target));
      if (n.timeout_ms > 0) parts.push(`≤ ${n.timeout_ms >= 1000 ? `${+(n.timeout_ms / 1000).toFixed(1)} s` : `${n.timeout_ms} ms`}`);
      if (n.region) parts.push(t("screen.in_area").toLowerCase());
      return parts.join(" · ");
    }
    case "set_var":
      return `${n.name} = ${n.value}`;
    case "notify":
      return n.message || "…";
    default:
      void doc;
      return "";
  }
}

// ───────────────────────────── properties ─────────────────────────────

function Prop({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="prop">
      <label className="prop-label">{label}</label>
      {children}
      {hint && <div className="prop-hint">{hint}</div>}
    </div>
  );
}

function ExprInput({ value, onChange, placeholder }: { value: string; onChange: (v: string) => void; placeholder?: string }) {
  const [text, setText] = useState(value);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => setText(value), [value]);
  useEffect(() => {
    const id = window.setTimeout(() => {
      if (!text.trim()) return setErr(t("flow.empty_expr"));
      api
        .exprCheck(text)
        .then(setErr)
        .catch(() => setErr(null));
    }, 180);
    return () => window.clearTimeout(id);
  }, [text]);
  return (
    <>
      <input
        className={`input mono ${err ? "invalid" : ""}`}
        value={text}
        placeholder={placeholder}
        spellCheck={false}
        onChange={(e) => setText(e.target.value)}
        onBlur={() => text !== value && onChange(text)}
        onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
      />
      {err && <div className="prop-error">{err}</div>}
    </>
  );
}

function TextInput({ value, onChange, placeholder, area }: { value: string; onChange: (v: string) => void; placeholder?: string; area?: boolean }) {
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  return area ? (
    <textarea className="textarea" rows={3} value={text} placeholder={placeholder} onChange={(e) => setText(e.target.value)} onBlur={() => text !== value && onChange(text)} />
  ) : (
    <input
      className="input"
      value={text}
      placeholder={placeholder}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => text !== value && onChange(text)}
      onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
    />
  );
}

function ButtonSelect({ value, onChange }: { value: MouseButton; onChange: (b: MouseButton) => void }) {
  return <Segmented value={value} onChange={onChange} options={MOUSE_BUTTONS.map((b) => ({ value: b, label: t(`button.${b}`) }))} />;
}

function BlockProps({
  node,
  doc,
  error,
  onPatch,
  vars,
  canPixel,
  canWindow,
}: {
  node: FlowNode;
  doc: DocState;
  error?: string;
  onPatch: (p: Partial<FlowNode>) => void;
  vars: string[];
  canPixel: boolean;
  canWindow: boolean;
}) {
  const listing = useStore((s) => s.listing);
  const [picking, setPicking] = useState<null | "key" | "keys">(null);
  const Icon = iconOf(node);
  const P = onPatch as (p: Record<string, unknown>) => void;
  const duration = doc.events.length ? doc.events[doc.events.length - 1].t : 0;

  return (
    <div className="block-props">
      <div className={`props-head tone-${TONE[node.type]}`}>
        <span className="block-icon">
          <Icon size={16} />
        </span>
        <div className="grow">
          <h3>{titleOf(node)}</h3>
          <div className="faint">{t(`block.${node.type}_hint` as TKey)}</div>
        </div>
        <Toggle value={node.enabled} onChange={(v) => onPatch({ enabled: v })} label={t("flow.enabled")} />
      </div>
      {error && <div className="prop-error block">{error}</div>}

      {node.type === "play_recording" && (
        <>
          <Prop label={t("flow.segment")} hint={t("flow.segment_hint", { d: fmtTime(duration, 2) })}>
            <div className="row" style={{ gap: 6 }}>
              <NumberInput value={node.start === null ? null : usToMs(node.start)} placeholder="0" min={0} step={100} suffix="ms" width={110} onChange={(v) => P({ start: msToUs(v) })} />
              <span className="faint">→</span>
              <NumberInput value={node.end === null ? null : usToMs(node.end)} placeholder={t("flow.end_short")} min={0} step={100} suffix="ms" width={110} onChange={(v) => P({ end: msToUs(v) })} />
            </div>
            <div className="row" style={{ gap: 6, marginTop: 6 }}>
              <button className="btn sm" disabled={!doc.range} onClick={() => doc.range && P({ start: doc.range[0], end: doc.range[1] })}>
                {t("flow.use_range")}
              </button>
              <button className="btn sm ghost" onClick={() => P({ start: null, end: null })}>
                {t("flow.whole_recording")}
              </button>
            </div>
          </Prop>
          <Prop label={t("pb.speed")}>
            <NumberInput value={node.speed} min={0.05} max={50} step={0.25} suffix="×" width={100} onChange={(v) => P({ speed: v })} />
          </Prop>
        </>
      )}

      {node.type === "play_macro" && (
        <Prop label={t("flow.macro")} hint={t("flow.macro_hint")}>
          <select
            className="select"
            value={node.macro_id}
            onChange={(e) => {
              const m = listing?.entries.find((x) => x.id === e.target.value);
              P({ macro_id: e.target.value, name: m?.manifest.meta.name ?? "" });
            }}
          >
            <option value="">{t("flow.pick_macro")}</option>
            {listing?.entries
              .filter((x) => x.id !== doc.id)
              .map((x) => (
                <option key={x.id} value={x.id}>
                  {x.folder ? `${x.folder} / ` : ""}
                  {x.manifest.meta.name}
                </option>
              ))}
          </select>
        </Prop>
      )}

      {node.type === "wait" && (
        <Prop label={t("flow.duration_ms")} hint={t("flow.expr_hint")}>
          <ExprInput value={node.ms} onChange={(v) => P({ ms: v })} placeholder="500 · random(200, 400)" />
          <div className="row" style={{ gap: 4, marginTop: 6 }}>
            {["100", "500", "1000", "random(300, 900)"].map((v) => (
              <button key={v} className="chip" onClick={() => P({ ms: v })}>
                {v}
              </button>
            ))}
          </div>
        </Prop>
      )}

      {node.type === "keys" && (
        <>
          <Prop label={t("flow.keys")} hint={t("flow.keys_hint")}>
            <div className="row wrap" style={{ gap: 6 }}>
              {node.keys.map((k, i) => (
                <span key={i} className="key-token">
                  <kbd>{keyLabel(k)}</kbd>
                  <button onClick={() => P({ keys: node.keys.filter((_, j) => j !== i) })} aria-label={t("common.remove")}>
                    ×
                  </button>
                </span>
              ))}
              <button className="btn sm" onClick={() => setPicking("keys")}>
                <Plus size={12} /> {t("flow.add_key")}
              </button>
            </div>
          </Prop>
          <Prop label={t("flow.hold")}>
            <NumberInput value={node.hold_ms} min={0} step={10} suffix="ms" width={100} onChange={(v) => P({ hold_ms: v })} />
          </Prop>
        </>
      )}

      {(node.type === "key_down" || node.type === "key_up") && (
        <Prop label={t("insp.key")} hint={node.type === "key_down" ? t("flow.key_down_hint") : undefined}>
          <button className="btn" onClick={() => setPicking("key")}>
            <kbd>{keyLabel(node.key)}</kbd> {t("insp.change")}
          </button>
        </Prop>
      )}

      {node.type === "type_text" && (
        <>
          <Prop label={t("flow.text")} hint={t("flow.text_hint")}>
            <TextInput area value={node.text} onChange={(v) => P({ text: v })} />
          </Prop>
          <Prop label={t("flow.interval")}>
            <NumberInput value={node.interval_ms} min={0} step={5} suffix="ms" width={100} onChange={(v) => P({ interval_ms: v })} />
          </Prop>
        </>
      )}

      {node.type === "move_mouse" && (
        <>
          <Prop label={node.relative ? t("flow.offset") : t("flow.position")} hint={t("flow.expr_hint")}>
            <div className="row" style={{ gap: 6 }}>
              <ExprInput value={node.x} onChange={(v) => P({ x: v })} placeholder="x" />
              <ExprInput value={node.y} onChange={(v) => P({ y: v })} placeholder="y" />
            </div>
          </Prop>
          {!node.relative && (
            <PickButton mode="point" label={t("screen.pick_point")} onPick={(r) => r.mode === "point" && P({ x: String(r.x), y: String(r.y) })} />
          )}
          <Prop label={t("flow.relative")}>
            <Toggle value={node.relative} onChange={(v) => P({ relative: v })} />
          </Prop>
          <Prop label={t("flow.motion")}>
            <Segmented<Motion>
              value={node.motion}
              onChange={(v) => P({ motion: v })}
              options={(["instant", "linear", "smooth", "human"] as Motion[]).map((m) => ({ value: m, label: t(`motion.${m}`) }))}
            />
          </Prop>
          {node.motion !== "instant" && (
            <Prop label={t("flow.duration_ms")}>
              <NumberInput value={node.duration_ms} min={0} step={50} suffix="ms" width={100} onChange={(v) => P({ duration_ms: v })} />
            </Prop>
          )}
        </>
      )}

      {node.type === "click" && (
        <>
          <Prop label={t("insp.button")}>
            <ButtonSelect value={node.button} onChange={(b) => P({ button: b })} />
          </Prop>
          <Prop label={t("studio.click_count")}>
            <NumberInput value={node.count} min={1} max={100} step={1} width={80} onChange={(v) => P({ count: Math.round(v) })} />
          </Prop>
          <Prop label={t("flow.hold")}>
            <NumberInput value={node.hold_ms} min={0} step={10} suffix="ms" width={100} onChange={(v) => P({ hold_ms: v })} />
          </Prop>
          {node.count > 1 && (
            <Prop label={t("flow.interval")}>
              <NumberInput value={node.interval_ms} min={0} step={10} suffix="ms" width={100} onChange={(v) => P({ interval_ms: v })} />
            </Prop>
          )}
          <p className="faint prop-hint">{t("flow.click_here_hint")}</p>
        </>
      )}

      {(node.type === "mouse_down" || node.type === "mouse_up") && (
        <Prop label={t("insp.button")}>
          <ButtonSelect value={node.button} onChange={(b) => P({ button: b })} />
        </Prop>
      )}

      {node.type === "scroll" && (
        <>
          <Prop label={t("insp.notches")} hint={t("studio.notches_hint")}>
            <NumberInput value={node.notches} min={-100} max={100} step={1} width={90} onChange={(v) => P({ notches: Math.round(v) })} />
          </Prop>
          <Prop label={t("insp.horizontal")}>
            <Toggle value={node.horizontal} onChange={(v) => P({ horizontal: v })} />
          </Prop>
        </>
      )}

      {node.type === "repeat" && (
        <>
          <Prop label={t("flow.count")} hint={t("flow.expr_hint")}>
            <div className="row" style={{ gap: 6 }}>
              {node.count !== null && <ExprInput value={node.count} onChange={(v) => P({ count: v })} />}
              <button className={`btn sm ${node.count === null ? "on" : ""}`} onClick={() => P({ count: node.count === null ? "3" : null })}>
                {t("flow.forever")}
              </button>
            </div>
          </Prop>
          <Prop label={t("flow.counter")} hint={t("flow.counter_hint")}>
            <TextInput value={node.counter} onChange={(v) => P({ counter: v.replace(/[^A-Za-z0-9_]/g, "") })} />
          </Prop>
        </>
      )}

      {(node.type === "while" || node.type === "if" || node.type === "wait_until") && (
        <Prop label={t("flow.condition")}>
          <ConditionEditor cond={node.cond} onChange={(c) => P({ cond: c })} canPixel={canPixel} canWindow={canWindow} />
        </Prop>
      )}
      {node.type === "while" && (
        <Prop label={t("flow.max_iterations")} hint={t("flow.max_iterations_hint")}>
          <div className="row" style={{ gap: 6 }}>
            <Toggle value={node.max_iterations !== null} onChange={(v) => P({ max_iterations: v ? 1000 : null })} />
            {node.max_iterations !== null && (
              <NumberInput value={node.max_iterations} min={1} step={10} width={100} onChange={(v) => P({ max_iterations: Math.round(v) })} />
            )}
          </div>
        </Prop>
      )}
      {node.type === "wait_until" && (
        <>
          <Prop label={t("flow.timeout")} hint={t("flow.timeout_hint")}>
            <div className="row" style={{ gap: 6 }}>
              <Toggle value={node.timeout_ms !== null} onChange={(v) => P({ timeout_ms: v ? 10_000 : null })} />
              {node.timeout_ms !== null && (
                <NumberInput value={node.timeout_ms} min={1} step={500} suffix="ms" width={110} onChange={(v) => P({ timeout_ms: v })} />
              )}
            </div>
          </Prop>
          <Prop label={t("flow.poll")}>
            <NumberInput value={node.poll_ms} min={5} step={10} suffix="ms" width={100} onChange={(v) => P({ poll_ms: v })} />
          </Prop>
        </>
      )}

      {node.type === "find" && (
        <>
          <Prop label={t("screen.look_for")}>
            <TargetEditor doc={doc} target={node.target} onChange={(target) => P({ target })} />
          </Prop>
          <Prop label={t("screen.where")}>
            <RegionEditor region={node.region} onChange={(region) => P({ region })} />
          </Prop>
          <ScreenTestButton doc={doc} target={node.target} region={node.region} />
          <Prop label={t("screen.wait_for_it")} hint={node.timeout_ms > 0 ? t("screen.wait_hint") : t("screen.no_wait_hint")}>
            <div className="row" style={{ gap: 6 }}>
              <Toggle value={node.timeout_ms > 0} onChange={(v) => P({ timeout_ms: v ? 5_000 : 0 })} label={t("screen.wait_for_it")} />
              {node.timeout_ms > 0 && <NumberInput value={node.timeout_ms} min={100} step={500} suffix="ms" width={110} onChange={(v) => P({ timeout_ms: v })} />}
            </div>
          </Prop>
          {node.timeout_ms > 0 && (
            <Prop label={t("flow.poll")}>
              <NumberInput value={node.poll_ms} min={10} step={50} suffix="ms" width={100} onChange={(v) => P({ poll_ms: v })} />
            </Prop>
          )}
          <Prop label={t("screen.then")}>
            <select className="select" value={node.action} onChange={(e) => P({ action: e.target.value as FoundAction })}>
              {FOUND_ACTIONS.map((a) => (
                <option key={a} value={a}>
                  {t(`found.${a}` as TKey)}
                </option>
              ))}
            </select>
          </Prop>
          {node.action !== "none" && (
            <>
              <Prop label={t("screen.offset")} hint={t("screen.offset_hint")}>
                <div className="row" style={{ gap: 6 }}>
                  <NumberInput value={node.offset_x} step={1} suffix="x" width={90} onChange={(v) => P({ offset_x: Math.round(v) })} />
                  <NumberInput value={node.offset_y} step={1} suffix="y" width={90} onChange={(v) => P({ offset_y: Math.round(v) })} />
                </div>
              </Prop>
              <Prop label={t("flow.motion")}>
                <Segmented<Motion>
                  value={node.motion}
                  onChange={(v) => P({ motion: v })}
                  options={(["instant", "linear", "smooth", "human"] as Motion[]).map((m) => ({ value: m, label: t(`motion.${m}`) }))}
                />
              </Prop>
              {node.motion !== "instant" && (
                <Prop label={t("flow.duration_ms")}>
                  <NumberInput value={node.duration_ms} min={0} step={50} suffix="ms" width={100} onChange={(v) => P({ duration_ms: v })} />
                </Prop>
              )}
            </>
          )}
          <Prop label={t("screen.if_missing")}>
            <Segmented<IfMissing>
              value={node.if_missing}
              onChange={(v) => P({ if_missing: v })}
              options={(["continue", "stop", "fail"] as IfMissing[]).map((m) => ({ value: m, label: t(`missing.${m}`), title: t(`missing.${m}_hint` as TKey) }))}
            />
            <div className="prop-hint">{t(`missing.${node.if_missing}_hint` as TKey)}</div>
          </Prop>
          <Prop label={t("screen.variable")} hint={t("screen.variable_hint", { v: node.var.trim() || "found" })}>
            <TextInput value={node.var} onChange={(v) => P({ var: v.replace(/[^A-Za-z0-9_]/g, "") })} />
          </Prop>
        </>
      )}

      {node.type === "set_var" && (
        <>
          <Prop label={t("flow.var_name")}>
            <TextInput value={node.name} onChange={(v) => P({ name: v.replace(/[^A-Za-z0-9_]/g, "") })} />
          </Prop>
          <Prop label={t("flow.value")} hint={t("flow.expr_hint")}>
            <ExprInput value={node.value} onChange={(v) => P({ value: v })} placeholder={vars[0] ? `${vars[0]} + 1` : "0"} />
          </Prop>
        </>
      )}

      {node.type === "notify" && (
        <Prop label={t("flow.message")} hint={t("flow.notify_hint")}>
          <TextInput value={node.message} onChange={(v) => P({ message: v })} />
        </Prop>
      )}

      {(node.type === "break" || node.type === "continue" || node.type === "stop") && (
        <p className="muted">{t(`block.${node.type}_hint` as TKey)}</p>
      )}

      {picking === "key" && (node.type === "key_down" || node.type === "key_up") && (
        <KeyPicker onClose={() => setPicking(null)} onPick={(k: Key) => P({ key: k })} />
      )}
      {picking === "keys" && node.type === "keys" && (
        <KeyPicker onClose={() => setPicking(null)} onPick={(k: Key) => P({ keys: [...node.keys, k] })} />
      )}
    </div>
  );
}

const COND_TYPES: Condition["type"][] = ["expr", "chance", "on_screen", "window_active", "window_exists", "pixel_color", "key_held", "not", "all", "any"];

function defaultCond(type: Condition["type"]): Condition {
  switch (type) {
    case "expr":
      return { type, expr: "i < 10" };
    case "window_active":
    case "window_exists":
      return { type, title: "", process: "" };
    case "pixel_color":
      return { type, x: 0, y: 0, color: "#ffffff", tolerance: 10 };
    case "key_held":
      return { type, key: { scan: 0x2a, vk: 0xa0 } };
    case "on_screen":
      return { type, target: { kind: "image", image: "", threshold: 0.9, any_scale: false }, region: null };
    case "chance":
      return { type, percent: 50 };
    case "not":
      return { type, cond: { type: "chance", percent: 50 } };
    case "all":
    case "any":
      return { type, conds: [{ type: "chance", percent: 50 }] };
  }
}

function ConditionEditor({
  cond,
  onChange,
  canPixel,
  canWindow,
  depth = 0,
}: {
  cond: Condition;
  onChange: (c: Condition) => void;
  canPixel: boolean;
  canWindow: boolean;
  depth?: number;
}) {
  const [picking, setPicking] = useState(false);
  const doc = useContext(FlowDoc);
  const C = onChange;
  return (
    <div className={`cond ${depth ? "nested" : ""}`}>
      <select className="select sm" value={cond.type} onChange={(e) => C(defaultCond(e.target.value as Condition["type"]))}>
        {COND_TYPES.filter((ty) => depth < 3 || !["not", "all", "any"].includes(ty)).map((ty) => (
          <option key={ty} value={ty}>
            {t(`cond.type_${ty}` as TKey)}
          </option>
        ))}
      </select>
      {cond.type === "expr" && <ExprInput value={cond.expr} onChange={(v) => C({ ...cond, expr: v })} placeholder="i < 10 && !done" />}
      {cond.type === "chance" && (
        <div className="row" style={{ gap: 6 }}>
          <Dices size={14} className="faint" />
          <NumberInput value={cond.percent} min={0} max={100} step={5} suffix="%" width={90} onChange={(v) => C({ ...cond, percent: v })} />
        </div>
      )}
      {(cond.type === "window_active" || cond.type === "window_exists") && (
        <>
          <TextInput value={cond.process} onChange={(v) => C({ ...cond, process: v })} placeholder={t("cond.process_ph")} />
          <TextInput value={cond.title} onChange={(v) => C({ ...cond, title: v })} placeholder={t("cond.title_ph")} />
          <div className="prop-hint">{canWindow ? t("cond.window_hint") : t("cond.window_unavailable")}</div>
        </>
      )}
      {cond.type === "pixel_color" && (
        <>
          <div className="row" style={{ gap: 6 }}>
            <NumberInput value={cond.x} step={1} width={80} onChange={(v) => C({ ...cond, x: Math.round(v) })} />
            <NumberInput value={cond.y} step={1} width={80} onChange={(v) => C({ ...cond, y: Math.round(v) })} />
            <input type="color" className="color-input" value={cond.color} onChange={(e) => C({ ...cond, color: e.target.value })} />
          </div>
          <PickButton mode="point" label={t("screen.pick_pixel")} onPick={(r) => r.mode === "point" && C({ ...cond, x: r.x, y: r.y, color: r.color })} />
          <div className="row" style={{ gap: 6 }}>
            <span className="faint">{t("cond.tolerance")}</span>
            <NumberInput value={cond.tolerance} min={0} max={255} step={1} width={80} onChange={(v) => C({ ...cond, tolerance: Math.round(v) })} />
          </div>
          {!canPixel && <div className="prop-error">{t("cond.pixel_unavailable")}</div>}
        </>
      )}
      {cond.type === "key_held" && (
        <>
          <button className="btn sm" onClick={() => setPicking(true)}>
            <kbd>{keyLabel(cond.key)}</kbd> {t("insp.change")}
          </button>
          {picking && <KeyPicker onClose={() => setPicking(false)} onPick={(k) => C({ ...cond, key: k })} />}
        </>
      )}
      {cond.type === "on_screen" && doc && (
        <>
          <TargetEditor doc={doc} target={cond.target} onChange={(target) => C({ ...cond, target })} />
          <RegionEditor region={cond.region} onChange={(region) => C({ ...cond, region })} />
          <ScreenTestButton doc={doc} target={cond.target} region={cond.region} />
        </>
      )}
      {cond.type === "not" && (
        <ConditionEditor cond={cond.cond} onChange={(c) => C({ ...cond, cond: c })} canPixel={canPixel} canWindow={canWindow} depth={depth + 1} />
      )}
      {(cond.type === "all" || cond.type === "any") && (
        <>
          {cond.conds.map((c, i) => (
            <div key={i} className="cond-item">
              <ConditionEditor
                cond={c}
                onChange={(n) => C({ ...cond, conds: cond.conds.map((x, j) => (j === i ? n : x)) })}
                canPixel={canPixel}
                canWindow={canWindow}
                depth={depth + 1}
              />
              <button className="btn ghost sm icon" onClick={() => C({ ...cond, conds: cond.conds.filter((_, j) => j !== i) })} aria-label={t("common.remove")}>
                <Trash2 size={12} />
              </button>
            </div>
          ))}
          <button className="btn sm" onClick={() => C({ ...cond, conds: [...cond.conds, { type: "chance", percent: 50 }] })}>
            <Plus size={12} /> {cond.type === "all" ? t("cond.add_and") : t("cond.add_or")}
          </button>
        </>
      )}
    </div>
  );
}
