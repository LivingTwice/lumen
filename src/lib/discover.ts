// Découvrir : des leçons venues d'ailleurs. Le natif (discover.rs) lit chaque
// jour les sources choisies pour la langue ; l'interface range ce qu'elles
// proposent par niveau et par rayon, et en fait des leçons à la demande.
import { create } from "zustand";
import { api, errorText, isTauri } from "./api";
import { count, locale, t } from "./i18n";
import { useApp } from "./store";
import type { DiscoverFeed, DiscoverItem, LangCode } from "./types";

export const LEVELS = ["A1", "A2", "B1", "B2", "C1"] as const;

/** 1 → « A1 », 5 → « C1 ». */
export function levelName(n: number): string {
  return LEVELS[Math.min(5, Math.max(1, Math.round(n))) - 1];
}

/** « A2 », ou « A2–B1 » pour une fourchette. */
export function levelRange(it: Pick<DiscoverItem, "lo" | "hi">): string {
  return it.lo === it.hi ? levelName(it.lo) : `${levelName(it.lo)}–${levelName(it.hi)}`;
}

/** Titre sans les émojis de décor en tête ou en fin (« 🖼️ #74 | … 🔵 »). */
export function cleanTitle(s: string): string {
  const out = s
    .replace(/^[\p{Extended_Pictographic}\uFE0F\u200D\s]+/u, "")
    .replace(/[\p{Extended_Pictographic}\uFE0F\u200D\s]+$/u, "")
    .trim();
  return out || s;
}

/** L'élément convient à ce niveau. */
export function fits(it: Pick<DiscoverItem, "lo" | "hi">, level: number): boolean {
  return it.lo <= level && level <= it.hi;
}

/**
 * Niveau estimé d'après les mots connus. Ils sont comptés comme sur LingQ (chaque
 * forme d'un mot compte) : les paliers sont donc plus hauts que dans un manuel.
 */
export function estimateLevel(known: number): number {
  if (known < 800) return 1;
  if (known < 2500) return 2;
  if (known < 6000) return 3;
  if (known < 12000) return 4;
  return 5;
}

/** Niveau de l'apprenant dans la langue active : choisi, sinon estimé. */
export function useLevel(lang: LangCode) {
  const chosen = Number(useApp((s) => s.settings[`level_${lang}`])) || 0;
  const known = useApp((s) => s.knownCount);
  const estimated = estimateLevel(known);
  return {
    level: chosen || estimated,
    auto: !chosen,
    estimated,
    known,
    /** `null` : revenir au niveau estimé */
    setLevel: (l: number | null) => void useApp.getState().setSetting(`level_${lang}`, l ? String(l) : ""),
  };
}

/** Dernière visite de Découvrir dans cette langue (secondes, 0 : jamais). */
export function seenAt(lang: LangCode): number {
  return Number(useApp.getState().settings[`discover_seen_${lang}`]) || 0;
}

export function markSeen(lang: LangCode) {
  void useApp.getState().setSetting(`discover_seen_${lang}`, String(Math.round(Date.now() / 1000)));
}

/** Apparu depuis la dernière visite (la toute première visite ne marque rien de nouveau). */
export function isNew(it: DiscoverItem, seen: number): boolean {
  return seen > 0 && it.fetched_at > seen && it.lesson_id === null;
}

/** « aujourd'hui », « hier », « il y a 3 j », « il y a 2 sem. », sinon la date. */
export function ago(ts: number): string {
  if (!ts) return "";
  const days = Math.floor((Date.now() / 1000 - ts) / 86400);
  if (days <= 0) return t("aujourd'hui", "today");
  if (days === 1) return t("hier", "yesterday");
  if (days < 7) return t(`il y a ${days} j`, `${days} days ago`);
  if (days < 35) {
    const w = Math.round(days / 7);
    return t(`il y a ${w} sem.`, w === 1 ? "a week ago" : `${w} weeks ago`);
  }
  const d = new Date(ts * 1000);
  return d.toLocaleDateString(locale(), { day: "numeric", month: "short", year: d.getFullYear() === new Date().getFullYear() ? undefined : "numeric" });
}

/** Ouvre la page d'origine dans le navigateur. */
export async function openSource(url: string) {
  if (!url) return;
  if (isTauri) {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url).catch(() => {});
  } else window.open(url, "_blank", "noopener");
}

