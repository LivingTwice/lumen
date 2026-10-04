// Chercher en ligne, depuis Découvrir : YouTube, chansons, podcasts, articles,
// Dailymotion. Le natif interroge chaque plateforme (search.rs) ; ici, les
// résultats par plateforme, les pages suivantes et les recherches récentes.
import { create } from "zustand";
import { api, errorText } from "./api";
import { t } from "./i18n";
import { useApp } from "./store";
import type { LangCode, LinkInfo, SearchHit, SearchPlatform } from "./types";

/** « Tout » montre un aperçu de chaque plateforme. */
export type SearchTab = "all" | SearchPlatform;
/** Durée des vidéos (YouTube, Dailymotion). */
export type Length = "" | "short" | "medium" | "long";

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
}

/** Une adresse collée dans le champ : on regarde ce qu'il y a derrière. */
export const URL_LIKE = /^(https?:\/\/|www\.)\S+$|^[a-z0-9-]+(\.[a-z0-9-]+)*\.(com|fr|net|org|it|de|es|tv|be|ch|io|co|uk|ly|app|link)(\/\S*)?$/i;

interface SearchStore {
  /** recherche lancée (vide : pas de recherche) */
  query: string;
  lang: LangCode | null;
  tab: SearchTab;
  length: Length;
  buckets: Partial<Record<SearchPlatform, Bucket>>;
  /** composants vidéo en cours d'installation (première recherche YouTube) */
  installing: boolean;
  /** adresse collée : ce que Lumen y a trouvé */
  link: { loading: boolean; info: LinkInfo | null; error: string | null } | null;
  /** `tab` : la catégorie choisie en même temps (un onglet touché avec du texte tapé) */
  run(lang: LangCode, query: string, tab?: SearchTab): void;
  setTab(tab: SearchTab): void;
  setLength(length: Length): void;
  more(platform: SearchPlatform): void;
  clear(): void;
}

/** Jetons par plateforme : la réponse d'une recherche dépassée est ignorée. */
const turns: Partial<Record<SearchPlatform | "link", number>> = {};
function bump(keys: (SearchPlatform | "link")[]) {
  for (const k of keys) turns[k] = (turns[k] ?? 0) + 1;
}

const empty = (): Bucket => ({ hits: [], page: 0, more: false, loading: true, error: null });

async function fetchBucket(platform: SearchPlatform, page: number) {
  const mine = turns[platform];
  const { lang, query, length } = useSearch.getState();
  if (!lang) return;
  try {
    const r = await api().searchOnline(lang, platform, query, page, length, (e) => {
      if (e.type === "stage" && e.stage === "tools") useSearch.setState({ installing: true });
    });
    if (mine !== turns[platform]) return;
    useSearch.setState((s) => {
      const cur = s.buckets[platform] ?? empty();
      // pas deux fois le même résultat d'une page à l'autre
      const seen = new Set(page ? cur.hits.map((h) => h.id) : []);
      const hits = page ? [...cur.hits, ...r.hits.filter((h) => !seen.has(h.id))] : r.hits;
      return { installing: false, buckets: { ...s.buckets, [platform]: { hits, page, more: r.more, loading: false, error: null } } };
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
  length: "",
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

  setLength(length) {
    if (length === get().length) return;
    bump(["youtube", "dailymotion"]);
    // la durée ne concerne que les vidéos : les autres résultats restent
    set((s) => {
      const buckets = { ...s.buckets };
      delete buckets.youtube;
      delete buckets.dailymotion;
      return { length, buckets };
    });
    if (get().query && !get().link) start(wanted(get().tab).filter((p) => p === "youtube" || p === "dailymotion"));
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
