import type { CSSProperties } from "react";
import { t } from "./i18n";
import { langInfo } from "./langs";
import { isWindows } from "./platform";

/**
 * Affichage des leçons : mise en page, police, taille, couleur de la page,
 * interligne, largeur. Réglé dans la leçon (bouton « Aa ») et dans Réglages › Lecture.
 */

export type ReaderLayout = "pages" | "scroll";

export interface ReadFont {
  id: string;
  label: string;
  /** pile de polices : celles du Mac d'abord, des secours pour Windows et le navigateur */
  stack: string;
  kind: "serif" | "sans";
  /** police du Mac seulement : pas proposée sous Windows (elle y prendrait l'allure d'une autre) */
  mac?: boolean;
}

/** Polices de lecture : celles livrées avec Lumen et celles de macOS. */
export const READ_FONTS: ReadFont[] = [
  { id: "literata", label: "Literata", stack: '"Literata Variable", Georgia, serif', kind: "serif" },
  { id: "newsreader", label: "Newsreader", stack: '"Newsreader Variable", "Literata Variable", Georgia, serif', kind: "serif" },
  { id: "newyork", label: "New York", stack: 'ui-serif, "New York", Georgia, serif', kind: "serif", mac: true },
  { id: "georgia", label: "Georgia", stack: 'Georgia, "Times New Roman", serif', kind: "serif" },
  { id: "palatino", label: "Palatino", stack: 'Palatino, "Palatino Linotype", "Book Antiqua", serif', kind: "serif" },
  { id: "geist", label: "Geist", stack: '"Geist Variable", -apple-system, system-ui, sans-serif', kind: "sans" },
  {
    id: "system",
    get label() {
      return t("Système", "System");
    },
    stack: '-apple-system, BlinkMacSystemFont, system-ui, "Segoe UI", sans-serif',
    kind: "sans",
  },
  {
    id: "rounded",
    get label() {
      return t("Arrondie", "Rounded");
    },
    stack: 'ui-rounded, "SF Pro Rounded", -apple-system, system-ui, sans-serif',
    kind: "sans",
    mac: true,
  },
];

/** Polices proposées sur ce système (une police du Mac choisie avant reste lisible : `readFont`). */
export const readFonts = (): ReadFont[] => (isWindows ? READ_FONTS.filter((f) => !f.mac) : READ_FONTS);

export interface Paper {
  id: string;
  label: string;
  /** palette de base : claire ou sombre (couleurs des mots, lanterne) */
  base: "" | "light" | "dark";
  /** pastille du menu : fond et texte */
  swatch: [string, string];
}

/** Couleurs de page. « Auto » suit le thème de Lumen (clair le jour, sombre le soir). */
export const PAPERS: Paper[] = [
  { id: "auto", label: "Auto", base: "", swatch: ["", ""] },
  {
    id: "paper",
    get label() {
      return t("Papier", "Paper");
    },
    base: "light",
    swatch: ["#fcfaf5", "#1f1b16"],
  },
  {
    id: "sepia",
    get label() {
      return t("Sépia", "Sepia");
    },
    base: "light",
    swatch: ["#f3e7cf", "#433423"],
  },
  {
    id: "dusk",
    get label() {
      return t("Crépuscule", "Dusk");
    },
    base: "dark",
    swatch: ["#2b2724", "#ebe3d7"],
  },
  {
    id: "night",
    get label() {
      return t("Nuit", "Night");
    },
    base: "dark",
    swatch: ["#17130f", "#eadfce"],
  },
  {
    id: "ink",
    get label() {
      return t("Encre", "Ink");
    },
    base: "dark",
    swatch: ["#050505", "#d6d0c6"],
  },
];

/** Largeur de la colonne de texte (sans les marges). */
export const WIDTHS: Record<string, number> = { narrow: 540, normal: 624, wide: 820 };

/** Panneau du mot : à droite du texte, flottant au-dessus du mot touché, ou selon la place. */
export type PanelMode = "auto" | "side" | "float";

export const panelMode = (v: string | undefined): PanelMode => (v === "side" || v === "float" ? v : "auto");

export function panelOptions(): { value: PanelMode; label: string }[] {
  return [
    { value: "auto", label: "Auto" },
    { value: "side", label: t("À droite", "Docked") },
    { value: "float", label: t("Flottant", "Floating") },
  ];
}

/**
 * Largeur du lecteur sous laquelle le panneau « Auto » flotte : la colonne, les
 * flèches et de l'air de part et d'autre, plus le panneau (360 px). Sur un écran
 * de 13 pouces avec la barre latérale, le texte prend alors toute la place.
 */
export const roomForSidePanel = (colW: number) => colW + 620;

export const SIZE_MIN = 16;
export const SIZE_MAX = 40;

/** Réglages de l'affichage et leurs valeurs d'origine (« Rétablir », DEFAULTS du store). */
export const LOOK_DEFAULTS: Record<string, string> = {
  // "pages" : pages qui tiennent dans l'écran, sans défilement ; "scroll" : une longue page à faire défiler
  reader_layout: "pages",
  read_font: "literata",
  font_size: "23",
  line_height: "1.75",
  read_width: "normal",
  // couleur de la page : "auto" suit le thème de Lumen
  read_paper: "auto",
  word_style: "tint",
  // panneau du mot : "auto" flotte au-dessus du mot quand la fenêtre est étroite
  word_panel: "auto",
};

/** Vitesses de lecture : audio et vidéo, ou voix (réglages `media_rate` et `tts_rate`). */
export function playbackRates(media: boolean): string[] {
  return media ? ["0.75", "0.9", "1", "1.25"] : ["0.75", "0.85", "0.95", "1.1"];
}

export function readFont(id: string | undefined): ReadFont {
  return READ_FONTS.find((f) => f.id === id) ?? READ_FONTS[0];
}

export function paper(id: string | undefined): Paper {
  return PAPERS.find((p) => p.id === id) ?? PAPERS[0];
}

export interface Look {
  layout: ReaderLayout;
  font: ReadFont;
  size: number;
  lineHeight: number;
  width: number;
  paper: Paper;
  /** classes de la racine du lecteur (couleur de la page) */
  className: string;
  /** variables CSS partagées par la page, sa mesure et le menu */
  style: CSSProperties;
}

export function readerLook(s: Record<string, string>): Look {
  const font = readFont(s.read_font);
  const size = Math.min(SIZE_MAX, Math.max(SIZE_MIN, Number(s.font_size) || 23));
  const lineHeight = Number(s.line_height) || 1.75;
  const width = WIDTHS[s.read_width] ?? WIDTHS.normal;
  const p = paper(s.read_paper);
  return {
    layout: s.reader_layout === "scroll" ? "scroll" : "pages",
    font,
    size,
    lineHeight,
    width,
    paper: p,
    className: p.base ? `paper-${p.base} paper-${p.id}` : "",
    style: {
      ["--read-size" as string]: `${size}px`,
      ["--read-lh" as string]: lineHeight,
      ["--read-font" as string]: font.stack,
      ["--col-w" as string]: `${width}px`,
    },
  };
}

/** Deux lettres dans l'écriture de la langue, pour montrer une police (« Aa », « Пр », « こん »). */
export function glyphSample(lang: string): string {
  const hello = langInfo(lang).hello;
  if (/^\p{Script=Latin}/u.test(hello)) return "Aa";
  const seg = new Intl.Segmenter(lang, { granularity: "grapheme" });
  return Array.from(seg.segment(hello), (g) => g.segment)
    .slice(0, 2)
    .join("");
}
