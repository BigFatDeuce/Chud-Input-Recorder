// Library: folders, macro cards, search, details, trash and versions.
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  ArchiveRestore,
  ArrowUpDown,
  ChevronDown,
  ChevronRight,
  Copy,
  Download,
  ExternalLink,
  Folder,
  FolderOpen,
  FolderPlus,
  History,
  LayoutGrid,
  List,
  Pencil,
  Play,
  Plus,
  Search,
  Star,
  Trash2,
  Upload,
  Workflow,
  X,
  AudioWaveform,
  FolderInput,
  TriangleAlert,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { RecordButton } from "../components/Transport";
import { Sparkline } from "../components/Sparkline";
import { Modal, useConfirm, useContextMenu, usePrompt, type MenuEntry } from "../components/ui";
import { fmtDate, fmtInt, fmtRelative, t, tn, useLang } from "../i18n";
import { hotkeyFor, playMacro } from "../lib/actions";
import { api, type MetaPatch } from "../lib/api";
import { fmtBytes, fmtDuration } from "../lib/format";
import { MACRO_COLORS, MACRO_ICONS, macroIcon } from "../lib/icons";
import { useStore } from "../lib/store";
import type { LibraryEntry, TrashItem, Version } from "../lib/types";

type Scope = "all" | "favorites" | "folder" | "trash";
type Sort = "modified" | "name" | "duration" | "created" | "events";

const SORTS: Sort[] = ["modified", "name", "duration", "created", "events"];

function persisted<T>(key: string, fallback: T): T {
  try {
    const v = localStorage.getItem(key);
    return v ? (JSON.parse(v) as T) : fallback;
  } catch {
    return fallback;
  }
}
function persist(key: string, v: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(v));
  } catch {
    /* storage unavailable */
  }
}

interface FolderNode {
  path: string;
  name: string;
  children: FolderNode[];
}

function folderTree(paths: string[]): FolderNode[] {
  const root: FolderNode = { path: "", name: "", children: [] };
  const byPath = new Map<string, FolderNode>([["", root]]);
  for (const p of [...paths].sort((a, b) => a.localeCompare(b))) {
    const parts = p.split("/");
    let parent = root;
    for (let i = 0; i < parts.length; i++) {
      const path = parts.slice(0, i + 1).join("/");
      let n = byPath.get(path);
      if (!n) {
        n = { path, name: parts[i], children: [] };
        byPath.set(path, n);
        parent.children.push(n);
      }
      parent = n;
    }
  }
  return root.children;
}

function inFolder(entry: LibraryEntry, folder: string): boolean {
  return entry.folder === folder || entry.folder.startsWith(`${folder}/`);
}

