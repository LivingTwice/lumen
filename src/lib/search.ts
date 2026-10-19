// Chercher en ligne, depuis Découvrir : YouTube, chansons, podcasts, articles,
// Dailymotion. Le natif interroge chaque plateforme (search.rs) ; ici, les
// résultats par plateforme, les pages suivantes et les recherches récentes.
import { useMemo } from "react";
import { create } from "zustand";
import { api, errorText } from "./api";
import { t } from "./i18n";
import { useApp } from "./store";
import type { LangCode, LinkInfo, SearchHit, SearchPlatform } from "./types";

/** « Tout » montre un aperçu de chaque plateforme. */
export type SearchTab = "all" | SearchPlatform;
/** Durée des vidéos et des épisodes. */
export type Length = "" | "short" | "medium" | "long";
/** Ordre des résultats. */
export type SortBy = "relevance" | "recent" | "popular" | "old";
/** Date de publication (« older » : il y a plus d'un an). */
export type When = "" | "day" | "week" | "month" | "year" | "older";

/** Filtres de la recherche (et des trouvailles de Découvrir), gardés dans le réglage `search_filters`. */
export interface SearchFilters {
  sort: SortBy;
  when: When;
  length: Length;
  /** vidéos sous-titrées (YouTube) */
  subtitles: boolean;
  /** cacher ce qui semble être dans une autre langue */
  onlyLang: boolean;
  /** cacher ce qui est déjà dans la bibliothèque */
  hideKnown: boolean;
}

export const NO_FILTERS: SearchFilters = { sort: "relevance", when: "", length: "", subtitles: false, onlyLang: false, hideKnown: false };

const SORTS: SortBy[] = ["relevance", "recent", "popular", "old"];
const WHENS: When[] = ["", "day", "week", "month", "year", "older"];
const LENGTHS: Length[] = ["", "short", "medium", "long"];

/** Filtres lus dans le réglage (une valeur inconnue revient à la valeur par défaut). */
export function filtersOf(raw: string | undefined): SearchFilters {
  let v: Partial<SearchFilters> = {};
  try {
    v = raw ? JSON.parse(raw) : {};
  } catch {
    /* réglage illisible : les valeurs par défaut */
  }
  return {
    sort: SORTS.includes(v.sort as SortBy) ? (v.sort as SortBy) : "relevance",
    when: WHENS.includes(v.when as When) ? (v.when as When) : "",
    length: LENGTHS.includes(v.length as Length) ? (v.length as Length) : "",
    subtitles: v.subtitles === true,
    onlyLang: v.onlyLang === true,
    hideKnown: v.hideKnown === true,
  };
}

export function useFilters(): SearchFilters {
  const raw = useApp((s) => s.settings.search_filters);
  return useMemo(() => filtersOf(raw), [raw]);
}

/** Filtres choisis en plus du tri (pastille du bouton « Filtres »). */
export function activeFilters(f: SearchFilters): number {
  return [f.when !== "", f.length !== "", f.subtitles, f.onlyLang, f.hideKnown].filter(Boolean).length;
}

/** Ce qu'un tri ou un filtre demande à une plateforme. */
export type Ability = "recent" | "old" | "popular" | "when" | "length" | "subtitles";

/**
 * Ce que chaque plateforme sait faire. « feed » : les trouvailles de Découvrir, sans
 * popularité connue. Les chansons ne se classent que par popularité (Deezer), les
 * articles par date de création ou par nombre de liens (Wikipédia).
 */
export function supports(p: SearchPlatform | "feed", what: Ability): boolean {
  switch (p) {
    case "youtube":
      return true;
    case "dailymotion":
      return what !== "subtitles";
    case "podcast":
    case "feed":
      return what !== "popular" && what !== "subtitles";
    case "music":
      return what === "popular";
    case "wiki":
      return what === "recent" || what === "old" || what === "popular";
  }
}

/** Ce que l'onglet permet : au moins une de ses plateformes. */
export function tabSupports(tab: SearchTab | "feed", what: Ability): boolean {
  if (tab === "feed") return supports("feed", what);
  return (tab === "all" ? PLATFORMS : [tab]).some((p) => supports(p, what));
}

