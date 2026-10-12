// Extraction de texte depuis toutes les sources importables.
import { Readability } from "@mozilla/readability";
import { t } from "./i18n";
import JSZip from "jszip";

export interface Extracted {
  title: string;
  text: string;
}

/** Nettoie un texte : espaces, lignes vides, césures de fin de ligne. */
export function cleanText(raw: string): string {
  return raw
    .replace(/\r\n?/g, "\n")
    .replace(/­/g, "")
    .replace(/[ \t ]+/g, " ")
    .replace(/(\p{L})-\n(\p{Ll})/gu, "$1$2")
    .split(/\n{2,}/)
    .map((p) => p.replace(/\s*\n\s*/g, " ").trim())
    .filter(Boolean)
    .join("\n\n");
}

function htmlToParagraphs(root: Element | Document): string {
  const blocks: string[] = [];
  const BLOCK = /^(P|H[1-6]|LI|BLOCKQUOTE|DD|DT|FIGCAPTION|PRE|TD|p|h[1-6]|li|blockquote|dd|dt|figcaption|pre|td)$/;
  const walk = (el: Element) => {
    for (const child of Array.from(el.children)) {
      if (/^(SCRIPT|STYLE|NAV|ASIDE|FOOTER|NOSCRIPT|SVG|svg)$/.test(child.tagName)) continue;
      if (BLOCK.test(child.tagName)) {
        const t = (child.textContent ?? "").replace(/\s+/g, " ").trim();
        if (t) blocks.push(t);
      } else {
        walk(child);
      }
    }
  };
  const body = "body" in root ? (root as Document).body : root;
  if (body) walk(body as Element);
  if (!blocks.length) return cleanText((body as Element)?.textContent ?? "");
  return blocks.join("\n\n");
}

/** Article web : extraction du contenu principal (algorithme de Firefox). */
export function extractArticle(html: string, url: string): Extracted {
  const doc = new DOMParser().parseFromString(html, "text/html");
  const base = doc.createElement("base");
  base.href = url;
  doc.head.prepend(base);
  const art = new Readability(doc, { charThreshold: 200 }).parse();
  if (!art || !art.content) throw new Error(t("Aucun article lisible n'a été trouvé sur cette page.", "No readable article was found on this page."));
  const content = new DOMParser().parseFromString(art.content, "text/html");
  const text = cleanText(htmlToParagraphs(content));
  return { title: (art.title || new URL(url).hostname).trim(), text };
}

export interface Chapter extends Extracted {
  words: number;
  /** couverture, page de titre, table des matières, mentions légales… : décochés d'office */
  extra?: boolean;
}

export interface Book {
  title: string;
  chapters: Chapter[];
  /** couverture du livre, telle que l'EPUB la porte */
  cover: Blob | null;
}

// Pages qui ne se lisent pas comme un chapitre, d'après les repères du livre
// (`epub:type`, « landmarks » d'EPUB 3, « guide » d'EPUB 2).
const EXTRA_TYPES = new Set([
  "cover", "titlepage", "title-page", "halftitlepage", "copyright-page", "copyright", "toc", "landmarks", "loi", "lot",
  "colophon", "imprint", "imprimatur", "other-credits", "credits", "contributors", "index", "dedication",
  "acknowledgments", "acknowledgements", "frontispiece", "errata", "bibliography", "endnotes", "footnotes", "rearnotes",
]);