/** Pointer-based drag (HTML5 drag & drop is taken by the OS file drop). */
function useDragTo(onDrop: (kind: string, payload: string[], folder: string) => void) {
  const [drag, setDrag] = useState<{ kind: string; payload: string[]; label: string; x: number; y: number } | null>(null);
  const [over, setOver] = useState<string | null>(null);
  const start = (e: React.MouseEvent, kind: string, payload: string[], label: string) => {
    if (e.button !== 0) return;
    const x0 = e.clientX;
    const y0 = e.clientY;
    let active = false;
    let target: string | null = null;
    const move = (ev: MouseEvent) => {
      if (!active && Math.hypot(ev.clientX - x0, ev.clientY - y0) < 6) return;
      active = true;
      const el = document.elementFromPoint(ev.clientX, ev.clientY)?.closest<HTMLElement>("[data-drop-folder]");
      target = el ? (el.dataset.dropFolder ?? null) : null;
      setOver(target);
      setDrag({ kind, payload, label, x: ev.clientX, y: ev.clientY });
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      if (active && target !== null) onDrop(kind, payload, target);
      setDrag(null);
      setOver(null);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };
  const ghost = drag ? (
    <div className="drag-ghost" style={{ left: drag.x + 14, top: drag.y + 10 }}>
      {drag.kind === "folder" ? <Folder size={14} /> : <AudioWaveform size={14} />}
      <span>{drag.label}</span>
    </div>
  ) : null;
  return { start, ghost, over, dragging: Boolean(drag) };
}

export function LibraryView() {
  useLang();
  const listing = useStore((s) => s.listing);
  const selection = useStore((s) => s.libSelection);
  const folder = useStore((s) => s.libFolder);
  const settings = useStore((s) => s.settings);
  const refresh = useStore((s) => s.refreshLibrary);
  const openDoc = useStore((s) => s.openDoc);
  const toast = useStore((s) => s.toast);
  const fail = useStore((s) => s.fail);
  const docs = useStore((s) => s.docs);

  const [scope, setScopeState] = useState<Scope>(() => persisted("lib.scope", "all"));
  const [sort, setSort] = useState<Sort>(() => persisted("lib.sort", "modified"));
  const [layout, setLayout] = useState<"grid" | "list">(() => persisted("lib.layout", "grid"));
  const [query, setQuery] = useState("");
  const [expanded, setExpanded] = useState<Record<string, boolean>>(() => persisted("lib.expanded", {}));
  const [trash, setTrash] = useState<TrashItem[]>([]);
  const [editing, setEditing] = useState<string | null>(null);
  const [versionsOf, setVersionsOf] = useState<string | null>(null);
  const anchor = useRef<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const menu = useContextMenu();
  const confirm = useConfirm();
  const prompt = usePrompt();

  const setSel = (ids: string[]) => useStore.setState({ libSelection: ids });
  const setFolder = (f: string) => useStore.setState({ libFolder: f });
  const setScope = (s: Scope, f = "") => {
    setScopeState(s);
    persist("lib.scope", s);
    setFolder(s === "folder" ? f : "");
    setSel([]);
  };

  useEffect(() => persist("lib.sort", sort), [sort]);
  useEffect(() => persist("lib.layout", layout), [layout]);
  useEffect(() => persist("lib.expanded", expanded), [expanded]);

  const loadTrash = useCallback(() => {
    api.trashList().then(setTrash).catch(() => setTrash([]));
  }, []);
  useEffect(() => {
    loadTrash();
  }, [loadTrash, listing]);

  // A folder that disappeared (renamed elsewhere) falls back to "all".
  useEffect(() => {
    if (scope === "folder" && listing && folder && !listing.folders.includes(folder)) setScope("all");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [listing]);

  const entries = listing?.entries ?? [];
  const tree = useMemo(() => folderTree(listing?.folders ?? []), [listing]);

  const visible = useMemo(() => {
    let list = entries;
    if (scope === "favorites") list = list.filter((e) => e.manifest.meta.favorite);
    else if (scope === "folder") list = list.filter((e) => inFolder(e, folder));
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    if (words.length) {
      list = list.filter((e) => {
        const m = e.manifest.meta;
        const hay = `${m.name} ${m.description} ${m.tags.map((x) => `#${x}`).join(" ")} ${e.folder}`.toLowerCase();
        return words.every((w) => hay.includes(w));
      });
    }
    const by: Record<Sort, (a: LibraryEntry, b: LibraryEntry) => number> = {
      modified: (a, b) => b.manifest.meta.modified_ms - a.manifest.meta.modified_ms,
      created: (a, b) => b.manifest.meta.created_ms - a.manifest.meta.created_ms,
      name: (a, b) => a.manifest.meta.name.localeCompare(b.manifest.meta.name, undefined, { numeric: true }),
      duration: (a, b) => b.manifest.summary.duration_us - a.manifest.summary.duration_us,
      events: (a, b) => b.manifest.summary.event_count - a.manifest.summary.event_count,
    };
    return [...list].sort((a, b) => {
      if (a.manifest.meta.favorite !== b.manifest.meta.favorite && sort === "modified")
        return a.manifest.meta.favorite ? -1 : 1;
      return by[sort](a, b);
    });
  }, [entries, scope, folder, query, sort]);

  const selected = useMemo(() => entries.filter((e) => selection.includes(e.id)), [entries, selection]);
  const one = selected.length === 1 ? selected[0] : null;

  // ── actions ──
  const destFolder = scope === "folder" ? folder : "";

  const createMacro = async () => {
    const name = await prompt.ask({
      title: t("lib.new_macro"),
      value: t("lib.untitled"),
      confirm: t("common.create"),
      cancel: t("common.cancel"),
    });
    if (!name?.trim()) return;
    try {
      const id = await api.docNew(name.trim(), destFolder);
      await refresh();
      await openDoc(id);
    } catch (e) {
      fail(e, "error.library");
    }
  };

  const importFiles = async () => {
    const picked = await openDialog({
      multiple: true,
      filters: [{ name: "Chud Input Recorder", extensions: ["chud", "json"] }],
    });
    if (!picked) return;
    const paths = Array.isArray(picked) ? picked : [picked];
    try {
      const r = await api.importFiles(paths, destFolder);
      await refresh();
      if (r.imported.length) {
        toast("success", t("toast.imported", { n: r.imported.length }));
        setSel(r.imported);
      }
      for (const [p, err] of r.failed) toast("error", `${p.split(/[\\/]/).pop()} — ${err}`);
    } catch (e) {
      fail(e, "error.import");
    }
  };

  const exportMacro = async (e: LibraryEntry) => {
    const dest = await saveDialog({
      defaultPath: `${e.manifest.meta.name}.chud`,
      filters: [
        { name: "Chud (.chud)", extensions: ["chud"] },
        { name: "JSON", extensions: ["json"] },
      ],
    });
    if (!dest) return;
    try {
      await api.exportMacro(e.id, dest);
      toast("success", t("toast.exported", { name: e.manifest.meta.name }), {
        label: t("lib.reveal"),
        run: () => revealItemInDir(dest).catch(() => {}),
      });
    } catch (err) {
      fail(err, "error.export");
    }
  };

  const rename = async (e: LibraryEntry) => {
    const name = await prompt.ask({
      title: t("lib.rename"),
      value: e.manifest.meta.name,
      confirm: t("common.rename"),
      cancel: t("common.cancel"),
    });
    if (!name?.trim() || name.trim() === e.manifest.meta.name) return;
    await patchMeta(e.id, { name: name.trim() });
  };

  const patchMeta = async (id: string, patch: MetaPatch) => {
    try {
      const meta = await api.setMeta(id, patch);
      const d = useStore.getState().docs[id];
      if (d) useStore.getState().patchDoc(id, { meta });
      await refresh();
    } catch (e) {
      fail(e, "error.library");
    }
  };

  const duplicate = async (ids: string[]) => {
    try {
      const made: string[] = [];
      for (const id of ids) made.push(await api.duplicate(id, t("lib.copy_suffix")));
      await refresh();
      setSel(made);
    } catch (e) {
      fail(e, "error.library");
    }
  };

  const trashMacros = async (ids: string[]) => {
    if (!ids.length) return;
    if (settings?.confirm_delete) {
      const names = entries.filter((e) => ids.includes(e.id)).map((e) => e.manifest.meta.name);
      const ok = await confirm.ask({
        title: tn("lib.delete_one", "lib.delete_many", ids.length),
        body: (
          <>
            {names.length <= 5 ? names.map((n) => <div key={n}>• {n}</div>) : <div>{names.slice(0, 5).join(", ")}…</div>}
            <div style={{ marginTop: 10 }}>{t("lib.delete_body")}</div>
          </>
        ),
        confirm: t("common.delete"),
        cancel: t("common.cancel"),
        danger: true,
      });
      if (!ok) return;
    }
    try {
      for (const id of ids) if (useStore.getState().docs[id]) await useStore.getState().closeDoc(id);
      await api.trash(ids);
      setSel([]);
      await refresh();
      toast("info", tn("toast.trashed_one", "toast.trashed_many", ids.length), {
        label: t("lib.open_trash"),
        run: () => setScope("trash"),
      });
    } catch (e) {
      fail(e, "error.library");
    }
  };

  const moveTo = async (ids: string[], to: string) => {
    try {
      await api.move(ids, to);
      await refresh();
    } catch (e) {
      fail(e, "error.library");
    }
  };

  const newFolder = async (parent: string) => {
    const name = await prompt.ask({
      title: t("lib.new_folder"),
      value: t("lib.folder_default"),
      confirm: t("common.create"),
      cancel: t("common.cancel"),
    });
    if (!name?.trim()) return;
    try {
      const path = await api.createFolder(parent, name.trim());
      if (parent) setExpanded((x) => ({ ...x, [parent]: true }));
      await refresh();
      setScope("folder", path);
    } catch (e) {
      fail(e, "error.library");
    }
  };

  const renameFolder = async (path: string) => {
    const cur = path.split("/").pop() ?? path;
    const name = await prompt.ask({ title: t("lib.rename_folder"), value: cur, confirm: t("common.rename"), cancel: t("common.cancel") });
    if (!name?.trim() || name.trim() === cur) return;
    try {
      const next = await api.renameFolder(path, name.trim());
      await refresh();
      if (folder === path) setScope("folder", next);
    } catch (e) {
      fail(e, "error.library");
    }
  };

  const trashFolder = async (path: string) => {
    const count = entries.filter((e) => inFolder(e, path)).length;
    const ok = await confirm.ask({
      title: t("lib.delete_folder", { name: path.split("/").pop() ?? path }),
      body: count ? tn("lib.delete_folder_body_one", "lib.delete_folder_body_many", count) : t("lib.delete_folder_empty"),
      confirm: t("common.delete"),
      cancel: t("common.cancel"),
      danger: true,
    });
    if (!ok) return;
    try {
      for (const e of entries) if (inFolder(e, path) && useStore.getState().docs[e.id]) await useStore.getState().closeDoc(e.id);
      await api.trashFolder(path);
      if (folder === path || folder.startsWith(`${path}/`)) setScope("all");
      await refresh();
    } catch (e) {
      fail(e, "error.library");
    }
  };

  const drag = useDragTo(async (kind, payload, to) => {
    if (kind === "macros") await moveTo(payload, to);
    else if (kind === "folder") {
      const src = payload[0];
      if (to === src || to.startsWith(`${src}/`)) return;
      try {
        await api.moveFolder(src, to);
        await refresh();
      } catch (e) {
        fail(e, "error.library");
      }
    }
  });

  // ── selection ──
  const clickCard = (e: React.MouseEvent, id: string) => {
    if (e.ctrlKey || e.metaKey) {
      setSel(selection.includes(id) ? selection.filter((x) => x !== id) : [...selection, id]);
      anchor.current = id;
    } else if (e.shiftKey && anchor.current) {
      const ids = visible.map((v) => v.id);
      const a = ids.indexOf(anchor.current);
      const b = ids.indexOf(id);
      if (a >= 0 && b >= 0) setSel(ids.slice(Math.min(a, b), Math.max(a, b) + 1));
    } else {
      setSel([id]);
      anchor.current = id;
    }
  };

  const cardMenu = (e: React.MouseEvent, entry: LibraryEntry) => {
    e.preventDefault();
    const ids = selection.includes(entry.id) ? selection : [entry.id];
    if (!selection.includes(entry.id)) setSel([entry.id]);
    const many = ids.length > 1;
    const folderItems: MenuEntry[] = [
      { label: t("lib.root"), icon: <FolderOpen size={14} />, onClick: () => moveTo(ids, "") },
      ...(listing?.folders ?? []).map((f) => ({
        label: f,
        icon: <Folder size={14} />,
        onClick: () => moveTo(ids, f),
      })),
    ];
    menu.open(e.clientX, e.clientY, [
      { label: t("lib.open_studio"), icon: <AudioWaveform size={14} />, shortcut: "Enter", disabled: many, onClick: () => openDoc(entry.id) },
      { label: t("transport.play"), icon: <Play size={14} />, disabled: many, onClick: () => playMacro(entry.id) },
      { separator: true },
      { label: t("lib.rename"), icon: <Pencil size={14} />, shortcut: "F2", disabled: many, onClick: () => rename(entry) },
      { label: t("lib.edit_details"), icon: <Pencil size={14} />, disabled: many, onClick: () => setEditing(entry.id) },
      {
        label: entry.manifest.meta.favorite ? t("lib.unfavorite") : t("lib.favorite"),
        icon: <Star size={14} />,
        onClick: () => ids.forEach((id) => patchMeta(id, { favorite: !entry.manifest.meta.favorite })),
      },
      { label: t("lib.duplicate"), icon: <Copy size={14} />, shortcut: "Ctrl+D", onClick: () => duplicate(ids) },
      { label: t("lib.export"), icon: <Download size={14} />, disabled: many, onClick: () => exportMacro(entry) },
      { label: t("lib.versions"), icon: <History size={14} />, disabled: many, onClick: () => setVersionsOf(entry.id) },
      {
        label: t("lib.reveal"),
        icon: <ExternalLink size={14} />,
        disabled: many,
        onClick: () => api.absolutePath(entry.id).then((p) => revealItemInDir(p)).catch((err) => fail(err)),
      },
      { separator: true },
      { title: t("lib.move_to") },
      ...folderItems.slice(0, 12),
      { separator: true },
      { label: t("common.delete"), icon: <Trash2 size={14} />, shortcut: "Del", danger: true, onClick: () => trashMacros(ids) },
    ]);
  };

  // ── keyboard ──
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
      if (document.querySelector(".modal-backdrop, .menu")) return;
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        searchRef.current?.focus();
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a" && scope !== "trash") {
        e.preventDefault();
        setSel(visible.map((v) => v.id));
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "d" && selection.length) {
        e.preventDefault();
        duplicate(selection);
      } else if (e.key === "Delete" && selection.length) {
        trashMacros(selection);
      } else if (e.key === "Enter" && one) {
        openDoc(one.id);
      } else if (e.key === "F2" && one) {
        rename(one);
      } else if (e.key === "Escape") {
        setSel([]);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const totalDuration = useMemo(() => entries.reduce((a, e) => a + e.manifest.summary.duration_us, 0), [entries]);
  const favCount = entries.filter((e) => e.manifest.meta.favorite).length;
  const title =
    scope === "all"
      ? t("lib.all")
      : scope === "favorites"
        ? t("lib.favorites")
        : scope === "trash"
          ? t("lib.trash")
          : (folder.split("/").pop() ?? folder);

  return (
    <div className="library">
      {/* ── sidebar ── */}
      <aside className="lib-side panel">
        <div className="lib-side-head">
          <span className="eyebrow">{t("lib.collections")}</span>
          <button className="btn ghost sm icon" onClick={() => newFolder(scope === "folder" ? folder : "")} title={t("lib.new_folder")}>
            <FolderPlus size={15} />
          </button>
        </div>
        <SideItem
          icon={<Orbitish />}
          label={t("lib.all")}
          count={entries.length}
          active={scope === "all"}
          onClick={() => setScope("all")}
          drop=""
          over={drag.over === ""}
        />
        <SideItem
          icon={<Star size={15} />}
          label={t("lib.favorites")}
          count={favCount}
          active={scope === "favorites"}
          onClick={() => setScope("favorites")}
        />
        <div className="lib-side-sep" />
        <div className="folder-tree">
          {tree.length === 0 && <div className="faint lib-side-hint">{t("lib.no_folders")}</div>}
          {tree.map((n) => (
            <FolderRow
              key={n.path}
              node={n}
              depth={0}
              expanded={expanded}
              setExpanded={setExpanded}
              active={scope === "folder" ? folder : null}
              over={drag.over}
              counts={entries}
              onSelect={(p) => setScope("folder", p)}
              onMenu={(e, p) =>
                menu.open(e.clientX, e.clientY, [
                  { label: t("lib.new_subfolder"), icon: <FolderPlus size={14} />, onClick: () => newFolder(p) },
                  { label: t("lib.rename_folder"), icon: <Pencil size={14} />, onClick: () => renameFolder(p) },
                  {
                    label: t("lib.reveal"),
                    icon: <ExternalLink size={14} />,
                    onClick: () =>
                      api
                        .absolutePath(null)
                        .then((root) => revealItemInDir(`${root}/${p}`))
                        .catch((err) => fail(err)),
                  },
                  { label: t("lib.move_to_root"), icon: <FolderInput size={14} />, disabled: !p.includes("/"), onClick: () => api.moveFolder(p, "").then(refresh).catch((err) => fail(err)) },
                  { separator: true },
                  { label: t("common.delete"), icon: <Trash2 size={14} />, danger: true, onClick: () => trashFolder(p) },
                ])
              }
              onDragStart={(e, p) => drag.start(e, "folder", [p], p.split("/").pop() ?? p)}
            />
          ))}
        </div>
        <div className="spacer" />
        <SideItem
          icon={<Trash2 size={15} />}
          label={t("lib.trash")}
          count={trash.length}
          active={scope === "trash"}
          onClick={() => setScope("trash")}
        />
        <div className="lib-side-foot faint mono" title={listing?.root}>
          {fmtInt(entries.length)} · {fmtDuration(totalDuration)}
        </div>
      </aside>

      {/* ── main ── */}
      <section className="lib-main">
        <div className="lib-toolbar">
          <div className="lib-title">
            <h1>{title}</h1>
            {scope !== "trash" && <span className="chip">{fmtInt(visible.length)}</span>}
          </div>
          {scope !== "trash" && (
            <>
              <div className="search">
                <Search size={14} />
                <input
                  ref={searchRef}
                  className="input"
                  placeholder={t("lib.search")}
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  onKeyDown={(e) => e.key === "Escape" && (setQuery(""), (e.target as HTMLInputElement).blur())}
                />
                {query && (
                  <button className="btn ghost sm icon" onClick={() => setQuery("")} aria-label={t("common.clear")}>
                    <X size={12} />
                  </button>
                )}
              </div>
              <div className="sort-select">
                <ArrowUpDown size={13} />
                <select className="select sm" value={sort} onChange={(e) => setSort(e.target.value as Sort)}>
                  {SORTS.map((s) => (
                    <option key={s} value={s}>
                      {t(`lib.sort_${s}`)}
                    </option>
                  ))}
                </select>
              </div>
              <div className="btn-group">
                <button className={`btn sm icon ${layout === "grid" ? "on" : ""}`} onClick={() => setLayout("grid")} title={t("lib.grid")}>
                  <LayoutGrid size={14} />
                </button>
                <button className={`btn sm icon ${layout === "list" ? "on" : ""}`} onClick={() => setLayout("list")} title={t("lib.list")}>
                  <List size={14} />
                </button>
              </div>
              <div className="vdivider" />
              <button className="btn sm" onClick={importFiles}>
                <Upload size={14} /> {t("lib.import")}
              </button>
              <button className="btn sm primary" onClick={createMacro}>
                <Plus size={14} /> {t("lib.new_macro")}
              </button>
            </>
          )}
          {scope === "trash" && (
            <>
              <div className="spacer" />
              <button
                className="btn sm danger"
                disabled={!trash.length}
                onClick={async () => {
                  const ok = await confirm.ask({
                    title: t("lib.empty_trash"),
                    body: t("lib.empty_trash_body", { n: trash.length }),
                    confirm: t("lib.empty_trash"),
                    cancel: t("common.cancel"),
                    danger: true,
                  });
                  if (!ok) return;
                  try {
                    const n = await api.emptyTrash();
                    toast("info", t("toast.trash_emptied", { n }));
                    loadTrash();
                  } catch (e) {
                    fail(e, "error.library");
                  }
                }}
              >
                <Trash2 size={14} /> {t("lib.empty_trash")}
              </button>
            </>
          )}
        </div>

        {listing && listing.broken.length > 0 && scope !== "trash" && (
          <div className="lib-warning">
            <TriangleAlert size={15} />
            <div>
              <b>{tn("lib.broken_one", "lib.broken_many", listing.broken.length)}</b>
              <div className="faint mono">
                {listing.broken.slice(0, 3).map(([p, err]) => (
                  <div key={p} className="ellipsis">
                    {p} — {err}
                  </div>
                ))}
              </div>
            </div>
          </div>
        )}

        <div
          className="lib-scroll"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) setSel([]);
          }}
        >
          {scope === "trash" ? (
            <TrashList
              items={trash}
              onRestore={async (it) => {
                try {
                  await api.restore(it.deleted_ms, it.name);
                  await refresh();
                  loadTrash();
                  toast("success", t("toast.restored", { name: it.name }));
                } catch (e) {
                  fail(e, "error.library");
                }
              }}
            />
          ) : visible.length === 0 ? (
            entries.length === 0 ? (
              <EmptyLibrary onCreate={createMacro} onImport={importFiles} />
            ) : (
              <div className="empty">
                <Search size={28} className="faint" />
                <h3>{t("lib.nothing_found")}</h3>
                <p className="muted">{query ? t("lib.nothing_found_query", { q: query }) : t("lib.nothing_here")}</p>
              </div>
            )
          ) : layout === "grid" ? (
            <div className="card-grid">
              {visible.map((e) => (
                <MacroCard
                  key={e.id}
                  entry={e}
                  selected={selection.includes(e.id)}
                  open={Boolean(docs[e.id])}
                  showFolder={scope !== "folder"}
                  onMouseDown={(ev) => {
                    if (!selection.includes(e.id) || ev.ctrlKey || ev.shiftKey) clickCard(ev, e.id);
                    const ids = selection.includes(e.id) && !ev.ctrlKey ? selection : [e.id];
                    drag.start(ev, "macros", ids, ids.length > 1 ? tn("lib.n_macros_one", "lib.n_macros_many", ids.length) : e.manifest.meta.name);
                  }}
                  onClick={(ev) => {
                    if (!ev.ctrlKey && !ev.shiftKey && selection.length > 1 && !drag.dragging) setSel([e.id]);
                  }}
                  onDoubleClick={() => openDoc(e.id)}
                  onContextMenu={(ev) => cardMenu(ev, e)}
                  onPlay={() => playMacro(e.id)}
                  onFavorite={() => patchMeta(e.id, { favorite: !e.manifest.meta.favorite })}
                />
              ))}
            </div>
          ) : (
            <table className="table lib-table">
              <thead>
                <tr>
                  <th />
                  <th>{t("lib.col_name")}</th>
                  <th>{t("lib.col_folder")}</th>
                  <th>{t("lib.col_duration")}</th>
                  <th>{t("lib.col_events")}</th>
                  <th>{t("lib.col_modified")}</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {visible.map((e) => {
                  const m = e.manifest.meta;
                  const Icon = macroIcon(m.icon);
                  return (
                    <tr
                      key={e.id}
                      className={selection.includes(e.id) ? "selected" : ""}
                      onMouseDown={(ev) => {
                        clickCard(ev, e.id);
                        const ids = selection.includes(e.id) && !ev.ctrlKey ? selection : [e.id];
                        drag.start(ev, "macros", ids, m.name);
                      }}
                      onDoubleClick={() => openDoc(e.id)}
                      onContextMenu={(ev) => cardMenu(ev, e)}
                    >
                      <td style={{ width: 28 }}>
                        <Icon size={15} color={m.color} />
                      </td>
                      <td>
                        <div className="row" style={{ gap: 6 }}>
                          <span className="ellipsis" style={{ maxWidth: 360 }}>
                            {m.name}
                          </span>
                          {m.favorite && <Star size={12} className="fav-on" fill="currentColor" />}
                          {e.manifest.summary.has_flow && <Workflow size={12} className="faint" />}
                        </div>
                      </td>
                      <td className="muted">{e.folder || "—"}</td>
                      <td className="mono">{fmtDuration(e.manifest.summary.duration_us)}</td>
                      <td className="mono">{fmtInt(e.manifest.summary.event_count)}</td>
                      <td className="muted" title={fmtDate(m.modified_ms)}>
                        {fmtRelative(m.modified_ms)}
                      </td>
                      <td style={{ width: 40 }}>
                        <button className="btn ghost sm icon" onClick={() => playMacro(e.id)} title={t("transport.play")}>
                          <Play size={13} />
                        </button>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </div>
      </section>

      {/* ── details ── */}
      {scope !== "trash" && (
        <aside className="lib-details panel">
          {one ? (
            <Details
              key={one.id}
              entry={one}
              onPatch={(p) => patchMeta(one.id, p)}
              onExport={() => exportMacro(one)}
              onDuplicate={() => duplicate([one.id])}
              onDelete={() => trashMacros([one.id])}
              onVersions={() => setVersionsOf(one.id)}
              onEdit={() => setEditing(one.id)}
            />
          ) : selected.length > 1 ? (
            <div className="details-multi">
              <div className="eyebrow">{t("lib.selection")}</div>
              <h2>{tn("lib.n_macros_one", "lib.n_macros_many", selected.length)}</h2>
              <div className="muted">{fmtDuration(selected.reduce((a, e) => a + e.manifest.summary.duration_us, 0))}</div>
              <div className="col" style={{ gap: 8, marginTop: 16 }}>
                <button className="btn" onClick={() => duplicate(selection)}>
                  <Copy size={14} /> {t("lib.duplicate")}
                </button>
                <button className="btn danger" onClick={() => trashMacros(selection)}>
                  <Trash2 size={14} /> {t("common.delete")}
                </button>
              </div>
              <p className="faint" style={{ marginTop: 16 }}>
                {t("lib.drag_hint")}
              </p>
            </div>
          ) : (
            <div className="details-idle">
              <RecordButton size={84} />
              <h3>{t("lib.record_cta")}</h3>
              <p className="muted">{t("lib.record_cta_body", { hk: hotkeyFor("toggle_record") || "—" })}</p>
              <div className="hk-list">
                <HkLine label={t("transport.record")} hk={hotkeyFor("toggle_record")} />
                <HkLine label={t("transport.play")} hk={hotkeyFor("toggle_play")} />
                <HkLine label={t("transport.pause")} hk={hotkeyFor("toggle_pause")} />
                <HkLine label={t("transport.emergency")} hk={hotkeyFor("emergency_stop")} danger />
              </div>
            </div>
          )}
        </aside>
      )}

      {editing && entries.find((e) => e.id === editing) && (
        <MetaModal entry={entries.find((e) => e.id === editing)!} onClose={() => setEditing(null)} onSave={(p) => patchMeta(editing, p)} />
      )}
      {versionsOf && entries.find((e) => e.id === versionsOf) && (
        <VersionsModal entry={entries.find((e) => e.id === versionsOf)!} onClose={() => setVersionsOf(null)} />
      )}
      {menu.element}
      {confirm.element}
      {prompt.element}
      {drag.ghost}
    </div>
  );
}

function Orbitish() {
  return <span className="orbit-glyph" />;
}

function HkLine({ label, hk, danger }: { label: string; hk: string; danger?: boolean }) {
  return (
    <div className={`hk-line ${danger ? "danger" : ""}`}>
      <span>{label}</span>
      {hk ? <kbd>{hk}</kbd> : <span className="faint">—</span>}
    </div>
  );
}

function SideItem({
  icon,
  label,
  count,
  active,
  onClick,
  drop,
  over,
}: {
  icon: ReactNode;
  label: string;
  count?: number;
  active: boolean;
  onClick: () => void;
  drop?: string;
  over?: boolean;
}) {
  return (
    <button
      className={`side-item ${active ? "active" : ""} ${over ? "drop-over" : ""}`}
      onClick={onClick}
      {...(drop !== undefined ? { "data-drop-folder": drop } : {})}
    >
      {icon}
      <span className="grow ellipsis">{label}</span>
      {count !== undefined && <span className="side-count">{fmtInt(count)}</span>}
    </button>
  );
}

function FolderRow({
  node,
  depth,
  expanded,
  setExpanded,
  active,
  over,
  counts,
  onSelect,
  onMenu,
  onDragStart,
}: {
  node: FolderNode;
  depth: number;
  expanded: Record<string, boolean>;
  setExpanded: (f: (x: Record<string, boolean>) => Record<string, boolean>) => void;
  active: string | null;
  over: string | null;
  counts: LibraryEntry[];
  onSelect: (p: string) => void;
  onMenu: (e: React.MouseEvent, p: string) => void;
  onDragStart: (e: React.MouseEvent, p: string) => void;
}) {
  const open = expanded[node.path] ?? false;
  const n = counts.filter((e) => inFolder(e, node.path)).length;
  return (
    <>
      <div
        className={`side-item folder ${active === node.path ? "active" : ""} ${over === node.path ? "drop-over" : ""}`}
        style={{ paddingLeft: 8 + depth * 14 }}
        data-drop-folder={node.path}
        onClick={() => onSelect(node.path)}
        onMouseDown={(e) => onDragStart(e, node.path)}
        onContextMenu={(e) => {
          e.preventDefault();
          onMenu(e, node.path);
        }}
        role="button"
      >
        <button
          className="twisty"
          style={{ visibility: node.children.length ? "visible" : "hidden" }}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation();
            setExpanded((x) => ({ ...x, [node.path]: !open }));
          }}
        >
          {open ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        </button>
        {active === node.path ? <FolderOpen size={15} /> : <Folder size={15} />}
        <span className="grow ellipsis">{node.name}</span>
        <span className="side-count">{fmtInt(n)}</span>
      </div>
      {open &&
        node.children.map((c) => (
          <FolderRow
            key={c.path}
            node={c}
            depth={depth + 1}
            expanded={expanded}
            setExpanded={setExpanded}
            active={active}
            over={over}
            counts={counts}
            onSelect={onSelect}
            onMenu={onMenu}
            onDragStart={onDragStart}
          />
        ))}
    </>
  );
}

function MacroCard({
  entry,
  selected,
  open,
  showFolder,
  onMouseDown,
  onClick,
  onDoubleClick,
  onContextMenu,
  onPlay,
  onFavorite,
}: {
  entry: LibraryEntry;
  selected: boolean;
  open: boolean;
  showFolder: boolean;
  onMouseDown: (e: React.MouseEvent) => void;
  onClick: (e: React.MouseEvent) => void;
  onDoubleClick: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
  onPlay: () => void;
  onFavorite: () => void;
}) {
  const m = entry.manifest.meta;
  const s = entry.manifest.summary;
  const Icon = macroIcon(m.icon);
  const armed = useStore((st) => st.status.armed === entry.id);
  const playing = useStore((st) => st.status.mode === "playing" && st.status.macro_id === entry.id);
  return (
    <div
      className={`macro-card ${selected ? "selected" : ""} ${playing ? "playing" : ""}`}
      style={{ ["--accent-c" as string]: m.color }}
      onMouseDown={onMouseDown}
      onClick={onClick}
      onDoubleClick={onDoubleClick}
      onContextMenu={onContextMenu}
    >
      <div className="card-top">
        <div className="card-icon">
          <Icon size={18} />
        </div>
        <div className="card-title">
          <div className="name ellipsis" title={m.name}>
            {m.name}
          </div>
          <div className="sub faint">
            {showFolder && entry.folder ? (
              <>
                <Folder size={10} /> {entry.folder} ·{" "}
              </>
            ) : null}
            {fmtRelative(m.modified_ms)}
          </div>
        </div>
        <button
          className={`fav ${m.favorite ? "on" : ""}`}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation();
            onFavorite();
          }}
          title={m.favorite ? t("lib.unfavorite") : t("lib.favorite")}
        >
          <Star size={14} fill={m.favorite ? "currentColor" : "none"} />
        </button>
      </div>
      <Sparkline data={s.density} color={m.color} height={38} className="card-spark" />
      <div className="card-stats">
        <span className="mono strong">{fmtDuration(s.duration_us)}</span>
        <span className="faint">·</span>
        <span className="mono">{fmtInt(s.event_count)}</span>
        <span className="faint">{t("lib.events_short")}</span>
        <div className="spacer" />
        {s.has_flow && (
          <span className="chip cyan" title={t("lib.has_flow")}>
            <Workflow size={10} />
          </span>
        )}
        {open && <span className="chip" title={t("lib.is_open")}>●</span>}
        {armed && <span className="chip gold">{t("lib.armed")}</span>}
      </div>
      <div className="card-mix">
        <MixBar s={s} />
      </div>
      {m.tags.length > 0 && (
        <div className="card-tags">
          {m.tags.slice(0, 4).map((x) => (
            <span key={x} className="tag">
              #{x}
            </span>
          ))}
        </div>
      )}
      <button
        className="card-play"
        onMouseDown={(e) => e.stopPropagation()}
        onClick={(e) => {
          e.stopPropagation();
          onPlay();
        }}
        title={t("transport.play")}
      >
        <Play size={14} fill="currentColor" />
      </button>
    </div>
  );
}

/** Proportions of input kinds (keyboard / clicks / moves / wheel / pad). */
function MixBar({ s }: { s: LibraryEntry["manifest"]["summary"] }) {
  const parts = [
    { n: s.keyboard, c: "var(--gold-400)", k: t("lane.keyboard") },
    { n: s.mouse_buttons, c: "var(--cyan)", k: t("lane.buttons") },
    { n: s.mouse_moves, c: "var(--sky)", k: t("lane.moves") },
    { n: s.wheel, c: "var(--mint)", k: t("lane.wheel") },
    { n: s.gamepad, c: "var(--pink)", k: t("lane.gamepad") },
  ];
  const total = parts.reduce((a, p) => a + p.n, 0) || 1;
  return (
    <div className="mixbar" title={parts.filter((p) => p.n).map((p) => `${p.k}: ${fmtInt(p.n)}`).join("\n")}>
      {parts.map((p, i) => (p.n ? <span key={i} style={{ flex: p.n / total, background: p.c }} /> : null))}
    </div>
  );
}

function Details({
  entry,
  onPatch,
  onExport,
  onDuplicate,
  onDelete,
  onVersions,
  onEdit,
}: {
  entry: LibraryEntry;
  onPatch: (p: MetaPatch) => void;
  onExport: () => void;
  onDuplicate: () => void;
  onDelete: () => void;
  onVersions: () => void;
  onEdit: () => void;
}) {
  const openDoc = useStore((s) => s.openDoc);
  const m = entry.manifest.meta;
  const s = entry.manifest.summary;
  const env = entry.manifest.env;
  const Icon = macroIcon(m.icon);
  const [desc, setDesc] = useState(m.description);
  useEffect(() => setDesc(m.description), [m.description]);
  return (
    <div className="details">
      <div className="details-hero" style={{ ["--accent-c" as string]: m.color }}>
        <div className="hero-icon">
          <Icon size={26} />
        </div>
        <div className="grow" style={{ minWidth: 0 }}>
          <h2 className="ellipsis" title={m.name}>
            {m.name}
          </h2>
          <div className="faint">{entry.folder || t("lib.root")}</div>
        </div>
        <button className="btn ghost sm icon" onClick={onEdit} title={t("lib.edit_details")}>
          <Pencil size={14} />
        </button>
      </div>
      <Sparkline data={s.density} color={m.color} height={56} />
      <div className="insp-main-actions">
        <button className="btn primary" onClick={() => playMacro(entry.id)}>
          <Play size={14} fill="currentColor" /> {t("transport.play")}
        </button>
        <button className="btn" onClick={() => openDoc(entry.id)}>
          <AudioWaveform size={14} /> {t("lib.open_studio")}
        </button>
      </div>
      <textarea
        className="textarea details-desc"
        placeholder={t("lib.description_ph")}
        value={desc}
        onChange={(e) => setDesc(e.target.value)}
        onBlur={() => desc !== m.description && onPatch({ description: desc })}
      />
      <TagEditor tags={m.tags} onChange={(tags) => onPatch({ tags })} />
      <div className="stat-grid">
        <Stat label={t("lib.col_duration")} value={fmtDuration(s.duration_us)} />
        <Stat label={t("lib.col_events")} value={fmtInt(s.event_count)} />
        <Stat label={t("lane.keyboard")} value={fmtInt(s.keyboard)} />
        <Stat label={t("lane.buttons")} value={fmtInt(s.mouse_buttons)} />
        <Stat label={t("lane.moves")} value={fmtInt(s.mouse_moves)} />
        <Stat label={t("lane.wheel")} value={fmtInt(s.wheel)} />
        {s.gamepad > 0 && <Stat label={t("lane.gamepad")} value={fmtInt(s.gamepad)} />}
        <Stat label={t("lib.size")} value={fmtBytes(entry.size)} />
      </div>
      <div className="details-meta faint">
        <div>
          {t("lib.created")} {fmtDate(m.created_ms)}
        </div>
        <div>
          {t("lib.modified")} {fmtDate(m.modified_ms)}
        </div>
        <div>
          {t("lib.recorded_on", { os: m.source_os || "?" })}
          {env.monitors.length ? ` · ${tn("lib.monitors_one", "lib.monitors_many", env.monitors.length)}` : ""}
          {env.keyboard_layout ? ` · ${env.keyboard_layout}` : ""}
        </div>
        {entry.manifest.has_original && <div>{t("lib.has_original")}</div>}
      </div>
      <div className="details-actions">
        <button className="btn sm" onClick={onExport}>
          <Download size={13} /> {t("lib.export")}
        </button>
        <button className="btn sm" onClick={onDuplicate}>
          <Copy size={13} /> {t("lib.duplicate")}
        </button>
        <button className="btn sm" onClick={onVersions}>
          <History size={13} /> {t("lib.versions")}
        </button>
        <button className="btn sm danger" onClick={onDelete}>
          <Trash2 size={13} />
        </button>
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="stat">
      <div className="stat-value mono">{value}</div>
      <div className="stat-label">{label}</div>
    </div>
  );
}

function TagEditor({ tags, onChange }: { tags: string[]; onChange: (t: string[]) => void }) {
  const [text, setText] = useState("");
  const add = () => {
    const v = text.trim().replace(/^#/, "");
    if (v && !tags.includes(v)) onChange([...tags, v]);
    setText("");
  };
  return (
    <div className="tag-editor">
      {tags.map((x) => (
        <span key={x} className="tag">
          #{x}
          <button onClick={() => onChange(tags.filter((y) => y !== x))} aria-label={t("common.remove")}>
            <X size={10} />
          </button>
        </span>
      ))}
      <input
        className="tag-input"
        placeholder={tags.length ? "" : t("lib.add_tag")}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === "," || e.key === " ") {
            e.preventDefault();
            add();
          } else if (e.key === "Backspace" && !text && tags.length) onChange(tags.slice(0, -1));
        }}
        onBlur={add}
      />
    </div>
  );
}