/** Une lecture est due : jamais faite, ou faite il y a plus de 20 h (le natif suit la même règle). */
const STALE = 20 * 3600;

interface Busy {
  /** 0 à 100 */
  progress: number;
  /** "tools" : installation des composants vidéo ; "discover" : lecture des sources */
  stage: string;
}

interface DiscoverStore {
  feeds: Partial<Record<string, DiscoverFeed>>;
  /** lecture demandée depuis l'interface, par langue */
  busy: Partial<Record<string, Busy>>;
  load(lang: LangCode): Promise<DiscoverFeed | null>;
  /** relit les sources ; `quiet` : sans message à la fin */
  refresh(lang: LangCode, opts?: { quiet?: boolean }): Promise<void>;
  /** charge la langue, et la relit si sa lecture du jour n'est pas faite */
  ensure(lang: LangCode): Promise<void>;
  hide(lang: LangCode, item: DiscoverItem): Promise<void>;
  /** l'élément est devenu une leçon */
  mark(lang: LangCode, id: string, lesson: number): Promise<void>;
}

let listening = false;

export const useDiscover = create<DiscoverStore>((set, get) => ({
  feeds: {},
  busy: {},

  async load(lang) {
    if (!listening) {
      listening = true;
      // lecture du jour faite en arrière-plan : la vue se met à jour d'elle-même
      void api()
        .discoverListen((l) => void get().load(l as LangCode))
        .catch(() => {
          listening = false;
        });
    }
    try {
      const feed = await api().discoverList(lang);
      set((s) => ({ feeds: { ...s.feeds, [lang]: feed } }));
      return feed;
    } catch {
      return null;
    }
  },

  async refresh(lang, opts = {}) {
    if (get().busy[lang]) return;
    set((s) => ({ busy: { ...s.busy, [lang]: { progress: 0, stage: "" } } }));
    const toast = useApp.getState().toast;
    try {
      const r = await api().discoverRefresh(lang, (e) =>
        set((s) => {
          const cur = s.busy[lang] ?? { progress: 0, stage: "" };
          return { busy: { ...s.busy, [lang]: e.type === "stage" ? { progress: 0, stage: e.stage } : { ...cur, progress: e.value } } };
        }),
      );
      await get().load(lang);
      if (!opts.quiet && !r.skipped) {
        if (r.sources > 0 && r.failed.length === r.sources)
          toast(t("Aucune source n'a répondu. Lumen a-t-il accès à Internet ?", "No source answered. Does Lumen have Internet access?"), "error");
        else if (r.added > 0) {
          const finds = count(r.added, "nouveauté", "nouveautés", "new find", "new finds");
          toast(t(`${finds} à découvrir`, `${finds} to discover`), "light");
        } else toast(t("Rien de neuf pour l'instant : revenez demain.", "Nothing new for now: come back tomorrow."));
      }
    } catch (e) {
      if (!opts.quiet) toast(errorText(e), "error");
    } finally {
      set((s) => {
        const busy = { ...s.busy };
        delete busy[lang];
        return { busy };
      });
    }
  },

  async ensure(lang) {
    const feed = get().feeds[lang] ?? (await get().load(lang));
    if (!feed || feed.refreshing || get().busy[lang]) return;
    // lecture quotidienne coupée dans les Réglages : seulement sur demande
    if (useApp.getState().settings.discover_auto === "0") return;
    // la première lecture se fait sous les yeux de l'apprenant ; ensuite, sans message
    if (Date.now() / 1000 - feed.refreshed_at > STALE) await get().refresh(lang, { quiet: feed.items.length > 0 });
  },

  async hide(lang, item) {
    set((s) => {
      const feed = s.feeds[lang];
      return feed ? { feeds: { ...s.feeds, [lang]: { ...feed, items: feed.items.filter((i) => i.id !== item.id) } } } : {};
    });
    await api()
      .discoverHide(item.id)
      .catch(() => {});
  },

  async mark(lang, id, lesson) {
    await api()
      .discoverMark(id, lesson)
      .catch(() => {});
    await get().load(lang);
  },
}));

/** Nouveautés à ce niveau depuis la dernière visite (pastille de l'onglet Découvrir). */
export function newAtLevel(feed: DiscoverFeed | undefined, seen: number, level: number): number {
  if (!feed) return 0;
  return feed.items.filter((it) => isNew(it, seen) && fits(it, level)).length;
}