// … et d'après leur titre, dans les langues les plus courantes
const EXTRA_TITLE =
  /^(table des matières|sommaire|table of contents|contents|índice|indice|inhalt|inhaltsverzeichnis|содержание|оглавление|spis treści|inhoud|inhoudsopgave|innehåll|indholdsfortegnelse|innhold|sisällys|sisällysluettelo|tartalom|tartalomjegyzék|obsah|cuprins|içindekiler|πίνακας περιεχομένων|περιεχόμενα|目次|목차|copyright|droits d'auteur|mentions légales|colophon|dédicace|dedication|dedica|widmung|remerciements|acknowledg?ements|ringraziamenti|danksagung|agradecimientos|agradecimentos|du même auteur|also by|dello stesso autore|vom selben autor|del mismo autor|about the author|à propos de l'auteur|sobre el autor|sull'autore|über den autor|page de titre|title page|cover|couverture|copertina|portada|capa|titelseite|frontespizio|index|license|licence|licenza|lizenz|licencia|licença|the full project gutenberg|the project gutenberg)(?![\p{L}])/iu;

/** Chemin d'un fichier du livre : `href` relatif au dossier `base`, sans son ancre. */
function zipPath(base: string, href: string): { path: string; frag: string } {
  const hash = href.indexOf("#");
  const raw = hash < 0 ? href : href.slice(0, hash);
  let frag = hash < 0 ? "" : href.slice(hash + 1);
  let path = base + raw;
  try {
    path = decodeURIComponent(path);
    frag = decodeURIComponent(frag);
  } catch {
    // adresse mal encodée : gardée telle quelle
  }
  const parts: string[] = [];
  for (const seg of path.split("/")) {
    if (seg === "..") parts.pop();
    else if (seg && seg !== ".") parts.push(seg);
  }
  return { path: parts.join("/"), frag };
}

const dirOf = (p: string) => (p.includes("/") ? p.slice(0, p.lastIndexOf("/") + 1) : "");
const oneLine = (s: string | null | undefined) => (s ?? "").replace(/\s+/g, " ").trim();

function parseXhtml(src: string): Document {
  const doc = new DOMParser().parseFromString(src, "application/xhtml+xml");
  return doc.getElementsByTagName("parsererror").length ? new DOMParser().parseFromString(src, "text/html") : doc;
}

/** Repères `epub:type` d'un élément (« bodymatter chapter »…). */
const typesOf = (el: Element | null | undefined) => oneLine(el?.getAttribute("epub:type")).toLowerCase().split(" ").filter(Boolean);

interface TocEntry {
  title: string;
  path: string;
  frag: string;
  depth: number;
}

/** Table des matières d'EPUB 3 (document `nav`) : entrées, et pages repérées comme annexes. */
function navToc(src: string, base: string, extra: Set<string>): TocEntry[] {
  const doc = parseXhtml(src);
  const navs = Array.from(doc.getElementsByTagName("nav"));
  for (const nav of navs.filter((n) => typesOf(n).includes("landmarks"))) {
    for (const a of Array.from(nav.getElementsByTagName("a"))) {
      if (typesOf(a).some((x) => EXTRA_TYPES.has(x))) extra.add(zipPath(base, a.getAttribute("href") ?? "").path);
    }
  }
  const toc = navs.find((n) => typesOf(n).includes("toc")) ?? navs.find((n) => n.getAttribute("role") === "doc-toc") ?? navs[0];
  if (!toc) return [];
  return Array.from(toc.getElementsByTagName("a")).flatMap((a) => {
    const href = a.getAttribute("href");
    if (!href) return [];
    let depth = 0;
    for (let el = a.parentElement; el && el !== toc; el = el.parentElement) if (el.localName === "ol" || el.localName === "ul") depth++;
    return [{ title: oneLine(a.textContent), ...zipPath(base, href), depth: Math.max(1, depth) }];
  });
}

/** Table des matières d'EPUB 2 (fichier NCX). */
function ncxToc(src: string, base: string): TocEntry[] {
  const doc = new DOMParser().parseFromString(src, "application/xml");
  return Array.from(doc.getElementsByTagName("navPoint")).flatMap((np) => {
    const kids = Array.from(np.children);
    const src = kids.find((k) => k.localName === "content")?.getAttribute("src");
    if (!src) return [];
    const label = kids.find((k) => k.localName === "navLabel");
    let depth = 0;
    for (let el: Element | null = np; el; el = el.parentElement) if (el.localName === "navPoint") depth++;
    return [{ title: oneLine(label?.textContent), ...zipPath(base, src), depth }];
  });
}

const BLOCK_TAG = /^(p|h[1-6]|li|blockquote|dd|dt|figcaption|pre|td)$/i;
// marqueurs (caractères à usage privé) posés devant le début de chaque entrée de la table
const CUT = /(\d+)/;

/**
 * Pose dans le document le marqueur de l'entrée `i` juste avant `target` (début du
 * document s'il n'y en a pas). Dans un paragraphe, un simple texte suffit ; ailleurs,
 * il faut un paragraphe à lui, sans quoi la lecture du texte ne le verrait pas.
 */
function markCut(doc: Document, target: Element | null, i: number) {
  const body = doc.body ?? doc.getElementsByTagName("body")[0];
  if (!body) return;
  const mark = `${i}`;
  const at = target && target !== body ? target : null;
  let inBlock = false;
  for (let el = at?.parentElement; el && el !== body; el = el.parentElement) if (BLOCK_TAG.test(el.localName)) inBlock = true;
  if (at && inBlock) {
    at.parentNode?.insertBefore(doc.createTextNode(mark), at);
    return;
  }
  const p = doc.createElementNS(body.namespaceURI, "p");
  p.textContent = mark;
  if (at) at.parentNode?.insertBefore(p, at);
  else body.insertBefore(p, body.firstChild);
}

/** Le texte en paragraphes, sans lignes vides au début ni à la fin. */
const tidy = (s: string) => s.split(/\n{2,}/).map((p) => p.trim()).filter(Boolean).join("\n\n");

/**
 * Livre EPUB : un chapitre par entrée de sa table des matières (au niveau qui donne
 * de vrais chapitres : les parties d'un livre, puis leurs chapitres), quel que soit
 * le nombre de fichiers dans lesquels l'éditeur l'a découpé. Sans table des matières :
 * un chapitre par fichier, les fichiers sans titre rattachés au précédent.
 */
export async function extractEpub(data: ArrayBuffer): Promise<Book> {
  const zip = await JSZip.loadAsync(data);
  const container = await zip.file("META-INF/container.xml")?.async("string");
  if (!container) throw new Error(t("Ce fichier EPUB est invalide.", "This EPUB file is invalid."));
  const opfPath = /full-path="([^"]+)"/.exec(container)?.[1];
  const opfFile = opfPath ? zip.file(opfPath) : null;
  if (!opfPath || !opfFile) throw new Error(t("Ce fichier EPUB est invalide.", "This EPUB file is invalid."));
  const opfDoc = new DOMParser().parseFromString(await opfFile.async("string"), "application/xml");
  const dir = dirOf(opfPath);
  const book = oneLine(opfDoc.getElementsByTagName("dc:title")[0]?.textContent) || t("Livre", "Book");

  const items = Array.from(opfDoc.getElementsByTagName("item")).map((el) => ({
    id: el.getAttribute("id") ?? "",
    path: zipPath(dir, el.getAttribute("href") ?? "").path,
    type: el.getAttribute("media-type") ?? "",
    props: el.getAttribute("properties") ?? "",
  }));
  const byId = new Map(items.map((i) => [i.id, i]));
  const spineEl = opfDoc.getElementsByTagName("spine")[0];
  // les pages hors du fil de lecture (`linear="no"` : notes, encarts) ne font pas de leçon
  const spine = Array.from(opfDoc.getElementsByTagName("itemref"))
    .filter((r) => r.getAttribute("linear") !== "no")
    .map((r) => byId.get(r.getAttribute("idref") ?? "")?.path)
    .filter((p): p is string => !!p && !!zip.file(p));
  const spineAt = new Map<string, number>();
  spine.forEach((p, i) => !spineAt.has(p) && spineAt.set(p, i));

  // pages annexes repérées par le livre lui-même
  const extra = new Set<string>();
  for (const ref of Array.from(opfDoc.getElementsByTagName("reference"))) {
    if (EXTRA_TYPES.has((ref.getAttribute("type") ?? "").toLowerCase())) extra.add(zipPath(dir, ref.getAttribute("href") ?? "").path);
  }

  // table des matières : EPUB 3, sinon EPUB 2
  let toc: TocEntry[] = [];
  const nav = items.find((i) => /(^|\s)nav(\s|$)/.test(i.props));
  if (nav && zip.file(nav.path)) {
    extra.add(nav.path);
    toc = navToc(await zip.file(nav.path)!.async("string"), dirOf(nav.path), extra);
  }
  if (!toc.length) {
    const ncx = byId.get(spineEl?.getAttribute("toc") ?? "") ?? items.find((i) => i.type === "application/x-dtbncx+xml");
    if (ncx && zip.file(ncx.path)) toc = ncxToc(await zip.file(ncx.path)!.async("string"), dirOf(ncx.path));
  }
  // une entrée par endroit du livre, dans l'ordre de lecture
  const seen = new Set<string>();
  const entries = toc.filter((e) => {
    const k = `${e.path}#${e.frag}`;
    if (!spineAt.has(e.path) || seen.has(k)) return false;
    seen.add(k);
    return true;
  });

  const cover = await coverOf(zip, opfDoc, items);
  const docs: { path: string; doc: Document }[] = [];
  for (const path of spine) docs.push({ path, doc: parseXhtml(await zip.file(path)!.async("string")) });
  const typed = (doc: Document) => {
    const body = doc.body ?? doc.getElementsByTagName("body")[0];
    const first = body?.firstElementChild;
    return [...typesOf(body), ...(first?.localName === "section" ? typesOf(first) : [])].some((x) => EXTRA_TYPES.has(x));
  };
  const isExtra = (path: string, doc: Document, title: string) => extra.has(path) || typed(doc) || EXTRA_TITLE.test(title);

  const chapters = entries.length >= 2 ? byToc(docs, entries, isExtra) : byFiles(docs, book, isExtra);
  if (!chapters.length) throw new Error(t("Aucun chapitre lisible dans ce livre.", "No readable chapter in this book."));
  // une courte page qui porte le titre du livre : sa page de titre
  const named = book.toLowerCase();
  for (const c of chapters) if (c.words < 150 && c.title.length >= 3 && named.includes(c.title.toLowerCase())) c.extra = true;
  return { title: book, chapters, cover };
}

