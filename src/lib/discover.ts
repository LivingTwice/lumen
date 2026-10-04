// Découvrir : des leçons venues d'ailleurs. Le natif (discover.rs) lit chaque
// jour les sources choisies pour la langue ; l'interface range ce qu'elles
// proposent par niveau et par rayon, et en fait des leçons à la demande.
import { useEffect } from "react";
import { create } from "zustand";
import { api, errorText, isTauri } from "./api";
import { count, locale, t } from "./i18n";
import { roughEstimate } from "./level";
import { useApp } from "./store";
import type { DiscoverFeed, DiscoverItem, LangCode, LevelEstimate } from "./types";

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

/** Niveaux estimés par langue (le natif regroupe les mots connus par lemme). */
const useEstimates = create<{ by: Partial<Record<string, LevelEstimate>>; load(lang: LangCode): Promise<void> }>((set) => ({
  by: {},
  async load(lang) {
    try {
      const e = await api().levelEstimate(lang);
      set((s) => ({ by: { ...s.by, [lang]: e } }));
    } catch {
      /* le repli approché suffit */
    }
  },
}));

/** Niveau estimé d'une langue, recalculé quand les mots connus changent. */
export function useEstimate(lang: LangCode): LevelEstimate {
  const known = useApp((s) => s.knownCount);
  const e = useEstimates((s) => s.by[lang]);
  const load = useEstimates((s) => s.load);
  useEffect(() => {
    // le calcul (quelques dixièmes de seconde) attend que l'apprenant ait fini de marquer ses mots
    const timer = window.setTimeout(() => void load(lang), e ? 1500 : 0);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lang, known, load]);
  return e ?? roughEstimate(lang, known);
}

/** « 6 700 mots différents » : ce que compte l'estimation, dit simplement. */
export function unitsLabel(e: LevelEstimate): string {
  if (e.unit === "kanji") return count(e.units, "kanji connu", "kanji connus", "known kanji", "known kanji");
  if (e.unit === "syllables") return count(e.units, "syllabe connue", "syllabes connues", "known syllable", "known syllables");
  return `${e.exact ? "" : "≈ "}${count(e.units, "mot différent", "mots différents", "distinct word", "distinct words")}`;
}

/** Niveau de l'apprenant dans la langue active : choisi, sinon estimé. */
export function useLevel(lang: LangCode) {
  const chosen = Number(useApp((s) => s.settings[`level_${lang}`])) || 0;
  const known = useApp((s) => s.knownCount);
  const estimate = useEstimate(lang);
  return {
    level: chosen || estimate.level,
    auto: !chosen,
    estimated: estimate.level,
    estimate,
    known,
    /** `null` : revenir au niveau estimé */
    setLevel: (l: number | null) => void useApp.getState().setSetting(`level_${lang}`, l ? String(l) : ""),
  };
}

/** Niveau choisi ou estimé, hors des composants (messages). */
export function levelNow(lang: LangCode): number {
  const chosen = Number(useApp.getState().settings[`level_${lang}`]) || 0;
  return chosen || (useEstimates.getState().by[lang] ?? roughEstimate(lang, useApp.getState().knownCount)).level;
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
  /** relit les sources ; `quiet` : sans message à la fin ; `force` : toutes, même lues il y a peu */
  refresh(lang: LangCode, opts?: { quiet?: boolean; force?: boolean }): Promise<void>;
  /** charge la langue, et relit les sources dont le tour est passé */
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
    const force = opts.force ?? true;
    set((s) => ({ busy: { ...s.busy, [lang]: { progress: 0, stage: force ? "discover" : "" } } }));
    const toast = useApp.getState().toast;
    const before = new Set((get().feeds[lang]?.items ?? []).map((it) => it.id));
    // les trouvailles apparaissent au fil de la lecture, source après source
    let reload: number | null = null;
    const soon = () => {
      if (reload !== null) return;
      reload = window.setTimeout(() => {
        reload = null;
        void get().load(lang);
      }, 900);
    };
    try {
      const r = await api().discoverRefresh(lang, force, (e) => {
        if (e.type === "stage" && e.stage === "found") return soon();
        set((s) => {
          const cur = s.busy[lang] ?? { progress: 0, stage: "" };
          return { busy: { ...s.busy, [lang]: e.type === "stage" ? { progress: 0, stage: e.stage } : { ...cur, progress: e.value } } };
        });
      });
      if (reload !== null) window.clearTimeout(reload);
      const feed = await get().load(lang);
      if (opts.quiet) return;
      if (r.skipped) {
        toast(t("Tout est à jour : Lumen vient de regarder ces sources.", "Everything is up to date: Lumen just checked these sources."));
        return;
      }
      if (r.sources > 0 && r.failed.length === r.sources) {
        toast(t("Aucune source n'a répondu. Lumen a-t-il accès à Internet ?", "No source answered. Does Lumen have Internet access?"), "error");
        return;
      }
      const level = levelNow(lang);
      const fresh = (feed?.items ?? []).filter((it) => !before.has(it.id));
      const here = fresh.filter((it) => it.shelf === "music" || fits(it, level)).length;
      const silent =
        r.failed.length > 0
          ? t(` (${r.failed.length} sur ${r.sources} n'ont pas répondu)`, ` (${r.failed.length} of ${r.sources} didn't answer)`)
          : "";
      if (here > 0) toast(t(`${count(here, "nouveauté", "nouveautés", "new find", "new finds")} à votre niveau${silent}`, `${count(here, "nouveauté", "nouveautés", "new find", "new finds")} at your level${silent}`), "light");
      else if (fresh.length > 0)
        toast(
          t(
            `${count(fresh.length, "nouveauté", "nouveautés", "", "")}, à d'autres niveaux que le vôtre${silent}`,
            `${count(fresh.length, "", "", "new find", "new finds")}, at other levels than yours${silent}`,
          ),
        );
      else toast(t(`Rien de neuf pour l'instant : les sources n'ont rien publié depuis.${silent}`, `Nothing new for now: the sources haven't published anything since.${silent}`));
    } catch (e) {
      if (!opts.quiet) toast(errorText(e), "error");
    } finally {
      if (reload !== null) window.clearTimeout(reload);
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
    // lecture d'arrière-plan coupée dans les Réglages : seulement sur demande
    if (useApp.getState().settings.discover_auto === "0") return;
    // le natif ne relit que les sources dont le tour est passé ; la toute première lecture se fait sous les yeux
    await get().refresh(lang, { quiet: true, force: feed.items.length === 0 && feed.refreshed_at === 0 });
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

/** Nouveautés à ce niveau depuis la dernière visite (pastille de Découvrir). */
export function newAtLevel(feed: DiscoverFeed | undefined, seen: number, level: number): number {
  if (!feed) return 0;
  return feed.items.filter((it) => isNew(it, seen) && fits(it, level)).length;
}

/** Ce que l'entrée Découvrir de la barre latérale signale : nouveautés à votre niveau, et dernière visite (0 : jamais). */
export function useDiscoverNews(lang: LangCode): { fresh: number; seen: number } {
  const feed = useDiscover((s) => s.feeds[lang]);
  const load = useDiscover((s) => s.load);
  const seen = Number(useApp((s) => s.settings[`discover_seen_${lang}`])) || 0;
  const { level } = useLevel(lang);
  useEffect(() => {
    void load(lang);
  }, [lang, load]);
  return { fresh: newAtLevel(feed, seen, level), seen };
}
