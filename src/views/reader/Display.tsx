import { AnimatePresence, motion } from "motion/react";
import { useEffect, useRef } from "react";
import { Icon } from "../../components/Icon";
import { Segmented } from "../../components/ui";
import { t } from "../../lib/i18n";
import { glyphSample, LOOK_DEFAULTS, PAPERS, panelMode, panelOptions, readFont, readFonts, SIZE_MAX, SIZE_MIN, type ReaderLayout } from "../../lib/reading";
import { useApp } from "../../lib/store";

const RING = { type: "spring", stiffness: 520, damping: 40 } as const;

/** Mise en page : pages sans défilement (comme LingQ) ou une longue page à faire défiler. */
export function LayoutPicker({ value, onChange, id }: { value: ReaderLayout; onChange(v: ReaderLayout): void; id: string }) {
  const opts: { v: ReaderLayout; label: string; hint: string }[] = [
    { v: "pages", label: t("Pages", "Pages"), hint: t("Sans défilement, des flèches pour tourner", "No scrolling, arrows to turn") },
    { v: "scroll", label: t("Défilement", "Scrolling"), hint: t("Une longue page à faire défiler", "One long page to scroll") },
  ];
  return (
    <div className="look-layouts" role="radiogroup" aria-label={t("Mise en page", "Layout")}>
      {opts.map((o) => (
        <button key={o.v} role="radio" aria-checked={value === o.v} className={`look-layout ${value === o.v ? "on" : ""}`} onClick={() => onChange(o.v)}>
          {value === o.v && <motion.span layoutId={`${id}-layout`} className="look-ring" transition={RING} />}
          <LayoutArt kind={o.v} />
          <span className="ll-text">
            <strong>{o.label}</strong>
            <span>{o.hint}</span>
          </span>
        </button>
      ))}
    </div>
  );
}

function LayoutArt({ kind }: { kind: ReaderLayout }) {
  return (
    <svg className="ll-art" viewBox="0 0 76 48" aria-hidden="true">
      {kind === "pages" ? (
        <>
          <rect className="ll-sheet" x="17" y="4" width="42" height="40" rx="4" />
          {[12, 18, 24, 30].map((y, i) => (
            <path key={y} className="ll-line" d={`M23 ${y}h${i === 3 ? 18 : 30}`} />
          ))}
          <path className="ll-arrow" d="M9 19l-4 5 4 5" />
          <path className="ll-arrow lit" d="M67 19l4 5-4 5" />
        </>
      ) : (
        <>
          <rect className="ll-sheet" x="17" y="4" width="42" height="40" rx="4" />
          {[11, 17, 23, 29, 35, 41].map((y) => (
            <path key={y} className="ll-line" d={`M23 ${y}h${y === 41 ? 22 : 28}`} />
          ))}
          <path className="ll-track" d="M64 8v32" />
          <path className="ll-thumb" d="M64 9v11" />
        </>
      )}
    </svg>
  );
}

/** Polices de lecture, montrées dans l'écriture de la langue étudiée. */
export function FontPicker({ value, onChange, lang, id }: { value: string; onChange(v: string): void; lang: string; id: string }) {
  const sample = glyphSample(lang);
  return (
    <div className="look-fonts" role="radiogroup" aria-label={t("Police", "Font")}>
      {readFonts().map((f) => (
        <button key={f.id} role="radio" aria-checked={value === f.id} className={`look-font ${value === f.id ? "on" : ""}`} onClick={() => onChange(f.id)} title={f.label}>
          {value === f.id && <motion.span layoutId={`${id}-font`} className="look-ring" transition={RING} />}
          <span className="lf-sample" style={{ fontFamily: f.stack }} lang={lang}>
            {sample}
          </span>
          <span className="lf-name">{f.label}</span>
        </button>
      ))}
    </div>
  );
}

/** Couleur de la page : fond et texte vont ensemble, pour garder un bon contraste. */
export function PaperPicker({ value, onChange, id, font }: { value: string; onChange(v: string): void; id: string; font?: string }) {
  return (
    <div className="look-papers" role="radiogroup" aria-label={t("Couleur de la page", "Page color")}>
      {PAPERS.map((p) => (
        <button
          key={p.id}
          role="radio"
          aria-checked={value === p.id}
          className={`look-paper ${value === p.id ? "on" : ""}`}
          onClick={() => onChange(p.id)}
          title={p.id === "auto" ? t("Suit le thème de Lumen", "Follows Lumen's theme") : p.label}
        >
          <span className="lp-dot">
            {value === p.id && <motion.span layoutId={`${id}-paper`} className="lp-ring" transition={RING} />}
            <span className={`lp-swatch ${p.id === "auto" ? "auto" : ""}`} style={p.swatch[0] ? { background: p.swatch[0], color: p.swatch[1], fontFamily: font } : { fontFamily: font }}>
              {p.id === "auto" ? (
                <>
                  <i className="lp-half day">A</i>
                  <i className="lp-half night">a</i>
                </>
              ) : (
                "Aa"
              )}
            </span>
          </span>
          <span className="lp-name">{p.label}</span>
        </button>
      ))}
    </div>
  );
}