/** Découpe selon la table des matières. */
function byToc(docs: { path: string; doc: Document }[], entries: TocEntry[], isExtra: (path: string, doc: Document, title: string) => boolean): Chapter[] {
  // texte de chaque entrée (à toutes les profondeurs), dans l'ordre de lecture
  const parts = new Map<number, string[]>();
  const front: string[] = [];
  const starts = new Map<number, Document>();
  let cur = -1;
  for (const { path, doc } of docs) {
    const mine = entries.map((e, i) => ({ e, i })).filter(({ e }) => e.path === path);
    // une page annexe que la table ne cite pas (couverture, mentions légales) n'est pas lue
    if (!mine.length && isExtra(path, doc, "")) continue;
    for (const { e, i } of mine) {
      const sel = e.frag ? `[id="${CSS.escape(e.frag)}"], a[name="${CSS.escape(e.frag)}"]` : "";
      markCut(doc, sel ? doc.querySelector(sel) : null, i);
      starts.set(i, doc);
    }
    const pieces = cleanText(htmlToParagraphs(doc)).split(CUT);
    (cur < 0 ? front : (parts.get(cur) ?? parts.set(cur, []).get(cur)!)).push(pieces[0]);
    for (let k = 1; k < pieces.length; k += 2) {
      cur = Number(pieces[k]);
      (parts.get(cur) ?? parts.set(cur, []).get(cur)!).push(pieces[k + 1] ?? "");
    }
  }

  // niveau de la table : le premier qui donne de vrais chapitres (les parties d'un
  // livre sont trop longues : on descend alors à leurs chapitres)
  const textOf = (i: number) => tidy((parts.get(i) ?? []).join("\n\n"));
  // `pre` : sous-parties qui précèdent le premier chapitre de ce niveau
  const at = (level: number) => {
    const out: (Chapter & { parent: string })[] = [];
    const pre: string[] = [];
    // titre de la dernière entrée vue à chaque profondeur : la partie qui contient un chapitre
    const above: string[] = [];
    entries.forEach((e, i) => {
      above[e.depth] = e.title;
      above.length = e.depth + 1;
      const text = textOf(i);
      if (e.depth > level || !starts.has(i)) {
        // sous-partie d'un chapitre : elle le complète
        if (out.length && text) out[out.length - 1].text = tidy(`${out[out.length - 1].text}\n\n${text}`);
        else if (text) pre.push(text);
        return;
      }
      const title = e.title.slice(0, 120) || t(`Chapitre ${out.length + 1}`, `Chapter ${out.length + 1}`);
      out.push({ title, text, words: 0, extra: isExtra(e.path, starts.get(i)!, e.title), parent: e.depth > 1 ? (above[e.depth - 1] ?? "") : "" });
    });
    for (const c of out) c.words = wordCount(c.text);
    return Object.assign(out, { pre });
  };
  const deepest = Math.min(3, Math.max(...entries.map((e) => e.depth)));
  let level = 1;
  let chapters = at(level);
  // longueur du chapitre où tombe la moitié du texte : quelques pages de garde
  // ne doivent pas cacher deux « parties » de 200 000 mots
  const middle = (cs: Chapter[]) => {
    const w = cs.filter((c) => !c.extra).map((c) => c.words).sort((a, b) => a - b);
    const half = w.reduce((n, x) => n + x, 0) / 2;
    let sum = 0;
    for (const x of w) if ((sum += x) >= half) return x;
    return 0;
  };
  while (level < deepest && (chapters.filter((c) => !c.extra && c.words >= 40).length < 3 || middle(chapters) > 8000)) {
    const finer = at(level + 1);
    if (finer.length <= chapters.length) break;
    level++;
    chapters = finer;
  }

  // « Chapitre I » dans chaque partie : le titre de la partie les distingue (l'entrée qui
  // les contient, ou dans une table à plat la page de titre qui les précède)
  const seenTitles = new Map<string, number>();
  for (const c of chapters) if (c.words >= 40) seenTitles.set(c.title, (seenTitles.get(c.title) ?? 0) + 1);
  let section = "";

  // une page minuscule (titre d'une partie) ouvre le chapitre qui la suit ; une annexe
  // minuscule (couverture, page de titre) disparaît
  const merged: Chapter[] = [];
  let carry = "";
  for (const c of chapters) {
    if (c.words < 40) {
      if (!c.extra && c.text) carry = tidy(`${carry}\n\n${c.text}`);
      if (!c.extra && c.title) section = c.title;
      continue;
    }
    const part = c.parent || section;
    if ((seenTitles.get(c.title) ?? 0) > 1 && part) c.title = `${part} · ${c.title}`.slice(0, 120);
    if (carry) {
      c.text = tidy(`${carry}\n\n${c.text}`);
      c.words = wordCount(c.text);
      carry = "";
    }
    merged.push({ title: c.title, text: c.text, words: c.words, extra: c.extra });
  }
  if (carry && merged.length) {
    const last = merged[merged.length - 1];
    last.text = tidy(`${last.text}\n\n${carry}`);
    last.words = wordCount(last.text);
  }
  // ce qui précède la première entrée : pages de garde, ou un prologue que la table oublie
  const opening = tidy([...front, ...chapters.pre].join("\n\n"));
  const words = wordCount(opening);
  if (words >= 40) {
    const head = opening.split("\n\n")[0];
    merged.unshift({ title: head.length <= 80 ? head : t("Début du livre", "Opening pages"), text: opening, words, extra: words < 500 });
  }
  return merged;
}

