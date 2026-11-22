// Small reusable controls.
import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";

export function Toggle({
  value,
  onChange,
  disabled,
  label,
}: {
  value: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
  label?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={value}
      aria-label={label}
      className={`toggle ${value ? "on" : ""}`}
      disabled={disabled}
      onClick={() => onChange(!value)}
    />
  );
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: { value: T; label: ReactNode; title?: string }[];
  onChange: (v: T) => void;
}) {
  return (
    <div className="segmented" role="radiogroup">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          title={o.title}
          className={o.value === value ? "active" : ""}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** Numeric input that commits on blur / Enter and accepts decimal commas. */
export function NumberInput({
  value,
  onChange,
  min,
  max,
  step = 1,
  suffix,
  className = "",
  width = 96,
  disabled,
  placeholder,
}: {
  value: number | null;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
  step?: number;
  suffix?: string;
  className?: string;
  width?: number;
  disabled?: boolean;
  placeholder?: string;
}) {
  const [text, setText] = useState(value === null ? "" : String(value));
  const [focus, setFocus] = useState(false);
  useEffect(() => {
    if (!focus) setText(value === null ? "" : String(+value.toFixed(6)));
  }, [value, focus]);
  const commit = () => {
    const v = parseFloat(text.replace(",", "."));
    if (Number.isFinite(v)) {
      let c = v;
      if (min !== undefined) c = Math.max(min, c);
      if (max !== undefined) c = Math.min(max, c);
      onChange(c);
      setText(String(c));
    } else setText(value === null ? "" : String(value));
  };
  return (
    <div className="num-input" style={{ width }}>
      <input
        className={`input sm mono ${className}`}
        value={text}
        disabled={disabled}
        placeholder={placeholder}
        inputMode="decimal"
        onFocus={() => setFocus(true)}
        onBlur={() => {
          setFocus(false);
          commit();
        }}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          if (e.key === "Escape") {
            setText(value === null ? "" : String(value));
            (e.target as HTMLInputElement).blur();
          }
          if (e.key === "ArrowUp" || e.key === "ArrowDown") {
            e.preventDefault();
            const cur = parseFloat(text.replace(",", ".")) || 0;
            const mult = e.shiftKey ? 10 : 1;
            let next = cur + (e.key === "ArrowUp" ? step : -step) * mult;
            if (min !== undefined) next = Math.max(min, next);
            if (max !== undefined) next = Math.min(max, next);
            next = +next.toFixed(6);
            setText(String(next));
            onChange(next);
          }
        }}
      />
      {suffix && <span className="num-suffix">{suffix}</span>}
    </div>
  );
}

export function Slider({
  value,
  min,
  max,
  step,
  onChange,
}: {
  value: number;
  min: number;
  max: number;
  step: number;
  onChange: (v: number) => void;
}) {
  const p = ((value - min) / (max - min)) * 100;
  return (
    <input
      type="range"
      className="slider"
      min={min}
      max={max}
      step={step}
      value={value}
      style={{ ["--p" as string]: `${p}%` }}
      onChange={(e) => onChange(parseFloat(e.target.value))}
    />
  );
}

export function FieldRow({ label, hint, children }: { label: ReactNode; hint?: ReactNode; children: ReactNode }) {
  return (
    <div className="field-row">
      <div className="label">
        <span>{label}</span>
        {hint && <span>{hint}</span>}
      </div>
      <div className="row" style={{ justifyContent: "flex-end" }}>
        {children}
      </div>
    </div>
  );
}

export function Modal({
  title,
  icon,
  onClose,
  children,
  footer,
  wide,
}: {
  title: ReactNode;
  icon?: ReactNode;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  wide?: boolean;
}) {
  useEffect(() => {
    const k = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", k);
    return () => window.removeEventListener("keydown", k);
  }, [onClose]);
  return createPortal(
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className={`modal ${wide ? "wide" : ""}`} role="dialog" aria-modal>
        <div className="modal-head">
          {icon}
          <h2 className="grow">{title}</h2>
          <button className="btn ghost icon" onClick={onClose} aria-label="Close">
            <X size={16} />
          </button>
        </div>
        <div className="modal-body">{children}</div>
        {footer && <div className="modal-foot">{footer}</div>}
      </div>
    </div>,
    document.body,
  );
}

export interface MenuEntry {
  label?: ReactNode;
  icon?: ReactNode;
  shortcut?: string;
  danger?: boolean;
  disabled?: boolean;
  onClick?: () => void;
  separator?: boolean;
  title?: string;
}

