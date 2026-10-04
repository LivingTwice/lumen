// Aperçu : regarder une vidéo, écouter un épisode ou une chanson, lire le début
// d'un article, avant d'en faire une leçon. Un résultat de recherche, un élément
// de Découvrir ou un lien collé prennent ici la même forme (`Previewable`).
import { create } from "zustand";
import { youtubeId } from "./covers";
import { cleanTitle } from "./discover";
import type { JobSpec } from "./imports";
import type { DiscoverItem, LangCode, LinkInfo, LinkMedia, Lyrics, SearchHit } from "./types";

export interface Previewable {
  /** identifiant de la leçon en préparation : l'adresse de ce qu'on importe */
  key: string;
  kind: "video" | "audio" | "song" | "text";
  /** "youtube", "dailymotion", "podcast", "music", "wiki", ou "discover" */
  platform: string;
  title: string;
  author: string;
  image: string;
  /** ce qu'on regarde et importe : vidéo, fichier son, article */
  url: string;
  /** page d'origine, à ouvrir dans le navigateur */
  page: string;
  /** fichier son ou vidéo lu tel quel (épisode de podcast) */
  direct: boolean;
  duration: number;
  published: number;
  count: number;
  summary: string;
  lo: number;
  hi: number;
  inLang: boolean | null;
  otherLang: string;
  /** la page porte aussi le texte (épisodes DW, RFI…) */
  pageText: boolean;
  /** collection de la leçon (émission, chaîne, source) */
  collection: string;
  /** article : nombre de mots annoncé */
  words: number;
  song?: { artist: string; title: string; album: string; lyrics: Lyrics | null; sample: string; video: boolean };
  discover?: DiscoverItem;
}

export function fromHit(h: SearchHit): Previewable {
  const base: Previewable = {
    key: h.url || h.page || h.id,
    kind: h.kind === "song" ? "song" : h.kind === "text" ? "text" : h.kind === "audio" ? "audio" : "video",
    platform: h.platform,
    title: cleanTitle(h.title),
    author: h.author,
    image: h.image,
    url: h.url,
    page: h.page || h.url,
    direct: h.platform === "podcast",
    duration: h.duration,
    published: h.published,
    count: h.count,
    summary: h.summary,
    lo: h.lo,
    hi: h.hi,
    inLang: h.in_lang,
    otherLang: h.other_lang,
    pageText: false,
    collection: h.platform === "podcast" ? h.author : "",
    words: h.words,
  };
  if (h.kind === "song") {
    base.key = `song:${h.author}|${h.title}`.toLowerCase();
    base.song = { artist: h.author, title: h.title, album: h.album, lyrics: h.lyrics, sample: h.sample, video: false };
    base.collection = h.author;
  }
  return base;
}

export function fromItem(it: DiscoverItem): Previewable {
  const song = it.shelf === "music" && !!it.artist && !!it.track;
  return {
    key: it.url,
    kind: song ? "song" : it.kind,
    platform: "discover",
    title: song ? it.track : cleanTitle(it.title),
    author: song ? it.artist : it.source_name,
    image: it.image,
    url: it.url,
    page: it.page || it.url,
    direct: it.kind !== "text" && !youtubeId(it.url),
    duration: it.duration,
    published: it.published,
    count: 0,
    summary: it.summary,
    lo: song ? 0 : it.lo,
    hi: song ? 0 : it.hi,
    inLang: true,
    otherLang: "",
    pageText: it.page_text,
    collection: song ? it.artist : it.source_name,
    words: 0,
    song: song ? { artist: it.artist, title: it.track, album: "", lyrics: null, sample: "", video: true } : undefined,
    discover: it,
  };
}

/** Un son ou une vidéo trouvé derrière un lien collé. */
export function fromLink(info: LinkInfo, m: LinkMedia): Previewable {
  return {
    key: m.url,
    kind: m.video ? "video" : "audio",
    platform: "link",
    title: m.title || info.title,
    author: m.collection || info.site,
    image: m.image || info.image,
    url: m.url,
    page: m.page || info.url,
    direct: m.direct,
    duration: m.duration,
    published: m.date ? Math.round(new Date(`${m.date}T12:00:00`).getTime() / 1000) : 0,
    count: 0,
    summary: "",
    lo: 0,
    hi: 0,
    inLang: null,
    otherLang: "",
    pageText: false,
    collection: m.collection,
    words: 0,
  };
}

/** Un article trouvé derrière un lien collé (le texte est déjà extrait). */
export function fromArticle(info: LinkInfo, title: string): Previewable {
  return { ...fromLink(info, { url: info.url, title, duration: 0, video: false, direct: false, image: info.image, date: "", page: info.url, collection: "" }), kind: "text", author: info.site };
}

/** Le son ou la vidéo, prêt pour `importLink`. */
export function mediaOf(p: Previewable): LinkMedia {
  return {
    url: p.url,
    title: p.title,
    duration: p.duration,
    video: p.kind === "video",
    direct: p.direct,
    image: p.image,
    date: p.published ? new Date(p.published * 1000).toISOString().slice(0, 10) : "",
    page: p.page || p.url,
    collection: p.collection,
  };
}

/** Ce que la file des imports doit faire de cet élément. `article` : le texte déjà lu. */
export function jobOf(p: Previewable, lang: LangCode, extra: { article?: { title: string; text: string }; songUrl?: string; lyrics?: Lyrics | null } = {}): JobSpec {
  const base = { key: p.key, lang, title: p.title, image: p.image, discoverId: p.discover?.id };
  if (p.kind === "text") {
    return extra.article
      ? { ...base, article: { title: extra.article.title || p.title, text: extra.article.text, source: p.page || p.url, collection: p.collection } }
      : { ...base, articleFrom: p.url };
  }
  if (p.song) {
    const s = p.song;
    return {
      ...base,
      song: {
        url: s.video ? p.url : (extra.songUrl ?? ""),
        artist: s.artist,
        title: s.title,
        album: s.album,
        duration: p.duration,
        image: s.video ? "" : p.image,
        page: s.video ? p.page : p.page || "",
        lyrics: extra.lyrics ?? s.lyrics,
        video: s.video,
      },
    };
  }
  return { ...base, media: mediaOf(p), textFrom: p.pageText ? p.page : undefined };
}

interface PreviewStore {
  item: Previewable | null;
  open(p: Previewable): void;
  close(): void;
}

export const usePreview = create<PreviewStore>((set) => ({
  item: null,
  open: (item) => set({ item }),
  close: () => set({ item: null }),
}));