/** Sans table des matières : un chapitre par fichier, les fichiers courts et sans titre rattachés au précédent. */
function byFiles(docs: { path: string; doc: Document }[], book: string, isExtra: (path: string, doc: Document, title: string) => boolean): Chapter[] {
  const out: Chapter[] = [];
  // le titre de fichier ne sert que s'il est propre à ce fichier (souvent, c'est celui du livre partout)
  const nameOf = (doc: Document) => oneLine(doc.querySelector("title")?.textContent);
  const names = new Map<string, number>();
  for (const { doc } of docs) names.set(nameOf(doc), (names.get(nameOf(doc)) ?? 0) + 1);
  for (const { path, doc } of docs) {
    const heading = oneLine(doc.querySelector("h1, h2, h3")?.textContent);
    const named = nameOf(doc);
    const title = heading || (named && named !== book && names.get(named) === 1 ? named : "");
    const text = cleanText(htmlToParagraphs(doc));
    const words = wordCount(text);
    const extra = isExtra(path, doc, title);
    if (!heading && words < 300 && out.length && !extra) {
      const last = out[out.length - 1];
      last.text = tidy(`${last.text}\n\n${text}`);
      last.words = wordCount(last.text);
      continue;
    }
    if (words < 40) continue; // pages de garde, tables, mentions légales
    out.push({ title: (title || t(`Chapitre ${out.length + 1}`, `Chapter ${out.length + 1}`)).slice(0, 120), text, words, extra });
  }
  return out;
}