/** Le filtre envoyé au natif (`search::Filters`) : seulement ce que la plateforme sait faire. */
function nativeFilter(p: SearchPlatform, f: SearchFilters, window?: string): string {
  const parts: string[] = [];
  if (f.sort !== "relevance" && supports(p, f.sort)) parts.push(`sort=${f.sort}`);
  if (f.when && supports(p, "when")) parts.push(`when=${f.when}`);
  if (f.length && supports(p, "length")) parts.push(`len=${f.length}`);
  if (f.subtitles && supports(p, "subtitles")) parts.push("cc=1");
  if (window !== undefined && parts.length) parts.push(`win=${window}`);
  return parts.join(";");
}

/** La période qu'a retenue YouTube pour un tri par date, dite simplement. */
export function windowText(w: string): string {
  switch (w) {
    case "up:2":
      return t("Vidéos publiées aujourd'hui", "Videos published today");
    case "up:3":
      return t("Vidéos publiées cette semaine", "Videos published this week");
    case "up:4":
      return t("Vidéos publiées ce mois-ci", "Videos published this month");
    case "up:5":
      return t("Vidéos publiées cette année", "Videos published this year");
  }
  const before = w.match(/^before:(\d{4})-(\d{2})-(\d{2})$/);
  if (before) {
    const [, y, m, d] = before;
    if (m === "01" && d === "01") return t(`Vidéos publiées avant ${y}`, `Videos published before ${y}`);
    const day = new Date(Number(y), Number(m) - 1, Number(d)).toLocaleDateString(t("fr-FR", "en-US"), { day: "numeric", month: "long", year: "numeric" });
    return t(`Vidéos publiées avant le ${day}`, `Videos published before ${day}`);
  }
  return "";
}

export const PLATFORMS: SearchPlatform[] = ["youtube", "music", "podcast", "wiki", "dailymotion"];

export function platformName(p: SearchTab): string {
  switch (p) {
    case "all":
      return t("Tout", "All");
    case "youtube":
      return "YouTube";
    case "music":
      return t("Chansons", "Songs");
    case "podcast":
      return "Podcasts";
    case "wiki":
      return t("Articles", "Articles");
    case "dailymotion":
      return "Dailymotion";
  }
}

export interface Bucket {
  hits: SearchHit[];
  page: number;
  more: boolean;
  loading: boolean;
  error: string | null;
  /** période retenue par YouTube pour un tri par date (voir `SearchPage.window`) */
  window: string;
}

/** Une adresse collée dans le champ : on regarde ce qu'il y a derrière. */
export const URL_LIKE = /^(https?:\/\/|www\.)\S+$|^[a-z0-9-]+(\.[a-z0-9-]+)*\.(com|fr|net|org|it|de|es|tv|be|ch|io|co|uk|ly|app|link)(\/\S*)?$/i;

interface SearchStore {
  /** recherche lancée (vide : pas de recherche) */
  query: string;
  lang: LangCode | null;
  tab: SearchTab;
  buckets: Partial<Record<SearchPlatform, Bucket>>;
  /** composants vidéo en cours d'installation (première recherche YouTube) */
  installing: boolean;
  /** adresse collée : ce que Lumen y a trouvé */
  link: { loading: boolean; info: LinkInfo | null; error: string | null } | null;
  /** `tab` : la catégorie choisie en même temps (un onglet touché avec du texte tapé) */
  run(lang: LangCode, query: string, tab?: SearchTab): void;
  setTab(tab: SearchTab): void;
  /** change des filtres (gardés dans les réglages) ; les plateformes concernées cherchent de nouveau */
  setFilters(patch: Partial<SearchFilters>): void;
  more(platform: SearchPlatform): void;
  clear(): void;
}

/** Jetons par plateforme : la réponse d'une recherche dépassée est ignorée. */
const turns: Partial<Record<SearchPlatform | "link", number>> = {};
function bump(keys: (SearchPlatform | "link")[]) {
  for (const k of keys) turns[k] = (turns[k] ?? 0) + 1;
}

const empty = (): Bucket => ({ hits: [], page: 0, more: false, loading: true, error: null, window: "" });