function MetaModal({ entry, onClose, onSave }: { entry: LibraryEntry; onClose: () => void; onSave: (p: MetaPatch) => void }) {
  const m = entry.manifest.meta;
  const [name, setName] = useState(m.name);
  const [description, setDescription] = useState(m.description);
  const [icon, setIcon] = useState(m.icon);
  const [color, setColor] = useState(m.color);
  const [tags, setTags] = useState(m.tags);
  const save = () => {
    onSave({ name: name.trim() || m.name, description, icon, color, tags });
    onClose();
  };
  return (
    <Modal
      title={t("lib.edit_details")}
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>
            {t("common.cancel")}
          </button>
          <button className="btn primary" onClick={save}>
            {t("common.save")}
          </button>
        </>
      }
    >
      <div className="col" style={{ gap: 14 }}>
        <div className="field">
          <label>{t("lib.col_name")}</label>
          <input className="input" value={name} onChange={(e) => setName(e.target.value)} autoFocus onKeyDown={(e) => e.key === "Enter" && save()} />
        </div>
        <div className="field">
          <label>{t("lib.description")}</label>
          <textarea className="textarea" rows={3} value={description} onChange={(e) => setDescription(e.target.value)} />
        </div>
        <div className="field">
          <label>{t("lib.tags")}</label>
          <TagEditor tags={tags} onChange={setTags} />
        </div>
        <div className="field">
          <label>{t("lib.color")}</label>
          <div className="swatches">
            {MACRO_COLORS.map((c) => (
              <button key={c} className={`swatch ${c === color ? "on" : ""}`} style={{ background: c }} onClick={() => setColor(c)} aria-label={c} />
            ))}
          </div>
        </div>
        <div className="field">
          <label>{t("lib.icon")}</label>
          <div className="icon-picker">
            {Object.entries(MACRO_ICONS).map(([k, I]) => (
              <button key={k} className={`icon-pick ${k === icon ? "on" : ""}`} style={{ color: k === icon ? color : undefined }} onClick={() => setIcon(k)} title={k}>
                <I size={17} />
              </button>
            ))}
          </div>
        </div>
      </div>
    </Modal>
  );
}