/** Couverture du livre : déclarée par EPUB 3 ou EPUB 2, sinon l'image qui s'appelle ainsi. */
async function coverOf(zip: JSZip, opf: Document, items: { id: string; path: string; type: string; props: string }[]): Promise<Blob | null> {
  const images = items.filter((i) => i.type.startsWith("image/"));
  const named = Array.from(opf.getElementsByTagName("meta")).find((m) => m.getAttribute("name") === "cover")?.getAttribute("content");
  const item =
    images.find((i) => /(^|\s)cover-image(\s|$)/.test(i.props)) ??
    images.find((i) => i.id === named) ??
    images.find((i) => /cover/i.test(`${i.id} ${i.path}`));
  const file = item && zip.file(item.path);
  if (!item || !file) return null;
  try {
    return new Blob([await file.async("arraybuffer")], { type: item.type });
  } catch {
    return null;
  }
}

/** PDF : texte de chaque page, paragraphes reconstitués. */
export async function extractPdf(data: ArrayBuffer): Promise<Extracted> {
  const pdfjs = await import("pdfjs-dist");
  const workerUrl = (await import("pdfjs-dist/build/pdf.worker.min.mjs?url")).default;
  pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;
  const doc = await pdfjs.getDocument({ data: new Uint8Array(data) }).promise;
  const meta = await doc.getMetadata().catch(() => null);
  const parts: string[] = [];
  for (let p = 1; p <= doc.numPages; p++) {
    const page = await doc.getPage(p);
    const content = await page.getTextContent();
    let line = "";
    let lastY: number | null = null;
    const lines: string[] = [];
    for (const item of content.items as Array<{ str: string; transform: number[]; hasEOL?: boolean }>) {
      const y = item.transform?.[5] ?? 0;
      if (lastY !== null && Math.abs(y - lastY) > 2 && line) {
        lines.push(line);
        line = "";
      }
      line += item.str;
      if (item.hasEOL) {
        lines.push(line);
        line = "";
      }
      lastY = y;
    }
    if (line) lines.push(line);
    // une ligne courte qui finit par un point clôt un paragraphe
    let para = "";
    for (const l of lines) {
      const t = l.trim();
      if (!t) {
        if (para) parts.push(para), (para = "");
        continue;
      }
      para += (para ? "\n" : "") + t;
      if (/[.!?:»"”]$/.test(t) && t.length < 60) {
        parts.push(para);
        para = "";
      }
    }
    if (para) parts.push(para);
  }
  const info = meta?.info as { Title?: string } | undefined;
  return { title: info?.Title?.trim() || t("Document PDF", "PDF document"), text: cleanText(parts.join("\n\n")) };
}

/** Sous-titres SRT ou WebVTT : texte seul, regroupé en paragraphes. */
export function extractSubtitles(raw: string): string {
  const cues = raw
    .replace(/\r/g, "")
    .replace(/^WEBVTT.*\n/, "")
    .split(/\n{2,}/)
    .map((block) =>
      block
        .split("\n")
        .filter((l) => !/^\d+$/.test(l.trim()) && !/-->/.test(l) && !/^(NOTE|STYLE)/.test(l))
        .join(" ")
        .replace(/<[^>]+>/g, "")
        .replace(/\{\\[^}]+\}/g, "")
        .trim(),
    )
    .filter(Boolean);
  const paras: string[] = [];
  let cur = "";
  for (const c of cues) {
    cur += (cur ? " " : "") + c;
    if (/[.!?…]$/.test(c) && cur.length > 380) {
      paras.push(cur);
      cur = "";
    }
  }
  if (cur) paras.push(cur);
  return paras.join("\n\n");
}

