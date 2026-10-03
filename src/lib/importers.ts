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
}

/** Livre EPUB : un chapitre par document de la « spine ». */
export async function extractEpub(data: ArrayBuffer): Promise<{ book: string; chapters: Chapter[] }> {
  const zip = await JSZip.loadAsync(data);
  const container = await zip.file("META-INF/container.xml")?.async("string");
  if (!container) throw new Error(t("Ce fichier EPUB est invalide.", "This EPUB file is invalid."));
  const opfPath = /full-path="([^"]+)"/.exec(container)?.[1];
  if (!opfPath) throw new Error(t("Ce fichier EPUB est invalide.", "This EPUB file is invalid."));
  const opf = await zip.file(opfPath)!.async("string");
  const opfDoc = new DOMParser().parseFromString(opf, "application/xml");
  const dir = opfPath.includes("/") ? opfPath.slice(0, opfPath.lastIndexOf("/") + 1) : "";
  const book = opfDoc.getElementsByTagName("dc:title")[0]?.textContent?.trim() || t("Livre", "Book");
  const manifest = new Map<string, string>();
  for (const item of Array.from(opfDoc.getElementsByTagName("item"))) {
    manifest.set(item.getAttribute("id") ?? "", item.getAttribute("href") ?? "");
  }
  const chapters: Chapter[] = [];
  for (const ref of Array.from(opfDoc.getElementsByTagName("itemref"))) {
    const href = manifest.get(ref.getAttribute("idref") ?? "");
    if (!href) continue;
    const path = decodeURIComponent(dir + href).replace(/[^/]+\/\.\.\//g, "");
    const file = zip.file(path);
    if (!file) continue;
    const html = await file.async("string");
    const doc = new DOMParser().parseFromString(html, "application/xhtml+xml");
    const usable = doc.getElementsByTagName("parsererror").length ? new DOMParser().parseFromString(html, "text/html") : doc;
    const heading = usable.querySelector("h1, h2, h3, title")?.textContent?.replace(/\s+/g, " ").trim();
    const text = cleanText(htmlToParagraphs(usable));
    const words = text.split(/\s+/).filter(Boolean).length;
    if (words < 40) continue; // pages de garde, tables, mentions légales
    chapters.push({ title: heading || t(`Chapitre ${chapters.length + 1}`, `Chapter ${chapters.length + 1}`), text, words });
  }
  if (!chapters.length) throw new Error(t("Aucun chapitre lisible dans ce livre.", "No readable chapter in this book."));
  return { book, chapters };
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
