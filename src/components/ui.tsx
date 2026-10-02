import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { useApp } from "../lib/store";
import { Icon } from "./Icon";

export function Orb({ size = 22 }: { size?: number }) {
  return <span className="orb" style={{ width: size, height: size, ["--s" as string]: `${size}px` }} aria-hidden="true" />;
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  id,
  label,
}: {
  value: T;
  options: { value: T; label: ReactNode }[];
  onChange(v: T): void;
  id: string;
  label?: string;
}) {
  return (
    <div className="segmented" role="radiogroup" aria-label={label}>
      {options.map((o) => (
        <button
          key={o.value}
          role="radio"
          aria-checked={o.value === value}
          className={o.value === value ? "on" : ""}
          onClick={() => onChange(o.value)}
        >
          {o.value === value && (
            <motion.span layoutId={`seg-${id}`} className="seg-pill" transition={{ type: "spring", stiffness: 500, damping: 38 }} />
          )}
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Switch({ on, onChange, label }: { on: boolean; onChange(v: boolean): void; label: string }) {
  return <button className={`switch ${on ? "on" : ""}`} role="switch" aria-checked={on} aria-label={label} onClick={() => onChange(!on)} />;
}

export function Toasts() {
  const toasts = useApp((s) => s.toasts);
  return (
    <div className="toasts" role="status" aria-live="polite">
      <AnimatePresence>
        {toasts.map((t) => (
          <motion.div
            key={t.id}
            className={`toast ${t.kind === "error" ? "error" : ""}`}
            initial={{ opacity: 0, y: 16, scale: 0.96 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 8, scale: 0.98 }}
            transition={{ type: "spring", stiffness: 420, damping: 30 }}
          >
            {t.kind === "light" && <span className="toast-glow" />}
            {t.text}
          </motion.div>
        ))}
      </AnimatePresence>
    </div>
  );
}

export function Sheet({
  open,
  onClose,
  title,
  children,
  footer,
  width,
}: {
  open: boolean;
  onClose(): void;
  title: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  width?: number;
}) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);
  return (
    <AnimatePresence>
      {open && (
        <motion.div
          className="scrim"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          onMouseDown={(e) => e.target === e.currentTarget && onClose()}
        >
          <motion.div
            className="sheet"
            role="dialog"
            aria-modal="true"
            style={width ? { width: `min(${width}px, 100%)` } : undefined}
            initial={{ opacity: 0, y: 24, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 12, scale: 0.98 }}
            transition={{ type: "spring", stiffness: 380, damping: 32 }}
          >
            <div className="sheet-head">
              <h2>{title}</h2>
              <button className="icon-btn" onClick={onClose} aria-label="Fermer">
                <Icon name="close" />
              </button>
            </div>
            <div className="sheet-body">{children}</div>
            {footer && <div className="sheet-foot">{footer}</div>}
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

/** Lumière qui suit le pointeur sur une surface (.glow-surface). */
export function useGlow<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const onMouseMove = useCallback((e: React.MouseEvent) => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    el.style.setProperty("--mx", `${e.clientX - r.left}px`);
    el.style.setProperty("--my", `${e.clientY - r.top}px`);
  }, []);
  return { ref, onMouseMove };
}

export function Menu({
  anchor,
  open,
  onClose,
  children,
  align = "left",
}: {
  anchor: ReactNode;
  open: boolean;
  onClose(): void;
  children: ReactNode;
  align?: "left" | "right";
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open, onClose]);
  return (
    <div ref={ref} style={{ position: "relative" }}>
      {anchor}
      <AnimatePresence>
        {open && (
          <motion.div
            className="menu"
            role="menu"
            style={{ top: "calc(100% + 6px)", [align]: 0 }}
            initial={{ opacity: 0, y: -6, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -4, scale: 0.98 }}
            transition={{ duration: 0.16 }}
          >
            {children}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/** Nombre qui monte doucement vers sa valeur. */
export function CountUp({ value, duration = 900 }: { value: number; duration?: number }) {
  const [shown, setShown] = useState(value);
  const from = useRef(value);
  useEffect(() => {
    const start = performance.now();
    const a = from.current;
    let raf = 0;
    const tick = (now: number) => {
      const p = Math.min(1, (now - start) / duration);
      const eased = 1 - Math.pow(1 - p, 3);
      setShown(Math.round(a + (value - a) * eased));
      if (p < 1) raf = requestAnimationFrame(tick);
      else from.current = value;
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [value, duration]);
  return <>{shown.toLocaleString("fr-FR")}</>;
}