export function decodeText(data: ArrayBuffer): string {
  const utf8 = new TextDecoder("utf-8", { fatal: false }).decode(data);
  if (!utf8.includes("�")) return utf8;
  return new TextDecoder("windows-1252").decode(data);
}

export const MEDIA_EXT = ["mp3", "m4a", "aac", "wav", "flac", "ogg", "oga", "mp4", "m4v", "mov", "mkv", "aiff", "aif", "caf"];
export const TEXT_EXT = ["txt", "md", "epub", "pdf", "srt", "vtt", "html", "htm"];

export function extOf(path: string): string {
  const m = /\.([a-z0-9]+)$/i.exec(path);
  return m ? m[1].toLowerCase() : "";
}

export function baseName(path: string): string {
  const name = path.split(/[\\/]/).pop() ?? path;
  return name.replace(/\.[^.]+$/, "").replace(/[_-]+/g, " ").trim();
}

// japonais et chinois s'écrivent sans espaces : chaque caractère compte, comme dans le lecteur
const CJK = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}]/gu;

export function wordCount(text: string): number {
  const cjk = text.match(CJK)?.length ?? 0;
  if (!cjk) return text.split(/\s+/).filter(Boolean).length;
  return cjk + text.replace(CJK, " ").split(/\s+/).filter((w) => /[\p{L}\p{N}]/u.test(w)).length;
}