function VersionsModal({ entry, onClose }: { entry: LibraryEntry; onClose: () => void }) {
  const [list, setList] = useState<Version[] | null>(null);
  const refresh = useStore((s) => s.refreshLibrary);
  const fail = useStore((s) => s.fail);
  const toast = useStore((s) => s.toast);
  const confirm = useConfirm();
  const load = useCallback(() => {
    api.versions(entry.id).then(setList).catch(() => setList([]));
  }, [entry.id]);
  useEffect(load, [load]);
  const restore = async (v: Version) => {
    const open = useStore.getState().docs[entry.id];
    const ok = await confirm.ask({
      title: t("lib.restore_version"),
      body: open?.dirty ? t("lib.restore_version_dirty") : t("lib.restore_version_body", { date: fmtDate(v.ms) }),
      confirm: t("lib.restore"),
      cancel: t("common.cancel"),
    });
    if (!ok) return;
    try {
      const payload = await api.restoreVersion(entry.id, v.ms);
      if (payload) useStore.getState().applyPayload(payload);
      await refresh();
      toast("success", t("toast.version_restored"));
      load();
    } catch (e) {
      fail(e, "error.library");
    }
  };
  return (
    <Modal title={`${t("lib.versions")} — ${entry.manifest.meta.name}`} icon={<History size={18} />} onClose={onClose}>
      <p className="muted" style={{ marginBottom: 12 }}>
        {t("lib.versions_body")}
      </p>
      {!list ? (
        <div className="spinner" />
      ) : list.length === 0 ? (
        <div className="empty">{t("lib.no_versions")}</div>
      ) : (
        <table className="table">
          <tbody>
            {list.map((v) => (
              <tr key={v.ms}>
                <td>{fmtDate(v.ms)}</td>
                <td className="faint">{fmtRelative(v.ms)}</td>
                <td className="mono faint">{fmtBytes(v.size)}</td>
                <td style={{ textAlign: "right" }}>
                  <button className="btn sm" onClick={() => restore(v)}>
                    <ArchiveRestore size={13} /> {t("lib.restore")}
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {confirm.element}
    </Modal>
  );
}

function TrashList({ items, onRestore }: { items: TrashItem[]; onRestore: (it: TrashItem) => void }) {
  if (!items.length)
    return (
      <div className="empty">
        <Trash2 size={30} className="faint" />
        <h3>{t("lib.trash_empty")}</h3>
        <p className="muted">{t("lib.trash_hint")}</p>
      </div>
    );
  return (
    <table className="table lib-table">
      <thead>
        <tr>
          <th />
          <th>{t("lib.col_name")}</th>
          <th>{t("lib.deleted")}</th>
          <th />
        </tr>
      </thead>
      <tbody>
        {[...items]
          .sort((a, b) => b.deleted_ms - a.deleted_ms)
          .map((it) => (
            <tr key={`${it.deleted_ms}/${it.name}`}>
              <td style={{ width: 28 }}>{it.is_dir ? <Folder size={15} /> : <AudioWaveform size={15} />}</td>
              <td>{it.name.replace(/\.chud$/i, "")}</td>
              <td className="muted" title={fmtDate(it.deleted_ms)}>
                {fmtRelative(it.deleted_ms)}
              </td>
              <td style={{ textAlign: "right" }}>
                <button className="btn sm" onClick={() => onRestore(it)}>
                  <ArchiveRestore size={13} /> {t("lib.restore")}
                </button>
              </td>
            </tr>
          ))}
      </tbody>
    </table>
  );
}

function EmptyLibrary({ onCreate, onImport }: { onCreate: () => void; onImport: () => void }) {
  return (
    <div className="empty-hero">
      <div className="empty-orbit">
        <RecordButton size={110} />
      </div>
      <h2>{t("lib.empty_title")}</h2>
      <p className="muted">{t("lib.empty_body", { hk: hotkeyFor("toggle_record") || "—" })}</p>
      <div className="row" style={{ gap: 10, justifyContent: "center" }}>
        <button className="btn" onClick={onImport}>
          <Upload size={14} /> {t("lib.import")}
        </button>
        <button className="btn" onClick={onCreate}>
          <Plus size={14} /> {t("lib.new_macro")}
        </button>
      </div>
      <p className="faint" style={{ marginTop: 18 }}>
        {t("lib.privacy_note")}
      </p>
    </div>
  );
}