async function fetchBucket(platform: SearchPlatform, page: number) {
  const mine = turns[platform];
  const { lang, query, buckets } = useSearch.getState();
  if (!lang) return;
  // les pages suivantes gardent la période retenue à la première
  const filter = nativeFilter(platform, filtersOf(useApp.getState().settings.search_filters), page ? (buckets[platform]?.window ?? "") : undefined);
  try {
    const r = await api().searchOnline(lang, platform, query, page, filter, (e) => {
      if (e.type === "stage" && e.stage === "tools") useSearch.setState({ installing: true });
    });
    if (mine !== turns[platform]) return;
    useSearch.setState((s) => {
      const cur = s.buckets[platform] ?? empty();
      // pas deux fois le même résultat d'une page à l'autre
      const seen = new Set(page ? cur.hits.map((h) => h.id) : []);
      const hits = page ? [...cur.hits, ...r.hits.filter((h) => !seen.has(h.id))] : r.hits;
      const window = page ? cur.window : (r.window ?? "");
      return { installing: false, buckets: { ...s.buckets, [platform]: { hits, page, more: r.more, loading: false, error: null, window } } };
    });
  } catch (e) {
    if (mine !== turns[platform]) return;
    useSearch.setState((s) => ({ installing: false, buckets: { ...s.buckets, [platform]: { ...(s.buckets[platform] ?? empty()), loading: false, error: errorText(e) } } }));
  }
}

/** Plateformes à interroger pour l'onglet affiché. */
function wanted(tab: SearchTab): SearchPlatform[] {
  return tab === "all" ? PLATFORMS : [tab];
}

function start(platforms: SearchPlatform[]) {
  const st = useSearch.getState();
  for (const p of platforms) {
    if (st.buckets[p]) continue;
    useSearch.setState((s) => ({ buckets: { ...s.buckets, [p]: empty() } }));
    void fetchBucket(p, 0);
  }
}

export const useSearch = create<SearchStore>((set, get) => ({
  query: "",
  lang: null,
  tab: "all",
  buckets: {},
  installing: false,
  link: null,

  run(lang, raw, tab) {
    if (tab) set({ tab });
    const query = raw.trim();
    bump([...PLATFORMS, "link"]);
    if (!query) {
      set({ query: "", buckets: {}, link: null, installing: false });
      return;
    }
    set({ query, lang, buckets: {}, link: null });
    remember(lang, query);
    if (URL_LIKE.test(query)) {
      // un lien (TikTok, Instagram, Vimeo, un article…) : on regarde ce qu'il contient
      const mine = turns.link;
      set({ link: { loading: true, info: null, error: null } });
      api()
        .linkProbe(query, () => {})
        .then((info) => mine === turns.link && set({ link: { loading: false, info, error: null } }))
        .catch((e) => mine === turns.link && set({ link: { loading: false, info: null, error: errorText(e) } }));
      return;
    }
    start(wanted(get().tab));
  },

  setTab(tab) {
    set({ tab });
    if (get().query && !get().link) start(wanted(tab));
  },

  setFilters(patch) {
    const app = useApp.getState();
    const cur = filtersOf(app.settings.search_filters);
    const next = { ...cur, ...patch };
    void app.setSetting("search_filters", JSON.stringify(next));
    // seules les plateformes dont la demande change cherchent de nouveau (« Seulement en
    // italien », « Masquer ma bibliothèque » se font à l'affichage)
    const changed = PLATFORMS.filter((p) => nativeFilter(p, cur) !== nativeFilter(p, next));
    if (!changed.length) return;
    bump(changed);
    set((s) => {
      const buckets = { ...s.buckets };
      for (const p of changed) delete buckets[p];
      return { buckets };
    });
    if (get().query && !get().link) start(wanted(get().tab).filter((p) => changed.includes(p)));
  },

  more(platform) {
    const b = get().buckets[platform];
    if (!b || b.loading || !b.more) return;
    set((s) => ({ buckets: { ...s.buckets, [platform]: { ...b, loading: true } } }));
    void fetchBucket(platform, b.page + 1);
  },

  clear() {
    bump([...PLATFORMS, "link"]);
    set({ query: "", buckets: {}, link: null, installing: false });
  },
}));

// ---------- recherches récentes (par langue) ----------

const RECENT_MAX = 8;

