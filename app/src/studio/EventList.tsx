// Virtualized list of all events (precise selection and inspection).
import { useEffect, useMemo, useRef, useState } from "react";
import { fmtInt, t, useLang } from "../i18n";
import { category, describe, type Category } from "../lib/events";
import { fmtDuration, fmtTime } from "../lib/format";
import { useStore, type DocState } from "../lib/store";
import type { EventId } from "../lib/types";

const ROW = 24;
const CAT_COLOR: Record<Category, string> = {
  keyboard: "#f9c83a",
  mouse_button: "#38d6ff",
  mouse_move: "#82b6ff",
  mouse_raw: "#a99bff",
  wheel: "#8fe8c2",
  pad_button: "#ff86b0",
  pad_axis: "#ff86b0",
};

type Filter = "all" | "keyboard" | "mouse" | "moves" | "gamepad";

export function EventList({ doc, onReveal }: { doc: DocState; onReveal: (t: number) => void }) {
  useLang();
  const patchDoc = useStore((s) => s.patchDoc);
  const scroller = useRef<HTMLDivElement>(null);
  const [top, setTop] = useState(0);
  const [height, setHeight] = useState(400);
  const [filter, setFilter] = useState<Filter>("all");
  const anchor = useRef<number | null>(null);

  const rows = useMemo(() => {
    const keep = (c: Category) =>
      filter === "all" ||
      (filter === "keyboard" && c === "keyboard") ||
      (filter === "mouse" && (c === "mouse_button" || c === "wheel")) ||
      (filter === "moves" && (c === "mouse_move" || c === "mouse_raw")) ||
      (filter === "gamepad" && (c === "pad_button" || c === "pad_axis"));
    const out: number[] = [];
    doc.events.forEach((e, i) => keep(category(e)) && out.push(i));
    return out;
  }, [doc.events, filter]);

  useEffect(() => {
    const el = scroller.current!;
    const ro = new ResizeObserver(() => setHeight(el.clientHeight));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Keep a single selected event in view.
  useEffect(() => {
    if (doc.selection.size !== 1 && doc.selection.size !== 2) return;
    const id = [...doc.selection][0];
    const r = rows.findIndex((i) => doc.events[i].id === id);
    const el = scroller.current;
    if (r < 0 || !el) return;
    const y = r * ROW;
    if (y < el.scrollTop || y > el.scrollTop + el.clientHeight - ROW) el.scrollTop = Math.max(0, y - el.clientHeight / 3);
  }, [doc.selection, rows, doc.events]);

  const first = Math.max(0, Math.floor(top / ROW) - 8);
  const last = Math.min(rows.length, Math.ceil((top + height) / ROW) + 8);

  const click = (e: React.MouseEvent, r: number) => {
    const id = doc.events[rows[r]].id;
    let sel: Set<EventId>;
    if (e.shiftKey && anchor.current !== null) {
      const [a, b] = [Math.min(anchor.current, r), Math.max(anchor.current, r)];
      sel = new Set(e.ctrlKey ? doc.selection : []);
      for (let k = a; k <= b; k++) sel.add(doc.events[rows[k]].id);
    } else if (e.ctrlKey || e.metaKey) {
      sel = new Set(doc.selection);
      if (sel.has(id)) sel.delete(id);
      else sel.add(id);
      anchor.current = r;
    } else {
      sel = new Set([id]);
      anchor.current = r;
    }
    patchDoc(doc.id, { selection: sel, cursor: doc.events[rows[r]].t });
  };

  return (
    <div className="event-list">
      <div className="event-filter">
        {(["all", "keyboard", "mouse", "moves", "gamepad"] as Filter[]).map((f) => (
          <button key={f} className={`chip ${filter === f ? "active" : ""}`} onClick={() => setFilter(f)}>
            {t(`events.filter_${f}`)}
          </button>
        ))}
        <span className="faint mono" style={{ marginLeft: "auto" }}>
          {fmtInt(rows.length)}
        </span>
      </div>
      <div ref={scroller} className="event-scroll" onScroll={(e) => setTop((e.target as HTMLDivElement).scrollTop)}>
        <div style={{ height: rows.length * ROW, position: "relative" }}>
          {Array.from({ length: Math.max(0, last - first) }, (_, k) => {
            const r = first + k;
            const i = rows[r];
            const e = doc.events[i];
            const prev = i > 0 ? doc.events[i - 1] : null;
            const c = category(e);
            return (
              <div
                key={e.id}
                className={`event-row ${doc.selection.has(e.id) ? "selected" : ""}`}
                style={{ top: r * ROW }}
                onClick={(ev) => click(ev, r)}
                onDoubleClick={() => onReveal(e.t)}
              >
                <span className="ev-dot" style={{ background: CAT_COLOR[c] }} />
                <span className="ev-time mono">{fmtTime(e.t, 3)}</span>
                <span className="ev-gap mono faint">{prev ? `+${fmtDuration(e.t - prev.t)}` : ""}</span>
                <span className="ev-desc ellipsis">{describe(e)}</span>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
