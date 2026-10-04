import type { PageRange } from "./tokenize";
import type { Token } from "./types";

/**
 * Pages qui tiennent dans l'écran, sans défilement (comme sur LingQ).
 *
 * Le texte est composé dans un bloc invisible de même largeur, même police et
 * mêmes paragraphes que la vraie page ; on y cherche le dernier mot dont la
 * ligne tient dans la hauteur. La page s'arrête de préférence à la fin d'une
 * phrase (on ne la coupe pas en deux), sinon à la fin d'une ligne. Chaque page
 * est recomposée à partir de son premier mot : une phrase qui commence en milieu
 * de ligne repart en début de ligne sur la page suivante, et les lignes changent.
 */

// fin de phrase, comme tokenize.ts et text.rs (le retour à la ligne aussi)
const SENTENCE_END = /[.!?…;。！？।؟]|\n/;
const CLOSERS = /^[»”’)\]」』）]+$/;
/** Une page s'arrête à la fin d'une phrase si elle est au moins remplie à ce point. */
const SENTENCE_FILL = 0.6;

export interface FitOptions {
  /** hauteur disponible pour le texte, en pixels */
  height: number;
  /** hauteur disponible sur la première page (sous le titre de la leçon) */
  first: number;
  /** hauteur d'une ligne, en pixels */
  line: number;
}

const ESC: Record<string, string> = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" };
const esc = (s: string) => s.replace(/[&<>"]/g, (c) => ESC[c]);

/** Même découpage en paragraphes que le rendu du lecteur (Reader.tsx). */
function compose(tokens: Token[], a: number, b: number): string {
  let html = "";
  let p = "";
  for (let i = a; i < b; i++) {
    const tk = tokens[i];
    if (tk.w) {
      p += `<span class="w" data-k="${i}">${esc(tk.t)}</span>`;
      continue;
    }
    const parts = tk.t.split(/\n+/);
    if (parts.length > 1) {
      if (parts[0]) p += esc(parts[0]);
      if (p) html += `<p>${p}</p>`;
      p = "";
      const tail = parts[parts.length - 1];
      if (tail.trim()) p += esc(tail);
    } else if (tk.t.trim()) {
      // ponctuation : mesurée elle aussi (un tiret ou un guillemet peut ouvrir la ligne)
      p += `<span data-k="${i}">${esc(tk.t)}</span>`;
    } else p += esc(tk.t);
  }
  if (p) html += `<p>${p}</p>`;
  return html;
}

/**
 * Découpe la leçon en pages qui tiennent dans `opts.height`.
 * `box` : bloc invisible qui porte les styles de la page (classe `page`, langue, sens d'écriture).
 */
export function fitPages(box: HTMLElement, tokens: Token[], opts: FitOptions): PageRange[] {
  const n = tokens.length;
  const pages: PageRange[] = [];
  let start = 0;
  let chunk = 600;
  while (start < n) {
    const height = pages.length ? opts.height : opts.first;
    let end = -1;
    // le morceau composé doit dépasser la page : sinon on l'agrandit
    for (;;) {
      const stop = Math.min(n, start + chunk);
      end = fitOne(box, tokens, start, stop, height, opts.line);
      if (end >= 0) break;
      chunk *= 2;
    }
    if (end <= start) end = start + 1;
    let words = 0;
    for (let i = start; i < end; i++) if (tokens[i].w) words++;
    pages.push({ start, end, words });
    // le morceau suivant : une page et demie comme celle-ci (plus court, il est agrandi)
    chunk = Math.max(400, Math.ceil((end - start) * 1.5));
    start = end;
  }
  box.innerHTML = "";
  if (!pages.length) pages.push({ start: 0, end: n, words: 0 });
  // une page sans aucun mot (blancs de fin) rejoint la précédente
  if (pages.length > 1 && pages[pages.length - 1].words === 0) {
    const last = pages.pop()!;
    pages[pages.length - 1].end = last.end;
  }
  return pages;
}

/**
 * Fin (exclusive) de la page qui commence à `start`, composée sur [start, stop).
 * -1 : tout le morceau tient mais la leçon continue (morceau trop court).
 */
function fitOne(box: HTMLElement, tokens: Token[], start: number, stop: number, height: number, line: number): number {
  box.innerHTML = compose(tokens, start, stop);
  const spans = box.querySelectorAll<HTMLElement>("[data-k]");
  if (!spans.length) return stop;
  const top = box.getBoundingClientRect().top;
  // bas de la ligne d'un mot : le milieu de son texte est celui de la ligne
  const bottom = (el: HTMLElement) => {
    const r = el.getBoundingClientRect();
    return (r.top + r.bottom) / 2 - top + line / 2;
  };
  const fits = (k: number) => bottom(spans[k]) <= height + 0.5;

  // premier élément qui déborde (les positions ne font que descendre)
  let lo = 0;
  let hi = spans.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (fits(mid)) lo = mid + 1;
    else hi = mid;
  }
  if (lo === spans.length) return stop === tokens.length ? stop : -1;
  // rien ne tient (écran minuscule) : au moins une ligne, pour avancer
  if (lo === 0) {
    const first = bottom(spans[0]);
    let k = 1;
    while (k < spans.length && bottom(spans[k]) <= first + 1) k++;
    return k < spans.length ? Number(spans[k].dataset.k) : stop;
  }
  const lineEnd = Number(spans[lo].dataset.k);

  // fin de phrase la plus tardive avant le débordement
  for (let j = lineEnd - 1; j > start; j--) {
    const tk = tokens[j];
    if (tk.w || !SENTENCE_END.test(tk.t)) continue;
    // la ponctuation finale garde ses guillemets fermants et les blancs qui suivent
    // (un guillemet droit n'est fermant que collé à elle : après un blanc, il ouvre la phrase suivante)
    let e = j + 1;
    while (e < lineEnd && !tokens[e].w) {
      const s = tokens[e].t.trim();
      if (s && !CLOSERS.test(s) && !(s === '"' && e === j + 1)) break;
      e++;
    }
    // hauteur atteinte par cette phrase : celle de son dernier élément mesuré
    let k = lo - 1;
    while (k > 0 && Number(spans[k].dataset.k) >= e) k--;
    if (bottom(spans[k]) >= height * SENTENCE_FILL) return e;
    break;
  }
  return lineEnd;
}

/** Page qui contient le jeton `i` (0 si aucune). */
export function pageOfToken(pages: PageRange[], i: number): number {
  const p = pages.findIndex((r) => i >= r.start && i < r.end);
  return p < 0 ? (i >= (pages[pages.length - 1]?.end ?? 0) ? Math.max(0, pages.length - 1) : 0) : p;
}