/**
 * Coupe un long texte en parties de `max` mots au plus, aux paragraphes, et de
 * longueurs voisines : 2 650 mots font deux parties d'environ 1 300, pas 2 600 et 50.
 */
export function splitLong(text: string, max = 2600): string[] {
  const paras = text.split("\n\n");
  const sizes = paras.map(wordCount);
  const total = sizes.reduce((a, b) => a + b, 0);
  if (total <= max) return [text];
  const count = Math.ceil(total / max);
  const parts: string[] = [];
  let cur: string[] = [];
  let n = 0;
  let done = 0;
  paras.forEach((p, i) => {
    const w = sizes[i];
    // la part visée se recalcule avec ce qui reste : les parts restent égales
    const left = count - parts.length;
    const target = (total - done) / left;
    // couper avant ce paragraphe s'il éloigne davantage de la longueur visée (ou dépasse le maximum)
    if (cur.length && left > 1 && (n + w > max || n + w - target > target - n)) {
      parts.push(cur.join("\n\n"));
      done += n;
      cur = [];
      n = 0;
    }
    cur.push(p);
    n += w;
  });
  if (cur.length) parts.push(cur.join("\n\n"));
  return parts;
}

/** Le début d'un long texte (aux paragraphes), environ `max` mots. */
export function firstWords(text: string, max: number): string {
  const out: string[] = [];
  let n = 0;
  for (const p of text.split("\n\n")) {
    const w = wordCount(p);
    if (n + w > max && out.length) break;
    out.push(p);
    n += w;
  }
  return out.length ? out.join("\n\n") : text;
}