export function recentSearches(lang: LangCode): string[] {
  try {
    const v = JSON.parse(useApp.getState().settings[`search_recent_${lang}`] || "[]");
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

function remember(lang: LangCode, query: string) {
  if (URL_LIKE.test(query)) return;
  const list = [query, ...recentSearches(lang).filter((q) => q.toLowerCase() !== query.toLowerCase())].slice(0, RECENT_MAX);
  void useApp.getState().setSetting(`search_recent_${lang}`, JSON.stringify(list));
}

export function forgetSearches(lang: LangCode) {
  void useApp.getState().setSetting(`search_recent_${lang}`, "[]");
}

// ---------- idées de recherche, dans la langue étudiée ----------

/** Thèmes proposés quand le champ est vide (mots simples, dans la langue étudiée). */
const IDEAS: Partial<Record<LangCode, string[]>> = {
  en: ["cooking", "travel vlog", "history", "science", "short stories", "street interviews", "podcast", "news"],
  es: ["cocina", "viajes", "historia", "ciencia", "cuentos", "entrevistas en la calle", "pódcast", "noticias"],
  fr: ["cuisine", "voyage", "histoire", "science", "contes", "micro-trottoir", "podcast", "actualités"],
  de: ["Kochen", "Reisen", "Geschichte", "Wissenschaft", "Märchen", "Straßenumfrage", "Podcast", "Nachrichten"],
  it: ["cucina", "viaggi", "storia", "scienza", "fiabe", "interviste per strada", "podcast", "notizie"],
  pt: ["culinária", "viagem", "história", "ciência", "contos", "entrevistas na rua", "podcast", "notícias"],
  ru: ["кулинария", "путешествия", "история", "наука", "сказки", "опрос на улице", "подкаст", "новости"],
  nl: ["koken", "reizen", "geschiedenis", "wetenschap", "sprookjes", "straatinterviews", "podcast", "nieuws"],
  sv: ["matlagning", "resor", "historia", "vetenskap", "sagor", "gatuintervjuer", "podd", "nyheter"],
  da: ["madlavning", "rejser", "historie", "videnskab", "eventyr", "gadeinterview", "podcast", "nyheder"],
  fi: ["ruoanlaitto", "matkailu", "historia", "tiede", "sadut", "katuhaastattelu", "podcast", "uutiset"],
  et: ["toiduvalmistamine", "reisimine", "ajalugu", "teadus", "muinasjutud", "tänavaintervjuu", "podcast", "uudised"],
  lv: ["ēdiena gatavošana", "ceļojumi", "vēsture", "zinātne", "pasakas", "intervijas uz ielas", "podkāsts", "ziņas"],
  lt: ["maisto gaminimas", "kelionės", "istorija", "mokslas", "pasakos", "gatvės interviu", "tinklalaidė", "naujienos"],
  pl: ["gotowanie", "podróże", "historia", "nauka", "bajki", "sonda uliczna", "podcast", "wiadomości"],
  cs: ["vaření", "cestování", "historie", "věda", "pohádky", "anketa na ulici", "podcast", "zprávy"],
  sk: ["varenie", "cestovanie", "história", "veda", "rozprávky", "anketa na ulici", "podcast", "správy"],
  sl: ["kuhanje", "potovanja", "zgodovina", "znanost", "pravljice", "ulična anketa", "podkast", "novice"],
  hr: ["kuhanje", "putovanja", "povijest", "znanost", "bajke", "ulična anketa", "podcast", "vijesti"],
  hu: ["főzés", "utazás", "történelem", "tudomány", "mesék", "utcai interjú", "podcast", "hírek"],
  ro: ["gătit", "călătorii", "istorie", "știință", "povești", "interviuri pe stradă", "podcast", "știri"],
  bg: ["готвене", "пътувания", "история", "наука", "приказки", "анкета на улицата", "подкаст", "новини"],
  uk: ["кулінарія", "подорожі", "історія", "наука", "казки", "опитування на вулиці", "подкаст", "новини"],
  el: ["μαγειρική", "ταξίδια", "ιστορία", "επιστήμη", "παραμύθια", "συνεντεύξεις στον δρόμο", "podcast", "ειδήσεις"],
  tr: ["yemek tarifleri", "seyahat", "tarih", "bilim", "masallar", "sokak röportajı", "podcast", "haberler"],
  ar: ["طبخ", "سفر", "تاريخ", "علوم", "قصص", "مقابلات في الشارع", "بودكاست", "أخبار"],
  hi: ["खाना बनाना", "यात्रा", "इतिहास", "विज्ञान", "कहानियाँ", "पॉडकास्ट", "समाचार"],
  id: ["masak", "jalan-jalan", "sejarah", "sains", "dongeng", "wawancara jalanan", "podcast", "berita"],
  vi: ["nấu ăn", "du lịch", "lịch sử", "khoa học", "truyện cổ tích", "phỏng vấn đường phố", "podcast", "tin tức"],
  ko: ["요리", "여행", "역사", "과학", "동화", "길거리 인터뷰", "팟캐스트", "뉴스"],
  ja: ["料理", "旅行", "歴史", "科学", "昔話", "街頭インタビュー", "ポッドキャスト", "ニュース"],
};

export function searchIdeas(lang: LangCode): string[] {
  return IDEAS[lang] ?? IDEAS.en!;
}
