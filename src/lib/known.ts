// Ce qui est déjà dans la bibliothèque. Découvrir, la recherche et l'aperçu
// reconnaissent une leçon à son origine (même page, même vidéo) ou, pour une
// chanson, à son artiste et son titre : une chanson trouvée par la recherche
// (Deezer) et le clip du classement mènent à la même leçon. Relu à chaque
// changement de la bibliothèque : une leçon supprimée n'est plus signalée.
import { useEffect, useMemo } from "react";
import { create } from "zustand";
import { api } from "./api";
import { youtubeId } from "./covers";
import { useApp } from "./store";
import type { LangCode, LessonLink } from "./types";

/** Même page, même vidéo : miroir de `same_page` (discover.rs), YouTube sous toutes ses formes. */
export function sameLink(u: string): string {
  const s = u.trim();
  const yt = youtubeId(s);
  if (yt) return `yt:${yt}`;
  let url: URL;
  try {
    url = new URL(s);
  } catch {
    return s.toLowerCase();
  }
  const host = url.hostname.replace(/^(www\.)+/, "").replace(/^(m\.)+/, "").toLowerCase();
  if (host === "youtu.be") return `yt:${url.pathname.replace(/^\/+|\/+$/g, "")}`;
  if (host.endsWith("youtube.com")) {
    const v = url.searchParams.get("v");
    if (v) return `yt:${v}`;
  }
  return `${host}${url.pathname.replace(/\/+$/, "")}`;
}

/** Un titre ou un nom sans ce qui varie d'un service à l'autre (accents, « (Official Video) », « feat. »). */
function plain(s: string): string {
  return s
    .normalize("NFKD")
    .replace(/\p{M}/gu, "")
    .toLowerCase()
    .replace(/\([^)]*\)|\[[^\]]*\]/g, " ")
    .replace(/\s(?:feat\.?|ft\.?|featuring)\s.*$/, "")
    .replace(/\s[-–]\s.*$/, "")
    .replace(/[^\p{L}\p{N}]+/gu, " ")
    .trim();
}

/** Clé d'une chanson : le premier artiste et le titre. Vide s'il manque l'un des deux. */
export function songKey(artist: string, title: string): string {
  const first = artist.split(/\s*(?:,|&|\+|\/|\bx\b|\bfeat\.?|\bft\.?|\bfeaturing\b|\bwith\b)\s*/i)[0] ?? artist;
  const a = plain(first);
  const b = plain(title);
  return a && b ? `${a}|${b}` : "";
}

interface Index {
  ids: Set<number>;
  pages: Map<string, number>;
  songs: Map<string, number>;
}

const indexes = new WeakMap<LessonLink[], Index>();

function indexOf(links: LessonLink[]): Index {
  const known = indexes.get(links);
  if (known) return known;
  const ix: Index = { ids: new Set(), pages: new Map(), songs: new Map() };
  for (const l of links) {
    ix.ids.add(l.id);
    if (l.source) ix.pages.set(sameLink(l.source), l.id);
    // une chanson est une leçon audio ou vidéo : titre, et artiste pour collection
    if ((l.kind === "audio" || l.kind === "video") && l.collection) {
      const k = songKey(l.collection, l.title);
      if (k && !ix.songs.has(k)) ix.songs.set(k, l.id);
    }
  }
  indexes.set(links, ix);
  return ix;
}

/** Ce qu'on cherche dans la bibliothèque : adresses, chanson, leçon déjà reliée par Découvrir. */
export interface Findable {
  url?: string;
  page?: string;
  artist?: string;
  title?: string;
  lesson?: number | null;
}

function lessonIn(ix: Index | null, x: Findable): number | null {
  // pas encore lu : ce que dit le natif
  if (!ix) return x.lesson ?? null;
  if (x.lesson && ix.ids.has(x.lesson)) return x.lesson;
  for (const u of [x.page, x.url]) {
    const id = u ? ix.pages.get(sameLink(u)) : undefined;
    if (id) return id;
  }
  const k = x.artist && x.title ? songKey(x.artist, x.title) : "";
  return (k && ix.songs.get(k)) || null;
}

interface KnownStore {
  links: Partial<Record<LangCode, LessonLink[]>>;
  /** version de la bibliothèque lue pour chaque langue */
  stamp: Partial<Record<LangCode, number>>;
  load(lang: LangCode): Promise<void>;
}

const inflight = new Map<string, Promise<void>>();

export const useKnown = create<KnownStore>((set, get) => ({
  links: {},
  stamp: {},
  load(lang) {
    const v = useApp.getState().libraryVersion;
    if (get().stamp[lang] === v) return Promise.resolve();
    const key = `${lang}:${v}`;
    let p = inflight.get(key);
    if (!p) {
      p = api()
        .lessonLinks(lang)
        .then((links) => set((s) => ({ links: { ...s.links, [lang]: links }, stamp: { ...s.stamp, [lang]: v } })))
        .catch(() => {})
        .finally(() => inflight.delete(key));
      inflight.set(key, p);
    }
    return p;
  },
}));

/** La leçon de la bibliothèque qui correspond (ou null). Relue quand la bibliothèque change. */
export function useLessonOf(lang: LangCode): (x: Findable) => number | null {
  const links = useKnown((s) => s.links[lang]);
  const load = useKnown((s) => s.load);
  const version = useApp((s) => s.libraryVersion);
  useEffect(() => {
    void load(lang);
  }, [lang, version, load]);
  return useMemo(() => {
    const ix = links ? indexOf(links) : null;
    return (x: Findable) => lessonIn(ix, x);
  }, [links]);
}
