import type { Token } from "./types";

/**
 * Langues où les accents aigu et grave ne distinguent pas les mots (accent
 * tonique des textes russes…) : retirés de la clé. Ailleurs (vietnamien, grec,
 * tchèque, français…), ils font partie du mot. Miroir de `strips_accents` (text.rs).
 */
const STRIPS_ACCENTS = new Set(["", "en", "it", "de", "pt", "ru", "es", "uk", "bg"]);

/** Même normalisation que le backend (text.rs, `normalize_for`). */
export function normalize(s: string, lang = ""): string {
  let t = s.trim().toLowerCase().replace(/’/g, "'");
  t = STRIPS_ACCENTS.has(lang) ? t.normalize("NFD").replace(/[\u0301\u0300]/g, "").normalize("NFC") : t.normalize("NFC");
  return t.split(/\s+/).filter(Boolean).join(" ");
}

const LETTER = /\p{L}/u;
const ELISION_LANGS = new Set(["it", "fr", "pt", "ca"]);

/** Découpage en mots (utilisé par l'aperçu navigateur ; l'app utilise Rust). */
export function tokenize(text: string, lang: string): Token[] {
  const seg = new Intl.Segmenter(lang, { granularity: "word" });
  const out: Token[] = [];
  const push = (t: string, s: number) => {
    const w = LETTER.test(t);
    out.push({ t, w, k: w ? normalize(t, lang) : "", s, e: s + t.length });
  };
  for (const part of seg.segment(text)) {
    const t = part.segment;
    if (ELISION_LANGS.has(lang) && LETTER.test(t)) {
      const m = t.match(/^(\p{L}{1,6}['’])(\p{L}.*)$/u);
      if (m) {
        push(m[1], part.index);
        push(m[2], part.index + m[1].length);
        continue;
      }
    }
    push(t, part.index);
  }
  return out;
}

export interface PageRange {
  start: number; // index du premier jeton
  end: number; // index exclusif
  words: number;
}

/** Découpe une leçon en pages d'environ `target` mots, aux limites de paragraphes. */
export function paginate(tokens: Token[], target = 230): PageRange[] {
  // 1. blocs : paragraphes, eux-mêmes coupés en phrases s'ils sont très longs
  const blocks: PageRange[] = [];
  let a = 0;
  const flushPara = (b: number) => {
    if (b <= a) return;
    const words = countWords(tokens, a, b);
    if (words <= target * 1.3) {
      blocks.push({ start: a, end: b, words });
    } else {
      let s = a;
      let w = 0;
      for (let i = a; i < b; i++) {
        if (tokens[i].w) w++;
        const end = !tokens[i].w && /[.!?…]/.test(tokens[i].t);
        if ((end && w >= target * 0.5) || w >= target * 1.3) {
          blocks.push({ start: s, end: i + 1, words: w });
          s = i + 1;
          w = 0;
        }
      }
      if (s < b) blocks.push({ start: s, end: b, words: countWords(tokens, s, b) });
    }
    a = b;
  };
  for (let i = 0; i < tokens.length; i++) {
    if (!tokens[i].w && tokens[i].t.includes("\n")) flushPara(i + 1);
  }
  flushPara(tokens.length);

  // 2. regroupement des blocs en pages
  const pages: PageRange[] = [];
  for (const blk of blocks) {
    const cur = pages[pages.length - 1];
    if (cur && (cur.words + blk.words <= target * 1.15 || blk.words === 0 || cur.words < target * 0.35)) {
      cur.end = blk.end;
      cur.words += blk.words;
    } else {
      pages.push({ ...blk });
    }
  }
  if (!pages.length) pages.push({ start: 0, end: tokens.length, words: 0 });
  if (pages.length > 1 && pages[pages.length - 1].words < target * 0.25) {
    const last = pages.pop()!;
    pages[pages.length - 1].end = last.end;
    pages[pages.length - 1].words += last.words;
  }
  return pages;
}

function countWords(tokens: Token[], a: number, b: number) {
  let n = 0;
  for (let i = a; i < b; i++) if (tokens[i].w) n++;
  return n;
}

// ponctuation de fin de phrase (latine, grecque « ; », japonaise, hindi, arabe), comme text.rs
const SENTENCE_END = /[.!?…;。！？।؟]|\n/;

/** Bornes (indices de jetons) de la phrase qui contient le jeton i. */
export function sentenceBounds(tokens: Token[], i: number): [number, number] {
  let a = i;
  while (a > 0) {
    const p = tokens[a - 1];
    if (!p.w && SENTENCE_END.test(p.t)) break;
    a--;
  }
  let b = i;
  while (b < tokens.length - 1) {
    const n = tokens[b];
    if (!n.w && SENTENCE_END.test(n.t)) break;
    b++;
  }
  // inclut la ponctuation finale et un guillemet fermant éventuel
  if (b < tokens.length && !tokens[b].w) b++;
  while (b < tokens.length && !tokens[b].w && /^[»"”’)\]]+$/.test(tokens[b].t.trim()) && tokens[b].t.trim()) b++;
  while (a < b && !tokens[a].w && !tokens[a].t.trim()) a++;
  return [a, b];
}

export function sliceText(text: string, tokens: Token[], a: number, b: number): string {
  if (a >= b) return "";
  return text.slice(tokens[a].s, tokens[b - 1].e).replace(/\s+/g, " ").trim();
}
