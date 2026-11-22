// Editors for "find on screen": what to look for (image, colour, text),
// where (whole screen or an area), and a live test.
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Camera, CircleCheck, CircleX, Crosshair, LoaderCircle, Pipette, ScanSearch, SquareDashed, TriangleAlert, Upload, X } from "lucide-react";
import { useState } from "react";
import { ScreenPicker, type PickMode, type PickResult } from "../components/ScreenPicker";
import { NumberInput, Segmented, Slider, Toggle, useConfirm } from "../components/ui";
import { t } from "../i18n";
import { api, errorText } from "../lib/api";
import { useStore, type DocState } from "../lib/store";
import type { DocImage, Region, ScreenTest, Target, TargetKind } from "../lib/types";
import { defaultTarget, imageUses } from "./tree";

function useCaps() {
  const caps = useStore((s) => s.info?.capabilities);
  return { capture: caps?.screen_capture ?? false, ocr: caps?.text_recognition ?? false, os: caps?.os ?? "" };
}

/** A button that opens the screen picker. */
export function PickButton({
  mode,
  onPick,
  label,
  small = true,
  primary,
}: {
  mode: PickMode;
  onPick: (r: PickResult) => unknown;
  label: string;
  small?: boolean;
  primary?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const { capture } = useCaps();
  const Icon = mode === "image" ? Camera : mode === "area" ? SquareDashed : mode === "point" ? Crosshair : Pipette;
  return (
    <>
      <button
        className={`btn ${small ? "sm" : ""} ${primary ? "primary" : ""}`}
        disabled={!capture}
        title={capture ? undefined : t("screen.capture_unavailable")}
        onClick={() => setOpen(true)}
      >
        <Icon size={13} /> {label}
      </button>
      {open && <ScreenPicker mode={mode} onPick={onPick} onClose={() => setOpen(false)} />}
    </>
  );
}

function ImageGallery({ doc, value, onChange }: { doc: DocState; value: string; onChange: (id: string) => void }) {
  const patchDoc = useStore((s) => s.patchDoc);
  const fail = useStore((s) => s.fail);
  const confirm = useConfirm();
  const added = (img: DocImage) => {
    const cur = useStore.getState().docs[doc.id];
    patchDoc(doc.id, { images: [...(cur?.images ?? []), img], dirty: true });
    onChange(img.id);
  };
  const removeImage = async (img: DocImage) => {
    const uses = imageUses(doc.flow?.blocks ?? [], img.id);
    if (uses > 0) {
      const ok = await confirm.ask({
        title: t("screen.remove_image"),
        body: t("screen.remove_image_used", { n: uses }),
        confirm: t("common.remove"),
        cancel: t("common.cancel"),
        danger: true,
      });
      if (!ok) return;
    }
    try {
      await api.docRemoveImage(doc.id, img.id);
      const cur = useStore.getState().docs[doc.id];
      patchDoc(doc.id, { images: (cur?.images ?? []).filter((i) => i.id !== img.id), dirty: true });
    } catch (e) {
      fail(e, "error.edit");
    }
  };
  const selected = doc.images.find((i) => i.id === value);
  return (
    <div className="img-gallery">
      {selected ? (
        <div className="img-current">
          <img src={selected.url} alt="" />
          <span className="mono faint">
            {selected.w} × {selected.h}
          </span>
          <button className="img-current-x btn ghost icon sm" onClick={() => removeImage(selected)} aria-label={t("screen.remove_image")} title={t("screen.remove_image")}>
            <X size={13} />
          </button>
        </div>
      ) : (
        <div className="img-current empty faint">{value ? t("screen.image_missing") : t("screen.no_image")}</div>
      )}
      {doc.images.length > 1 || (doc.images.length === 1 && !selected) ? (
        <div className="img-thumbs" role="listbox" aria-label={t("screen.images")}>
          {doc.images.map((img) => (
            <div key={img.id} className={`img-thumb ${img.id === value ? "on" : ""}`} role="option" aria-selected={img.id === value}>
              <button onClick={() => onChange(img.id)} title={`${img.w} × ${img.h}`}>
                <img src={img.url} alt="" />
              </button>
              <button className="img-thumb-x" onClick={() => removeImage(img)} aria-label={t("screen.remove_image")} title={t("screen.remove_image")}>
                <X size={10} />
              </button>
            </div>
          ))}
        </div>
      ) : null}
      <div className="row wrap" style={{ gap: 6 }}>
        <PickButton
          mode="image"
          primary={!selected}
          label={t("screen.capture_image")}
          onPick={async (r) => {
            if (r.mode === "point") return;
            added(await api.docAddImage(doc.id, r.region));
          }}
        />
        <button
          className="btn sm"
          onClick={async () => {
            const path = await openDialog({
              multiple: false,
              filters: [{ name: t("screen.picture_files"), extensions: ["png", "jpg", "jpeg", "bmp", "gif", "webp"] }],
            });
            if (typeof path !== "string") return;
            try {
              added(await api.docImportImage(doc.id, path));
            } catch (e) {
              fail(e, "error.edit");
            }
          }}
        >
          <Upload size={13} /> {t("screen.import_image")}
        </button>
      </div>
      {confirm.element}
    </div>
  );
}

/** What to look for. */
export function TargetEditor({ doc, target, onChange }: { doc: DocState; target: Target; onChange: (t: Target) => void }) {
  const { ocr, os } = useCaps();
  return (
    <div className="target-editor">
      <Segmented<TargetKind>
        value={target.kind}
        onChange={(k) => k !== target.kind && onChange(defaultTarget(k, doc.images.at(-1)?.id))}
        options={(["image", "text", "color"] as TargetKind[]).map((k) => ({ value: k, label: t(`screen.kind_${k}`) }))}
      />
      {target.kind === "image" && (
        <>
          <ImageGallery doc={doc} value={target.image} onChange={(image) => onChange({ ...target, image })} />
          <div className="prop">
            <label className="prop-label row" style={{ justifyContent: "space-between" }}>
              {t("screen.similarity")} <span className="mono">{Math.round(target.threshold * 100)} %</span>
            </label>
            <Slider value={target.threshold} min={0.5} max={1} step={0.01} onChange={(v) => onChange({ ...target, threshold: v })} />
            <div className="prop-hint">{t("screen.similarity_hint")}</div>
          </div>
          <div className="row" style={{ gap: 8 }}>
            <Toggle value={target.any_scale} onChange={(v) => onChange({ ...target, any_scale: v })} label={t("screen.any_scale")} />
            <span>{t("screen.any_scale")}</span>
          </div>
          <div className="prop-hint">{t("screen.any_scale_hint")}</div>
        </>
      )}
      {target.kind === "text" && (
        <>
          <TextField value={target.text} onChange={(text) => onChange({ ...target, text })} placeholder={t("screen.text_ph")} />
          <div className="row" style={{ gap: 8 }}>
            <Toggle value={target.match_case} onChange={(v) => onChange({ ...target, match_case: v })} label={t("screen.match_case")} />
            <span>{t("screen.match_case")}</span>
          </div>
          <div className={ocr ? "prop-hint" : "prop-error"}>
            {ocr ? t("screen.text_hint") : os === "windows" ? t("screen.ocr_missing_windows") : t("screen.ocr_unavailable")}
          </div>
        </>
      )}
      {target.kind === "color" && (
        <>
          <div className="row" style={{ gap: 6 }}>
            <input type="color" className="color-input" value={target.color} onChange={(e) => onChange({ ...target, color: e.target.value })} />
            <code className="mono">{target.color}</code>
            <PickButton mode="point" label={t("screen.pick_color")} onPick={(r) => r.mode === "point" && onChange({ ...target, color: r.color })} />
          </div>
          <div className="row" style={{ gap: 6 }}>
            <span className="faint">{t("cond.tolerance")}</span>
            <NumberInput value={target.tolerance} min={0} max={255} step={1} width={80} onChange={(v) => onChange({ ...target, tolerance: Math.round(v) })} />
          </div>
          <div className="prop-hint">{t("screen.color_hint")}</div>
        </>
      )}
    </div>
  );
}

function TextField({ value, onChange, placeholder }: { value: string; onChange: (v: string) => void; placeholder?: string }) {
  const [text, setText] = useState(value);
  const [prev, setPrev] = useState(value);
  if (value !== prev) {
    setPrev(value);
    setText(value);
  }
  return (
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

/** Where to look: every screen, or an area. */
export function RegionEditor({ region, onChange }: { region: Region | null; onChange: (r: Region | null) => void }) {
  const [picking, setPicking] = useState(false);
  return (
    <div className="region-editor">
      <Segmented<"all" | "area">
        value={region ? "area" : "all"}
        onChange={(v) => (v === "all" ? onChange(null) : setPicking(true))}
        options={[
          { value: "all", label: t("screen.everywhere") },
          { value: "area", label: t("screen.in_area") },
        ]}
      />
      {region && (
        <div className="row" style={{ gap: 6 }}>
          <code className="mono faint">
            {region.x}, {region.y} · {region.w} × {region.h}
          </code>
          <button className="btn sm ghost" onClick={() => setPicking(true)}>
            <SquareDashed size={12} /> {t("screen.change_area")}
          </button>
        </div>
      )}
      <div className="prop-hint">{t("screen.area_hint")}</div>
      {picking && <ScreenPicker mode="area" onPick={(r) => r.mode !== "point" && onChange(r.region)} onClose={() => setPicking(false)} />}
    </div>
  );
}

/** Runs the search now and shows where (or how close) it was found. */
export function ScreenTestButton({ doc, target, region }: { doc: DocState; target: Target; region: Region | null }) {
  const [busy, setBusy] = useState(false);
  const [res, setRes] = useState<ScreenTest | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const { capture } = useCaps();
  const run = async () => {
    setBusy(true);
    setErr(null);
    try {
      setRes(await api.screenTest(doc.id, target, region));
    } catch (e) {
      setRes(null);
      setErr(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const f = res?.found;
  const best = res?.best;
  return (
    <div className="screen-test">
      <button className="btn sm" disabled={busy || !capture} onClick={run} title={t("screen.test_hint")}>
        {busy ? <LoaderCircle size={13} className="spin" /> : <ScanSearch size={13} />} {t("screen.test")}
      </button>
      {err && (
        <div className="prop-error">
          <TriangleAlert size={12} /> {err}
        </div>
      )}
      {res && (
        <div className={`screen-test-result ${f ? "ok" : "miss"}`}>
          <div className="row" style={{ gap: 6 }}>
            {f ? <CircleCheck size={14} /> : <CircleX size={14} />}
            <strong>{f ? t("screen.found") : t("screen.not_found")}</strong>
            <span className="faint mono">{Math.round(res.ms)} ms</span>
          </div>
          {f && (
            <div className="mono faint">
              {t("screen.found_at", { x: f.x + Math.floor(f.w / 2), y: f.y + Math.floor(f.h / 2) })}
              {target.kind === "image" && ` · ${Math.round(f.score * 100)} %`}
            </div>
          )}
          {!f && best && target.kind === "image" && (
            <div className="faint">{t("screen.closest", { p: Math.round(best.score * 100), x: best.x, y: best.y })}</div>
          )}
          {res.words !== null && <div className="faint">{t("screen.words_read", { n: res.words })}</div>}
          {res.preview && <img className="screen-test-preview" src={res.preview} alt="" />}
        </div>
      )}
    </div>
  );
}