/** Context menu at a screen position; closes on outside click / Escape. */
export function ContextMenu({
  x,
  y,
  items,
  onClose,
  above = false,
}: {
  x: number;
  y: number;
  items: MenuEntry[];
  onClose: () => void;
  /** Opens upward from `y` (menus anchored to the transport bar). */
  above?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y });
  useEffect(() => {
    const el = ref.current;
    if (el) {
      const r = el.getBoundingClientRect();
      setPos({
        x: Math.max(8, Math.min(x, window.innerWidth - r.width - 8)),
        y: above ? Math.max(8, y - r.height) : Math.min(y, window.innerHeight - r.height - 8),
      });
    }
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("mousedown", close, true);
    window.addEventListener("keydown", key);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener("mousedown", close, true);
      window.removeEventListener("keydown", key);
      window.removeEventListener("blur", onClose);
    };
  }, [x, y, onClose, above]);
  return createPortal(
    <div ref={ref} className="menu" style={{ left: pos.x, top: pos.y }} onContextMenu={(e) => e.preventDefault()}>
      {items.map((it, i) =>
        it.separator ? (
          <div key={i} className="menu-sep" />
        ) : it.title ? (
          <div key={i} className="menu-title">
            {it.title}
          </div>
        ) : (
          <button
            key={i}
            className={`menu-item ${it.danger ? "danger" : ""}`}
            disabled={it.disabled}
            onClick={() => {
              onClose();
              it.onClick?.();
            }}
          >
            {it.icon}
            <span>{it.label}</span>
            {it.shortcut && <span className="shortcut">{it.shortcut}</span>}
          </button>
        ),
      )}
    </div>,
    document.body,
  );
}

export function useContextMenu() {
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuEntry[]; above?: boolean } | null>(null);
  const el = menu ? <ContextMenu {...menu} onClose={() => setMenu(null)} /> : null;
  return {
    open: (x: number, y: number, items: MenuEntry[], above = false) => setMenu({ x, y, items, above }),
    element: el,
  };
}

/** Simple confirmation dialog as a promise-returning hook. */
export function useConfirm() {
  const [state, setState] = useState<{
    title: string;
    body: ReactNode;
    confirm: string;
    cancel: string;
    danger?: boolean;
    resolve: (v: boolean) => void;
  } | null>(null);
  const ask = (o: { title: string; body: ReactNode; confirm: string; cancel: string; danger?: boolean }) =>
    new Promise<boolean>((resolve) => setState({ ...o, resolve }));
  const close = (v: boolean) => {
    state?.resolve(v);
    setState(null);
  };
  const element = state ? (
    <Modal
      title={state.title}
      onClose={() => close(false)}
      footer={
        <>
          <button className="btn" onClick={() => close(false)}>
            {state.cancel}
          </button>
          <button className={`btn ${state.danger ? "danger" : "primary"}`} onClick={() => close(true)} autoFocus>
            {state.confirm}
          </button>
        </>
      }
    >
      <div className="muted" style={{ lineHeight: 1.6 }}>
        {state.body}
      </div>
    </Modal>
  ) : null;
  return { ask, element };
}

/** Prompt for a text value. */
export function usePrompt() {
  const [state, setState] = useState<{
    title: string;
    value: string;
    confirm: string;
    cancel: string;
    resolve: (v: string | null) => void;
  } | null>(null);
  const ask = (o: { title: string; value: string; confirm: string; cancel: string }) =>
    new Promise<string | null>((resolve) => setState({ ...o, resolve }));
  const close = (v: string | null) => {
    state?.resolve(v);
    setState(null);
  };
  const element = state ? (
    <Modal
      title={state.title}
      onClose={() => close(null)}
      footer={
        <>
          <button className="btn" onClick={() => close(null)}>
            {state.cancel}
          </button>
          <button className="btn primary" onClick={() => close(state.value)}>
            {state.confirm}
          </button>
        </>
      }
    >
      <input
        className="input"
        style={{ width: "100%" }}
        autoFocus
        value={state.value}
        onChange={(e) => setState({ ...state, value: e.target.value })}
        onKeyDown={(e) => {
          if (e.key === "Enter") close(state.value);
        }}
        onFocus={(e) => e.target.select()}
      />
    </Modal>
  ) : null;
  return { ask, element };
}