/** Taille du texte : petit A, curseur, grand A. */
export function SizeControl({ value, onChange }: { value: number; onChange(v: number): void }) {
  const step = (d: number) => onChange(Math.min(SIZE_MAX, Math.max(SIZE_MIN, value + d)));
  return (
    <div className="look-size">
      <button className="icon-btn ls-a small" onClick={() => step(-1)} disabled={value <= SIZE_MIN} aria-label={t("Texte plus petit", "Smaller text")}>
        A
      </button>
      <input
        className="range"
        type="range"
        min={SIZE_MIN}
        max={SIZE_MAX}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        aria-label={t("Taille du texte", "Text size")}
      />
      <button className="icon-btn ls-a big" onClick={() => step(1)} disabled={value >= SIZE_MAX} aria-label={t("Texte plus grand", "Larger text")}>
        A
      </button>
      <span className="ls-value num">{value}</span>
    </div>
  );
}

export function lineHeightOptions() {
  return [
    { value: "1.55", label: t("Serré", "Tight") },
    { value: "1.75", label: "Normal" },
    { value: "1.95", label: t("Aéré", "Airy") },
  ];
}

export function widthOptions() {
  return [
    { value: "narrow", label: t("Étroite", "Narrow") },
    { value: "normal", label: t("Normale", "Normal") },
    { value: "wide", label: t("Large", "Wide") },
  ];
}

/**
 * Menu « Aa » de la leçon : tout l'affichage, appliqué à la page en direct.
 * Les choix valent pour toutes les leçons (mêmes réglages que Réglages › Lecture).
 */
export function DisplayMenu({ open, onClose, lang }: { open: boolean; onClose(): void; lang: string }) {
  const settings = useApp((s) => s.settings);
  const setSetting = useApp((s) => s.setSetting);
  const ref = useRef<HTMLDivElement>(null);
  const set = (k: string) => (v: string) => void setSetting(k, v);
  const changed = Object.entries(LOOK_DEFAULTS).some(([k, v]) => (settings[k] ?? v) !== v);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const el = e.target as HTMLElement;
      // le bouton « Aa » referme lui-même le menu
      if (ref.current?.contains(el) || el.closest(".look-btn")) return;
      onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      onClose();
    };
    window.addEventListener("mousedown", onDown);
    // phase de capture : Échap ferme le menu sans toucher la sélection de la page
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [open, onClose]);

  const reset = () => {
    for (const [k, v] of Object.entries(LOOK_DEFAULTS)) if ((settings[k] ?? v) !== v) void setSetting(k, v);
  };

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          ref={ref}
          className="look-pop no-drag"
          role="dialog"
          aria-label={t("Affichage de la leçon", "Lesson display")}
          initial={{ opacity: 0, y: -8, scale: 0.96 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: -6, scale: 0.97 }}
          transition={{ type: "spring", stiffness: 520, damping: 36, mass: 0.7 }}
        >
          <div className="look-head">
            <h3>{t("Affichage", "Display")}</h3>
            <AnimatePresence>
              {changed && (
                <motion.button className="look-reset" onClick={reset} initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}>
                  <Icon name="refresh" size={13} /> {t("Rétablir", "Reset")}
                </motion.button>
              )}
            </AnimatePresence>
          </div>

          <section>
            <span className="eyebrow">{t("Mise en page", "Layout")}</span>
            <LayoutPicker id="pop" value={settings.reader_layout === "scroll" ? "scroll" : "pages"} onChange={set("reader_layout")} />
          </section>

          <section>
            <span className="eyebrow">{t("Police", "Font")}</span>
            <FontPicker id="pop" lang={lang} value={settings.read_font || "literata"} onChange={set("read_font")} />
          </section>

          <section>
            <span className="eyebrow">{t("Taille", "Size")}</span>
            <SizeControl value={Number(settings.font_size) || 23} onChange={(v) => void setSetting("font_size", String(v))} />
          </section>

          <section>
            <span className="eyebrow">{t("Couleur de la page", "Page color")}</span>
            <PaperPicker id="pop" value={settings.read_paper || "auto"} onChange={set("read_paper")} font={readFont(settings.read_font).stack} />
          </section>

          <section className="look-rows">
            <div className="look-row">
              <span>{t("Interligne", "Line spacing")}</span>
              <Segmented id="pop-lh" value={settings.line_height} onChange={set("line_height")} options={lineHeightOptions()} />
            </div>
            <div className="look-row">
              <span>{t("Largeur", "Width")}</span>
              <Segmented id="pop-w" value={settings.read_width || "normal"} onChange={set("read_width")} options={widthOptions()} />
            </div>
            <div className="look-row">
              <span>{t("Mots", "Words")}</span>
              <Segmented
                id="pop-ws"
                value={settings.word_style}
                onChange={set("word_style")}
                options={[
                  { value: "tint", label: t("Teinte", "Tint") },
                  { value: "line", label: t("Soulignés", "Underlined") },
                ]}
              />
            </div>
            <div className="look-row">
              <span title={t("À droite du texte, ou flottant au-dessus du mot touché. « Auto » le fait flotter quand la fenêtre est étroite.", "On the right of the text, or floating above the word you tap. “Auto” floats it when the window is narrow.")}>
                {t("Panneau", "Panel")}
              </span>
              <Segmented id="pop-wp" value={panelMode(settings.word_panel)} onChange={set("word_panel")} options={panelOptions()} />
            </div>
          </section>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
