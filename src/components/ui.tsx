import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";
import { useApp } from "../lib/store";
import { Icon } from "./Icon";
import { formatNumber, t } from "../lib/i18n";
import { timeParts } from "../lib/progress";

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
  /** `title` : le nom d'une option qui n'est qu'une icône (bulle d'aide, lecteurs d'écran) */
  options: { value: T; label: ReactNode; title?: string }[];
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
          aria-label={o.title}
          title={o.title}
          className={`${o.value === value ? "on" : ""} ${o.title ? "seg-icon" : ""}`}
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
            {t.action && (
              <button
                className="toast-action"
                onClick={() => {
                  t.action!.run();
                  useApp.setState((s) => ({ toasts: s.toasts.filter((x) => x.id !== t.id) }));
                }}
              >
                {t.action.label}
              </button>
            )}
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
              <button className="icon-btn" onClick={onClose} aria-label={t("Fermer", "Close")}>
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

/** Place d'un menu ouvert, en coordonnées de la fenêtre. */
interface MenuSpot {
  top: number;
  left: number;
  /** ouvert au-dessus de son bouton, faute de place dessous */
  up: boolean;
}

/**
 * Menu déroulant. Il est posé à la racine de la page (portail) et placé d'après son
 * bouton : rendu dans la carte, il passait sous la carte suivante (chaque case animée
 * forme sa propre couche) et se faisait couper par les rangées qui défilent.
 */
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
  const box = useRef<HTMLDivElement>(null);
  const [spot, setSpot] = useState<MenuSpot | null>(null);
  const place = useCallback(() => {
    const a = ref.current?.getBoundingClientRect();
    const el = box.current;
    if (!a || !el) return;
    // offsetWidth et offsetHeight ignorent l'animation d'entrée (échelle)
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    const edge = 8;
    const gap = 6;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const up = a.bottom + gap + h > vh - edge && a.top - gap - h >= edge;
    const top = up ? a.top - gap - h : Math.max(edge, Math.min(a.bottom + gap, vh - edge - h));
    const left = Math.max(edge, Math.min(align === "right" ? a.right - w : a.left, vw - edge - w));
    setSpot((s) => (s && s.top === top && s.left === left && s.up === up ? s : { top, left, up }));
  }, [align]);
  useLayoutEffect(() => {
    if (open && !document.documentElement.classList.contains("compact")) place();
    else setSpot(null);
  }, [open, place]);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent | TouchEvent) => {
      const target = e.target as Node;
      if (ref.current?.contains(target) || box.current?.contains(target)) return;
      onClose();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    // la page défile ou change de taille : le menu suit son bouton (pas la feuille de l'iPhone)
    const follow = () => !document.documentElement.classList.contains("compact") && place();
    window.addEventListener("mousedown", onDown);
    window.addEventListener("touchstart", onDown, { passive: true });
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", follow);
    window.addEventListener("scroll", follow, true);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("touchstart", onDown);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", follow);
      window.removeEventListener("scroll", follow, true);
    };
  }, [open, onClose, place]);
  // iPhone (écran étroit) : une feuille qui monte du bas, sous le pouce, qu'on referme en la tirant
  const sheet = typeof document !== "undefined" && document.documentElement.classList.contains("compact");
  if (sheet)
    return (
      <div ref={ref} className={`menu-anchor ${open ? "open" : ""}`} style={{ position: "relative" }}>
        {anchor}
        {createPortal(
          <AnimatePresence>
            {open && (
              <motion.div key="scrim" className="m-scrim menu-scrim" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} onClick={onClose} />
            )}
            {open && (
              <motion.div
                key="sheet"
                ref={box}
                className="menu menu-sheet"
                role="menu"
                initial={{ y: "110%" }}
                animate={{ y: 0 }}
                exit={{ y: "110%" }}
                transition={{ type: "spring", stiffness: 520, damping: 42 }}
                drag="y"
                dragConstraints={{ top: 0, bottom: 0 }}
                dragElastic={{ top: 0.05, bottom: 0.6 }}
                onDragEnd={(_, info) => (info.offset.y > 70 || info.velocity.y > 500) && onClose()}
              >
                <span className="m-grabber" aria-hidden="true" />
                {children}
              </motion.div>
            )}
          </AnimatePresence>,
          document.body,
        )}
      </div>
    );
  return (
    <div ref={ref} className={`menu-anchor ${open ? "open" : ""}`} style={{ position: "relative" }}>
      {anchor}
      {createPortal(
        <AnimatePresence>
          {open && (
            <motion.div
              ref={box}
              className="menu"
              role="menu"
              style={{
                position: "fixed",
                top: spot?.top ?? 0,
                left: spot?.left ?? 0,
                visibility: spot ? "visible" : "hidden",
                transformOrigin: `${align === "right" ? "right" : "left"} ${spot?.up ? "bottom" : "top"}`,
              }}
              initial={{ opacity: 0, y: spot?.up ? 6 : -6, scale: 0.97 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, y: spot?.up ? 4 : -4, scale: 0.98 }}
              transition={{ duration: 0.16 }}
            >
              {children}
            </motion.div>
          )}
        </AnimatePresence>,
        document.body,
      )}
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
  return <>{formatNumber(shown)}</>;
}

/** Durée mise en forme : chiffres en grand, unités en petit (« 1 h 24 min »). */
export function Duration({ secs }: { secs: number }) {
  return (
    <span className="dur">
      {timeParts(secs).map(([v, u], i) => (
        <span key={i}>
          {i > 0 && " "}
          {v}
          <small>{u}</small>
        </span>
      ))}
    </span>
  );
}

/** « Revenir en haut » : apparaît quand on est bien descendu dans une vue qui défile. */
export function ScrollTop({ target, after = 480 }: { target: RefObject<HTMLElement | null>; after?: number }) {
  const [shown, setShown] = useState(false);
  const still = useReducedMotion();
  useEffect(() => {
    const el = target.current;
    if (!el) return;
    const onScroll = () => setShown(el.scrollTop > after);
    onScroll();
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  }, [target, after]);
  return (
    <AnimatePresence>
      {shown && (
        <motion.button
          className="scroll-top"
          onClick={() => target.current?.scrollTo({ top: 0, behavior: still ? "auto" : "smooth" })}
          aria-label={t("Revenir en haut", "Back to top")}
          title={t("Revenir en haut", "Back to top")}
          initial={{ opacity: 0, y: 14, scale: 0.86 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: 10, scale: 0.9, transition: { duration: 0.16, ease: "easeOut" } }}
          whileHover={{ y: -2 }}
          whileTap={{ scale: 0.92 }}
          transition={{ type: "spring", stiffness: 380, damping: 26 }}
        >
          <Icon name="up" size={18} stroke={2.2} />
        </motion.button>
      )}
    </AnimatePresence>
  );
}
