// Backend simulé, utilisé uniquement quand l'interface tourne dans un
// navigateur (développement et tests visuels). Dans l'application, tout passe
// par le backend Rust.
import type { Api, TermUpdate } from "./api";
import { t } from "./i18n";
import pkg from "../../package.json";
import { LANGS, STARTERS, bundledDict } from "./langs";
import { tokenize, normalize } from "./tokenize";
import { roughEstimate } from "./level";
import type {
  BackupCounts,
  BackupInfo,
  BackupPlace,
  BackupStatus,
  ChatMessage,
  ChatSummary,
  DayStat,
  DiscoverItem,
  GoalReached,
  LangCode,
  Lesson,
  LessonSummary,
  LinkInfo,
  LinkMedia,
  Lyrics,
  ModelRow,
  SearchHit,
  SearchPlatform,
  NewLesson,
  Playlist,
  Span,
  Stats,
  Term,
} from "./types";

interface Db {
  settings: Record<string, string>;
  lessons: Lesson[];
  terms: Record<string, Term>; // clé : lang|term
  activity: Record<string, DayStat & { lang: string }>;
  nextId: number;
  installed: string[];
  playlists?: Playlist[];
  chats?: (Omit<ChatSummary, "lesson_title" | "count" | "preview"> & { messages: ChatMessage[] })[];
  backup?: { last_at: number | null; size: number };
}

const KEY = "lumen-mock-db";

function load(): Db {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) return JSON.parse(raw);
  } catch {
    /* stockage indisponible */
  }
  return { settings: {}, lessons: [], terms: {}, activity: {}, nextId: 1, installed: [] };
}

function save(db: Db) {
  try {
    localStorage.setItem(KEY, JSON.stringify(db));
  } catch {
    /* stockage indisponible */
  }
}

/** Jour local (AAAA-MM-JJ), comme le natif. */
const ymd = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
const today = () => ymd(new Date());
const addDays = (d: Date, n: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);
const parseDay = (s: string) => new Date(Number(s.slice(0, 4)), Number(s.slice(5, 7)) - 1, Number(s.slice(8, 10)));
const emptyDay = (day: string): DayStat => ({ day, words_read: 0, known_added: 0, lingqs: 0, listen_secs: 0, learn_secs: 0, goal_met: false });
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

let lingqCancelled = false;
/** réponses du chat en cours (identifiant de conversation → arrêt demandé) */
const chatStops = new Map<number, boolean>();
const STARTERS_MOCK =
  "Every morning, Martha climbed the narrow stairs of the old lighthouse. From the top, the sea looked endless and calm.\n\nOne day, she found a letter hidden between two stones. The paper was damp, but the words could still be read.";

// Découvrir : un échantillon réel en italien (miniatures de YouTube et des
// podcasts), des éléments à l'œuvre générée pour les autres langues
type MockFind = Pick<DiscoverItem, "source" | "source_name" | "shelf" | "kind" | "title" | "lo" | "hi" | "duration"> & {
  yt?: string;
  /** chanson : artiste */
  artist?: string;
  image?: string;
  /** âge en jours */
  age: number;
};
const IT_FINDS: MockFind[] = [
  ...(
    [
      ["uwSAbdqNa4U", "🖼️ #74 | An unusual picnic | Italian for Beginners (A2) 🔵", 2, 2, 823, 1],
      ["IlK7I9UbCmc", "🖼️ #56 | Small Objects | Italian for Absolute Beginners (A0–A1) 🟢", 1, 1, 1109, 3],
      ["gC8709ukvEk", "🎙️ Ep. 138 | Di sudore, puzza e brufoli", 1, 3, 1477, 4],
      ["XtGHWl4dKhI", "🎮 A little to the left #7 | Cat toys | Italian for Absolute Beginners (A0–A1) 🟢", 1, 1, 857, 6],
      ["dm5BGYkl9lE", "🖼️ #73 | Magnetic fish | Italian for Beginners (A2) 🔵", 2, 2, 650, 8],
    ] as const
  ).map(([yt, title, lo, hi, duration, age]) => ({ yt, title, lo, hi, duration, age, source: "it-si", source_name: "Italiano sì", shelf: "learn", kind: "video" }) as MockFind),
  ...(
    [
      ["g-xVfw1MCdo", "Italians Disagree About What Retiring Here Costs | Easy Italian 275", 2, 3, 964, 2],
      ["fr-aDyyZO4M", "DOVERE: The Verb Italians Use All Day | Super Easy Italian 94", 1, 1, 467, 5],
      ["hfYubttHxfM", "What Italians Say You Must See - and How to Say It | Easy Italian 274", 2, 3, 792, 9],
      ["1YMnHT8_TE4", "Essere and Stare: When Italians Use Each One | Easy Italian 273", 2, 3, 543, 12],
      ["I30hFP22PyM", "Understand Spoken Italian With 4 Words in 20 | Super Easy Italian 93", 1, 1, 928, 15],
    ] as const
  ).map(([yt, title, lo, hi, duration, age]) => ({ yt, title, lo, hi, duration, age, source: "it-easy", source_name: "Easy Italian", shelf: "learn", kind: "video" }) as MockFind),
  ...(
    [
      ["hPNPzNs4yzQ", "Come parlare italiano INFORMALE", 1915, 1],
      ["Dq8ZuLSZzxw", "Perché gli italiani parlano così? Le frasi marcate", 1482, 6],
      ["IVGn8R1V84c", "Sai usare il CONDIZIONALE in italiano?", 1076, 11],
      ["oyPT-JXDOso", "MANZONI: l'uomo che cambiò l'italiano", 1563, 17],
    ] as const
  ).map(([yt, title, duration, age]) => ({ yt, title, lo: 3, hi: 5, duration, age, source: "it-podcast", source_name: "Podcast Italiano", shelf: "learn", kind: "video" }) as MockFind),
  ...(
    [
      ["CS0WGcKoQ3M", "I lavori di rinnovo dell'Aeroporto di Firenze: il progetto per ruotare la pista di atterraggio", 513, 2],
      ["SjWeqEoofJs", "La privacy non esiste più: come i social hanno cambiato la vita privata", 859, 4],
      ["GmAKhMSDj3A", "Sta davvero tornando il nucleare in Italia? La spiegazione semplice della legge delega", 746, 7],
      ["qbeXqZOrS80", "Donare il midollo osseo fa male? Come funzionano la donazione e il trapianto", 750, 10],
    ] as const
  ).map(([yt, title, duration, age]) => ({ yt, title, lo: 5, hi: 5, duration, age, source: "it-geopop", source_name: "Geopop", shelf: "culture", kind: "video" }) as MockFind),
  ...(
    [
      ["2642580/c1a-oxz7k-xxmpxkzzb1kq-ifj4ja.jpg", "G7 Oil Release, Environment Ruling & More | News in Easy Italian", 217, 0],
      ["2641660/c1a-oxz7k-kp5x4wwpcqk2-rkyvby.jpg", "Ethiopia-Eritrea Ties Cut, US Job Report & More | News in Easy Italian", 241, 1],
      ["2640063/c1a-oxz7k-z3ojxx0jh7w3-inzq1z.jpg", "US Ends Iraq Withdrawal & More | News in Easy Italian", 230, 2],
    ] as const
  ).map(([img, title, duration, age]) => ({
    image: `https://episodes.castos.com/69145abf642d78-65143310/images/${img}`,
    title,
    lo: 2,
    hi: 3,
    duration,
    age,
    source: "it-news-easy",
    source_name: "News In Easy Italian",
    shelf: "news",
    kind: "audio",
  }) as MockFind),
  ...(
    [
      ["Otto secoli di San Francesco in mostra ad Arezzo", 0],
      ["In Umbria nei luoghi di San Francesco a 800 anni dalla morte", 0],
      ["Maltempo, allerta arancione in Liguria e Toscana", 1],
    ] as const
  ).map(([title, age]) => ({ title, lo: 5, hi: 5, duration: 0, age, source: "it-ansa", source_name: "ANSA", shelf: "news", kind: "text" }) as MockFind),
  ...(
    [
      ["PshqcQwshdU", "Lontano", "Sfera Ebbasta", 238, 2],
      ["XgTSQwZcHH8", "PER NOI", "Geolier", 231, 3],
    ] as const
  ).map(([yt, title, artist, duration, age]) => ({ yt, title, artist, lo: 2, hi: 5, duration, age, source: "it-chart", source_name: "Top 100 · Italie", shelf: "music", kind: "video" }) as MockFind),
];

// Chansons : paroles inventées pour l'aperçu (jamais de vraies paroles ici)
const MOCK_LRC =
  "[00:02.00] La luce del mattino\n[00:06.00] Sopra il mare calmo\n[00:10.50] Il faro si accende piano\n[00:15.00]\n[00:17.00] Canta, canta la sera\n[00:21.00] Una lettera nascosta\n[00:25.00] Tra due pietre antiche\n[00:29.00] La luce del mattino\n[00:33.00] Ritorna sopra il mare";

/** Paroles inventées pour une langue : les phrases du texte de départ, une par ligne. */
function mockLyrics(lang: LangCode): Lyrics {
  if (lang === "it") return { synced: MOCK_LRC, plain: "", instrumental: false };
  const lines = STARTERS[lang].text.split(/(?<=[.!?。])\s+|\n+/).filter((l) => l.trim().length > 3).slice(0, 9);
  const synced = lines.map((l, i) => `[00:${String(2 + i * 4).padStart(2, "0")}.00] ${l.trim()}`).join("\n");
  return { synced, plain: "", instrumental: false };
}

/** Vidéo et son libres de droits, pour essayer l'aperçu dans le navigateur. */
const SAMPLE_VIDEO = "https://interactive-examples.mdn.mozilla.net/media/cc0-videos/flower.mp4";
const SAMPLE_AUDIO = "https://interactive-examples.mdn.mozilla.net/media/cc0-audio/t-rex-roar.mp3";

/** Éléments générés pour les autres langues : titres tirés du texte de départ. */
function genericFinds(lang: LangCode): MockFind[] {
  const words = STARTERS[lang].text.split(/(?<=[.!?。])\s+/).filter((s) => s.length > 12);
  const pick = (i: number) => (words[i % words.length] ?? STARTERS[lang].title).replace(/[«»"“”]/g, "").slice(0, 80);
  const rows: [string, string, MockFind["shelf"], MockFind["kind"], number, number, number][] = [
    ["learn-ci", "Comprehensible Input", "learn", "video", 1, 2, 640],
    ["learn-easy", `Easy ${LANGS.find((l) => l.code === lang)?.native ?? lang}`, "learn", "video", 2, 3, 910],
    ["learn-pod", "Slow Stories", "learn", "audio", 2, 3, 1320],
    ["news-easy", "News In Easy", "news", "audio", 2, 3, 236],
    ["news", "BBC News", "news", "text", 5, 5, 0],
    ["culture", "Kurzgesagt", "culture", "video", 4, 5, 610],
  ];
  return rows.flatMap(([id, name, shelf, kind, lo, hi, duration], r) =>
    [0, 1, 2].map((i) => ({ source: `${lang}-${id}`, source_name: name, shelf, kind, lo, hi, duration: duration ? duration + i * 97 : 0, title: i ? pick(r * 3 + i) : STARTERS[lang].title, age: r + i * 3 })),
  );
}

// nature et sens en français, puis en anglais (interface en anglais)
const MOCK_DICT: Record<string, [string, string, string, string, string]> = {
  lighthouse: ["Nom commun", "ˈlaɪt.haʊs", "Phare", "Noun", "A tower with a bright light that guides ships"],
  stairs: ["Nom commun", "stɛəz", "Escalier", "Noun", "A flight of steps"],
  narrow: ["Adjectif", "ˈnæɹ.əʊ", "Étroit", "Adjective", "Of little width"],
  endless: ["Adjectif", "ˈɛnd.ləs", "Sans fin, infini", "Adjective", "Having no end, infinite"],
  letter: ["Nom commun", "ˈlɛt.ə", "Lettre", "Noun", "A written message"],
  damp: ["Adjectif", "dæmp", "Humide", "Adjective", "Slightly wet"],
  faro: ["Nom commun", "ˈfa.ɾo", "Phare", "Noun", "Lighthouse"],
  carta: ["Nom commun", "ˈkaɾ.ta", "Lettre", "Noun", "Letter"],
};

export function createMockApi(): Api {
  let db = load();
  const commit = () => save(db);
  const playlists = () => (db.playlists ??= []);
  const chats = () => (db.chats ??= []);
  const chatSummary = (c: ReturnType<typeof chats>[number]): ChatSummary => {
    const last = c.messages[c.messages.length - 1];
    const { messages, ...rest } = c;
    return {
      ...rest,
      lesson_title: db.lessons.find((l) => l.id === c.lesson_id)?.title ?? null,
      count: messages.length,
      preview: last ? last.content.replace(/\*\*/g, "").replace(/\s+/g, " ").slice(0, 160) : "",
    };
  };
  /** mêmes règles que le natif : sans doublon, leçons de la langue de la playlist */
  const fill = (lang: string, ids: number[]) => [...new Set(ids)].filter((id) => db.lessons.some((l) => l.id === id && l.lang === lang));

  const bump = (lang: string, field: "words_read" | "known_added" | "lingqs" | "listen_secs" | "learn_secs", n: number) => {
    const k = `${today()}|${lang}`;
    const a = db.activity[k] ?? { ...emptyDay(today()), lang };
    (a[field] as number) = ((a[field] as number) ?? 0) + n;
    db.activity[k] = a;
  };

  /** objectif du jour en minutes (réglage `daily_goal`) */
  const goalMin = () => {
    const m = Number(db.settings.daily_goal);
    return m >= 1 && m <= 600 ? Math.round(m) : 10;
  };
  /** série en cours et record ; la série d'hier tient tant que la journée n'est pas finie */
  const streakOf = (rows: DayStat[], goal: number): Stats["streak"] => {
    const td = today();
    const met = new Set(rows.filter((d) => d.goal_met || (d.day === td && (d.learn_secs ?? 0) >= goal * 60)).map((d) => d.day));
    const done = met.has(td);
    let current = 0;
    for (let d = done ? new Date() : addDays(new Date(), -1); met.has(ymd(d)); d = addDays(d, -1)) current++;
    let best = 0;
    let run = 0;
    let prev = "";
    for (const d of [...met].sort()) {
      run = prev && ymd(addDays(parseDay(prev), 1)) === d ? run + 1 : 1;
      best = Math.max(best, run);
      prev = d;
    }
    return { current, best: Math.max(best, current), today_done: done, goal_min: goal, today_secs: rows.find((d) => d.day === td)?.learn_secs ?? 0 };
  };

  const models: Omit<ModelRow, "installed" | "active" | "downloading" | "partial">[] = [
    { id: "qwen3.5-0.8b", kind: "llm", name: "Qwen3.5 0.8B", get detail() { return t("Très rapide, pour les Mac avec 8 Go de mémoire", "Very fast, for Macs with 8 GB of memory"); }, size: 533e6, url: "", file: "", ram_gb: 8 },
    { id: "qwen3.5-2b", kind: "llm", name: "Qwen3.5 2B", get detail() { return t("L'équilibre idéal entre qualité et vitesse", "The ideal balance between quality and speed"); }, size: 1281e6, url: "", file: "", ram_gb: 8 },
    { id: "qwen3.5-4b", kind: "llm", name: "Qwen3.5 4B", get detail() { return t("Les traductions les plus fines, à partir de 16 Go", "The finest translations, 16 GB of memory or more"); }, size: 2741e6, url: "", file: "", ram_gb: 16 },
    { id: "qwen3-asr-1.7b", kind: "asrtext", name: "Qwen3-ASR 1.7B", get detail() { return t("Texte des transcriptions plus juste que Whisper, sans phrase sautée, dans 23 langues ; Whisper garde le minutage des mots", "More accurate transcripts than Whisper, with no skipped sentences, in 23 languages; Whisper still times the words"); }, size: 2520744288, url: "", file: "", ram_gb: 16 },
    { id: "whisper-small", kind: "asr", name: "Whisper Small", get detail() { return t("Transcription légère et rapide", "Light and fast transcription"); }, size: 190e6, url: "", file: "", ram_gb: 8 },
    { id: "whisper-turbo", kind: "asr", name: "Whisper Large v3 Turbo", get detail() { return t("Transcription et minutage des mots, très précis, 99 langues", "Transcription and word timing, very precise, 99 languages"); }, size: 574e6, url: "", file: "", ram_gb: 8 },
    { id: "supertonic-3", kind: "tts", name: "Supertonic 3", get detail() { return t("Voix naturelle pour les mots, les expressions et l'audio des leçons, dans les 31 langues", "Natural voice for words, phrases and lesson audio, in all 31 languages"); }, size: 149e6, url: "", file: "", ram_gb: 8 },
  ];

  // miroir de new_share (db.rs)
  const newShare = (fresh: number, distinct: number) => {
    if (!distinct || !fresh) return 0;
    if (fresh === distinct) return 100;
    return Math.min(99, Math.max(1, Math.round((fresh * 100) / distinct)));
  };

  const summary = (l: Lesson): LessonSummary => {
    const keys = tokenize(l.text, l.lang).filter((t) => t.w).map((t) => t.k);
    const uniq = new Set<string>();
    for (const k of keys) if (!db.terms[`${l.lang}|${k}`]) uniq.add(k);
    const distinct = new Set(keys).size;
    const newPct = newShare(uniq.size, distinct);
    return {
      id: l.id,
      lang: l.lang,
      title: l.title,
      collection: l.collection,
      kind: l.kind,
      source: l.source,
      hue: l.hue,
      word_count: l.word_count,
      page: l.page,
      completed: l.completed,
      has_media: !!l.media_path,
      created_at: (l as Lesson & { created_at?: number }).created_at ?? 0,
      opened_at: (l as Lesson & { opened_at?: number }).opened_at ?? null,
      new_words: uniq.size,
      new_pct: newPct,
      // même base que new_pct : les mots différents (comme lessons_list)
      known_pct: distinct ? 100 - newPct : 100,
      excerpt: l.text.slice(0, 220).replace(/\n/g, " "),
      position: l.position ?? 0,
      duration: l.duration ?? 0,
      cover_path: l.cover_path ?? null,
    };
  };

  // dictionnaires : ceux qui ne sont pas livrés « se téléchargent » en une seconde et demie
  const dictListeners = new Set<(lang: string) => void>();
  const dictReady = new Set<string>();
  const dictFetching = new Set<string>();
  const dictFetch = (lang: string) => {
    const id = `${t("fr", "en")}:${lang}`;
    if (dictReady.has(id) || dictFetching.has(id)) return;
    dictFetching.add(id);
    setTimeout(() => {
      dictFetching.delete(id);
      dictReady.add(id);
      dictListeners.forEach((f) => f(lang));
    }, 1500);
  };
  const dictState = (lang: string) => {
    const id = `${t("fr", "en")}:${lang}`;
    const bundled = bundledDict(lang);
    return { exists: lang !== t("fr", "en"), ready: bundled || dictReady.has(id), downloading: dictFetching.has(id), bundled };
  };

  // sauvegarde : ce navigateur, et une sauvegarde fictive d'un autre Mac pour essayer la restauration
  const backupListeners = new Set<(s: BackupStatus) => void>();
  const backupCounts = (): BackupCounts => {
    const terms = Object.values(db.terms);
    return {
      known: terms.filter((t) => t.status === 4 && !t.term.includes(" ")).length,
      learning: terms.filter((t) => t.status >= 1 && t.status <= 3).length,
      phrases: terms.filter((t) => t.term.includes(" ") && t.status !== 5).length,
      lessons: db.lessons.length,
      langs: (db.settings.langs ?? "").split(",").filter(Boolean) as LangCode[],
    };
  };
  // nuages de ce Mac simulé : iCloud Drive, Dropbox et un compte Google Drive
  const MOCK_HOME = "/Users/vous";
  const mockPlaces = (): BackupPlace[] => [
    { kind: "icloud", name: "iCloud Drive", account: null, path: "" },
    { kind: "dropbox", name: "Dropbox", account: null, path: `${MOCK_HOME}/Library/CloudStorage/Dropbox` },
    { kind: "gdrive", name: "Google Drive", account: "lea@gmail.com", path: `${MOCK_HOME}/Library/CloudStorage/GoogleDrive-lea@gmail.com` },
  ];
  const placeOf = (custom: string): BackupPlace => {
    if (!custom) return mockPlaces()[0];
    const known = mockPlaces().find((p) => p.path && (custom === p.path || custom.startsWith(`${p.path}/`)));
    if (known) return known;
    if (/^\/Users\/[^/]+\/Dropbox(\/|$)/.test(custom)) return { kind: "dropbox", name: "Dropbox", account: null, path: custom };
    const volume = custom.match(/^\/Volumes\/([^/]+)/);
    if (volume) return { kind: "drive", name: volume[1], account: null, path: volume[0] };
    return { kind: "folder", name: custom.split("/").filter(Boolean).pop() ?? custom, account: null, path: custom };
  };
  const backupStatus = (): BackupStatus => {
    const b = (db.backup ??= { last_at: null, size: 0 });
    const custom = db.settings.backup_dir ?? "";
    const saved = db.settings.backup_on === "1" && b.last_at !== null;
    return {
      enabled: db.settings.backup_on === "1",
      decided: !!db.settings.backup_on,
      dir: custom ? `${custom}/Lumen` : "~/Library/Mobile Documents/com~apple~CloudDocs/Lumen",
      icloud: !custom,
      icloud_available: true,
      place: placeOf(custom),
      running: false,
      last_at: saved ? b.last_at : null,
      size: saved ? b.size : 0,
      media_size: saved && db.settings.backup_audio !== "0" ? 186e6 : 0,
      media_count: saved ? 12 : 0,
      counts: backupCounts(),
      cloud: saved ? (!custom || placeOf(custom).kind === "dropbox" || placeOf(custom).kind === "gdrive" ? "uploaded" : "local") : "unknown",
      cloud_error: null,
      error: null,
      local_audio: 186e6,
      local_video: 1.4e9,
    };
  };
  const OTHER_MAC: BackupCounts = { known: 4210, learning: 812, phrases: 37, lessons: 52, langs: ["it", "en"] };
  const dayAgo = (n: number) => new Date(Date.now() - n * 86400e3).toISOString().slice(0, 10);

  /** IA en ligne choisie pour ce rôle (miroir de `online::config_from`) : erreur si la clé manque. */
  const online = (role: "words" | "chat"): boolean => {
    const s = db.settings;
    if (s.online_on !== "1" || s[role === "words" ? "online_words" : "online_chat"] === "0") return false;
    const id = s.online_provider || "deepseek";
    const names: Record<string, string> = { deepseek: "DeepSeek", gemini: "Gemini", mistral: "Mistral", openai: "OpenAI", anthropic: "Claude", openrouter: "OpenRouter" };
    if (id === "custom") {
      if (!s.online_url?.trim()) throw t("Indiquez l'adresse de votre serveur dans Réglages › IA.", "Enter your server's address in Settings › AI.");
      return true;
    }
    const key = id === "gemini" ? s.gemini_key : s[`online_key_${id}`];
    const name = names[id] ?? "DeepSeek";
    if (!key?.trim()) throw t(`Ajoutez votre clé ${name} dans Réglages › IA.`, `Add your ${name} key in Settings › AI.`);
    return true;
  };
  const fakeStream = async (text: string, onPiece: (t: string) => void) => {
    for (const part of text.match(/.{1,4}/gsu) ?? []) {
      await sleep(18);
      onPiece(part);
    }
    return text;
  };

  // Découvrir : la première lecture « interroge les sources » deux secondes et demie
  const discoverListeners = new Set<(lang: string) => void>();
  const discover = new Map<string, { items: DiscoverItem[]; at: number; hidden: Set<string>; marks: Map<string, number> }>();
  const discoverItems = (lang: LangCode, at: number): DiscoverItem[] =>
    (lang === "it" ? IT_FINDS : genericFinds(lang)).map((f, i) => {
      const id = `${f.source}-${i}`;
      const url = f.yt ? `https://www.youtube.com/watch?v=${f.yt}` : `https://example.org/${lang}/${id}`;
      return {
        id,
        source: f.source,
        source_name: f.source_name,
        shelf: f.shelf,
        kind: f.kind,
        title: f.title,
        url: f.kind === "audio" ? `https://cdn.example/${id}.mp3` : url,
        page: url,
        image: f.yt ? `https://i.ytimg.com/vi/${f.yt}/hqdefault.jpg` : (f.image ?? ""),
        summary: f.kind === "text" ? STARTERS[lang].text.slice(0, 180) + "…" : "",
        duration: f.duration,
        published: Math.round(Date.now() / 1000 - f.age * 86400 - i * 1800),
        lo: f.lo,
        hi: f.hi,
        page_text: false,
        fetched_at: at,
        lesson_id: null,
        artist: f.shelf === "music" ? (f.artist ?? f.source_name) : "",
        track: f.shelf === "music" ? f.title : "",
      };
    });

  /** Résultats de recherche simulés, plausibles pour chaque plateforme. */
  const searchHits = (lang: LangCode, platform: SearchPlatform, query: string, page: number): SearchHit[] => {
    const q = query.trim();
    const base = { summary: "", duration: 0, published: 0, count: 0, lo: 0, hi: 0, in_lang: true, other_lang: "", album: "", sample: "", lyrics: null, words: 0 };
    const words = STARTERS[lang].text.split(/(?<=[.!?。])\s+/).filter((x) => x.length > 12);
    const line = (i: number) => (words[i % words.length] ?? STARTERS[lang].title).slice(0, 70);
    const now = Math.round(Date.now() / 1000);
    if (platform === "youtube") {
      const finds = lang === "it" ? IT_FINDS.filter((f) => f.yt) : [];
      const list = finds.length ? finds : Array.from({ length: 8 }, (_, i) => ({ yt: "", title: `${q} · ${line(i)}`, duration: 300 + i * 91, lo: 0, hi: 0, source_name: "Lumen Stories" }) as MockFind);
      return list.slice(0, 12).map((f, i) => ({
        ...base,
        id: `yt:${f.yt || `mock${page}${i}`}`,
        platform,
        kind: "video",
        title: f.title,
        url: f.yt ? `https://www.youtube.com/watch?v=${f.yt}` : `https://www.youtube.com/watch?v=mock${page}${i}`,
        page: f.yt ? `https://www.youtube.com/watch?v=${f.yt}` : "",
        image: f.yt ? `https://i.ytimg.com/vi/${f.yt}/hqdefault.jpg` : "",
        author: f.source_name,
        duration: f.duration,
        count: 12000 + i * 4321,
        lo: f.lo,
        hi: f.hi,
        in_lang: i === 3 ? false : true,
        other_lang: i === 3 ? "en" : "",
      }));
    }
    if (platform === "dailymotion")
      return Array.from({ length: 6 }, (_, i) => ({ ...base, id: `dm:${page}${i}`, platform, kind: "video" as const, title: `${q} · ${line(i + 2)}`, url: `https://www.dailymotion.com/video/mock${page}${i}`, page: "", image: "", author: "Euronews", duration: 120 + i * 40, published: now - i * 86400 * 3, count: 830 + i * 77 }));
    if (platform === "podcast")
      return [
        ...["Slow Stories", "Café des mots", "Notes du soir"].map((name, i) => ({ ...base, id: `as:${i}`, platform, kind: "show" as const, title: name, url: `https://feeds.example/${i}.rss`, page: "", image: "", author: "Lumen Radio", summary: t("Société", "Society"), count: 40 + i * 12 })),
        ...Array.from({ length: 6 }, (_, i) => ({ ...base, id: `ap:${page}${i}`, platform, kind: "audio" as const, title: `${line(i)}`, url: `https://cdn.example/mock-${i}.mp3`, page: "", image: "", author: "Slow Stories", summary: line(i + 1), duration: 900 + i * 133, published: now - i * 86400 * 7 })),
      ];
    if (platform === "music")
      return ["La luce del mattino", "Il faro", "Mare calmo", "Lettera nascosta", "Canta la sera"].map((title, i) => ({
        ...base,
        id: `dz:${page}${i}`,
        platform,
        kind: "song" as const,
        title,
        url: "",
        page: "",
        image: "",
        author: i % 2 ? "Aurora Lumi" : "I Fari",
        album: "Lumière",
        duration: 190 + i * 21,
        lyrics: i === 4 ? null : mockLyrics(lang),
        in_lang: i === 4 ? null : true,
        words: 60,
      }));
    return [
      { ...base, id: `wk:v:${page}`, platform, kind: "text", title: q, url: `https://${lang}.vikidia.org/wiki/${encodeURIComponent(q)}`, page: "", image: "", author: "Vikidia", summary: line(0), lo: 3, hi: 4, words: 900 },
      { ...base, id: `wk:w:${page}`, platform, kind: "text", title: q, url: `https://${lang}.wikipedia.org/wiki/${encodeURIComponent(q)}`, page: "", image: "", author: "Wikipedia", summary: line(1), lo: 5, hi: 5, words: 4200 },
      { ...base, id: `wk:w2:${page}`, platform, kind: "text", title: `${q} (${t("histoire", "history")})`, url: `https://${lang}.wikipedia.org/wiki/${encodeURIComponent(q)}_2`, page: "", image: "", author: "Wikipedia", summary: line(2), lo: 5, hi: 5, words: 2400 },
    ];
  };

  const mock: Api = {
    async appInfo() {
      return {
        version: t(`${pkg.version} (aperçu navigateur)`, `${pkg.version} (browser preview)`),
        data_dir: t("(navigateur)", "(browser)"),
        platform: "web",
        ytdlp: false,
        transcriber: false,
        dict_langs: LANGS.map((l) => l.code).filter((c) => c !== t("fr", "en")),
      };
    },
    async settingsGet() {
      return { ...db.settings };
    },
    async settingsSet(key, value) {
      db.settings[key] = value;
      // nouvel objectif du jour : une journée déjà au-dessus est acquise
      if (key === "daily_goal") for (const a of Object.values(db.activity)) if (a.day === today() && (a.learn_secs ?? 0) >= goalMin() * 60) a.goal_met = true;
      commit();
    },
    async lessonsList(lang) {
      return db.lessons
        .filter((l) => l.lang === lang)
        .map(summary)
        .sort((a, b) => (b.opened_at ?? b.created_at) - (a.opened_at ?? a.created_at));
    },
    async lessonOpen(id) {
      const l = db.lessons.find((x) => x.id === id);
      if (!l) throw t("Leçon introuvable", "Lesson not found");
      (l as Lesson & { opened_at?: number }).opened_at = Date.now() / 1000;
      commit();
      const tokens = tokenize(l.text, l.lang);
      const terms: Record<string, Term> = {};
      for (const [k, t] of Object.entries(db.terms)) {
        const [lang, term] = k.split("|");
        if (lang === l.lang) terms[term] = t;
      }
      return { lesson: l, tokens, terms };
    },
    async lessonCreate(n: NewLesson) {
      const id = db.nextId++;
      const l: Lesson & { created_at: number } = {
        id,
        lang: n.lang,
        title: n.title,
        collection: n.collection ?? "",
        kind: n.kind ?? "text",
        source: n.source ?? "",
        text: n.text,
        media_path: n.media_path ?? null,
        timings: n.timings ?? null,
        video_path: n.video_path ?? null,
        hue: Math.floor(Math.random() * 360),
        word_count: tokenize(n.text, n.lang).filter((t) => t.w).length,
        page: 0,
        completed: false,
        position: 0,
        anchor: 0,
        duration: 0,
        cover_path: null,
        timing_v: 0,
        created_at: Date.now() / 1000,
      };
      db.lessons.push(l);
      commit();
      return id;
    },
    async lessonUpdate(id, patch) {
      const l = db.lessons.find((x) => x.id === id);
      if (l) Object.assign(l, patch);
      commit();
    },
    async lessonDelete(id) {
      db.lessons = db.lessons.filter((l) => l.id !== id);
      for (const c of chats()) if (c.lesson_id === id) c.lesson_id = null;
      for (const p of playlists()) {
        p.lessons = p.lessons.filter((x) => x !== id);
        if (p.current === id) p.current = null;
      }
      commit();
    },
    async playlistsList(lang) {
      return playlists()
        .filter((p) => p.lang === lang)
        .map((p) => ({ ...p, lessons: [...p.lessons], current: p.current && p.lessons.includes(p.current) ? p.current : null }))
        .sort((a, b) => b.created_at - a.created_at || b.id - a.id);
    },
    async playlistCreate(lang, name, lessons) {
      const id = Math.max(0, ...playlists().map((p) => p.id)) + 1;
      playlists().push({ id, lang, name: name.trim().replace(/\s+/g, " ").slice(0, 120) || t("Nouvelle playlist", "New playlist"), lessons: fill(lang, lessons), current: null, created_at: Date.now() / 1000 });
      commit();
      return id;
    },
    async playlistUpdate(id, patch) {
      const p = playlists().find((x) => x.id === id);
      if (!p) return;
      if (patch.name !== undefined) p.name = patch.name.trim().replace(/\s+/g, " ").slice(0, 120) || t("Nouvelle playlist", "New playlist");
      if (patch.lessons) p.lessons = fill(p.lang, patch.lessons);
      if (patch.current !== undefined) p.current = patch.current > 0 ? patch.current : null;
      commit();
    },
    async playlistDelete(id) {
      db.playlists = playlists().filter((p) => p.id !== id);
      commit();
    },
    async lessonSetCover(id, data) {
      const l = db.lessons.find((x) => x.id === id);
      if (!l) throw t("Leçon introuvable", "Lesson not found");
      // dans le navigateur, l'image est gardée telle quelle (adresse data:)
      let url: string | null = null;
      if (data) {
        let bin = "";
        for (let i = 0; i < data.length; i += 0x8000) bin += String.fromCharCode(...data.subarray(i, i + 0x8000));
        url = `data:image/jpeg;base64,${btoa(bin)}`;
      }
      l.cover_path = url;
      commit();
      return url;
    },
    async termSet(u: TermUpdate) {
      const term = normalize(u.term, u.lang);
      const k = `${u.lang}|${term}`;
      const prev = db.terms[k];
      if (u.status === 0) {
        delete db.terms[k];
      } else {
        db.terms[k] = {
          term,
          status: u.status as Term["status"],
          translation: u.translation ?? prev?.translation ?? "",
          note: u.note ?? prev?.note ?? "",
          lemma: u.lemma ?? prev?.lemma ?? "",
          context: prev?.context || u.context || "",
          updated_at: Date.now() / 1000,
        };
        if (!prev && u.status <= 3) bump(u.lang, "lingqs", 1);
        if (prev?.status !== 4 && u.status === 4) bump(u.lang, "known_added", 1);
      }
      commit();
    },
    async termsMarkKnown(lang, keys, wordsRead) {
      let added = 0;
      for (const raw of new Set(keys)) {
        const k = `${lang}|${normalize(raw, lang)}`;
        if (!db.terms[k]) {
          db.terms[k] = { term: normalize(raw, lang), status: 4, translation: "", note: "", lemma: "", context: "", updated_at: Date.now() / 1000 };
          added++;
        }
      }
      bump(lang, "known_added", added);
      bump(lang, "words_read", wordsRead);
      commit();
      return added;
    },
    async termsList(q) {
      let items = Object.entries(db.terms)
        .filter(([k]) => k.startsWith(q.lang + "|"))
        .map(([, t]) => t);
      if (q.filter === "learning") items = items.filter((t) => t.status <= 3);
      if (q.filter === "known") items = items.filter((t) => t.status === 4);
      if (q.filter === "ignored") items = items.filter((t) => t.status === 5);
      if (q.filter === "phrases") items = items.filter((t) => t.term.includes(" "));
      if (q.filter === "all") items = items.filter((t) => t.status !== 5);
      if (q.search) {
        const s = normalize(q.search);
        items = items.filter((t) => t.term.includes(s) || t.translation.toLowerCase().includes(s));
      }
      items.sort((a, b) => (a.status <= 3 ? 0 : 1) - (b.status <= 3 ? 0 : 1) || b.updated_at - a.updated_at);
      return { items: items.slice(q.offset, q.offset + q.limit), total: items.length };
    },
    async stats(lang): Promise<Stats> {
      const terms = Object.entries(db.terms).filter(([k]) => k.startsWith(lang + "|")).map(([, t]) => t);
      const goal = goalMin();
      const now = new Date();
      const td = new Date(now.getFullYear(), now.getMonth(), now.getDate());
      const K = { td: ymd(td) } as Record<string, string>;
      const rows: DayStat[] = Object.values(db.activity)
        .filter((a) => a.lang === lang)
        .map(({ lang: _l, ...a }) => {
          const d = { ...emptyDay(a.day), ...a };
          if (d.day === K.td && d.learn_secs >= goal * 60) d.goal_met = true;
          return d;
        })
        .sort((a, b) => a.day.localeCompare(b.day));
      const active = (d: DayStat) => d.learn_secs > 0 || d.words_read > 0 || d.listen_secs > 0 || d.lingqs > 0 || d.known_added > 0;
      const span = (start: Date): Span => ({ start: ymd(start), words_read: 0, known_added: 0, lingqs: 0, listen_secs: 0, learn_secs: 0, active_days: 0, goal_days: 0 });
      const add = (s: Span, d: DayStat) => {
        s.words_read += d.words_read;
        s.known_added += d.known_added;
        s.lingqs += d.lingqs;
        s.listen_secs += d.listen_secs;
        s.learn_secs += d.learn_secs;
        s.active_days += active(d) ? 1 : 0;
        s.goal_days += d.goal_met ? 1 : 0;
      };
      // bornes des périodes (semaine depuis lundi, mois depuis le 1er)
      const yesterday = addDays(td, -1);
      const week = addDays(td, -((td.getDay() + 6) % 7));
      const lastWeek = addDays(week, -7);
      const month = new Date(td.getFullYear(), td.getMonth(), 1);
      const lastMonth = new Date(td.getFullYear(), td.getMonth() - 1, 1);
      const periods = { today: span(td), yesterday: span(yesterday), week: span(week), last_week: span(lastWeek), month: span(month), last_month: span(lastMonth), total: span(td) };
      const first = rows.find(active)?.day ?? null;
      if (first) periods.total.start = first;
      const weeksFrom = addDays(week, -77);
      const weeks = Array.from({ length: 12 }, (_, i) => span(addDays(weeksFrom, 7 * i)));
      const minFrom = new Date(month.getFullYear(), month.getMonth() - 11, 1);
      const maxFrom = new Date(month.getFullYear(), month.getMonth() - 35, 1);
      let monthsFrom = first ? new Date(parseDay(first).getFullYear(), parseDay(first).getMonth(), 1) : minFrom;
      if (monthsFrom > minFrom) monthsFrom = minFrom;
      if (monthsFrom < maxFrom) monthsFrom = maxFrom;
      const months: Span[] = [];
      for (let m = monthsFrom; m <= month; m = new Date(m.getFullYear(), m.getMonth() + 1, 1)) months.push(span(m));
      Object.assign(K, { y: ymd(yesterday), w: ymd(week), lw: ymd(lastWeek), m: ymd(month), lm: ymd(lastMonth), wf: ymd(weeksFrom), mf: ymd(monthsFrom) });
      const records = { words_read: 0, words_day: "", learn_secs: 0, learn_day: "" };
      for (const d of rows) {
        add(periods.total, d);
        if (d.day === K.td) add(periods.today, d);
        if (d.day === K.y) add(periods.yesterday, d);
        if (d.day >= K.w && d.day <= K.td) add(periods.week, d);
        if (d.day >= K.lw && d.day < K.w) add(periods.last_week, d);
        if (d.day >= K.m && d.day <= K.td) add(periods.month, d);
        if (d.day >= K.lm && d.day < K.m) add(periods.last_month, d);
        if (d.day >= K.wf && d.day <= K.td) {
          const w = weeks[Math.floor(Math.round((parseDay(d.day).getTime() - weeksFrom.getTime()) / 86400000) / 7)];
          if (w) add(w, d);
        }
        if (d.day >= K.mf && d.day <= K.td) {
          const dd = parseDay(d.day);
          const m = months[(dd.getFullYear() - monthsFrom.getFullYear()) * 12 + dd.getMonth() - monthsFrom.getMonth()];
          if (m) add(m, d);
        }
        if (d.words_read > records.words_read) Object.assign(records, { words_read: d.words_read, words_day: d.day });
        if (d.learn_secs > records.learn_secs) Object.assign(records, { learn_secs: d.learn_secs, learn_day: d.day });
      }
      const byDay = new Map(rows.map((r) => [r.day, r]));
      const days: DayStat[] = [];
      for (let d = addDays(week, -175); d <= td; d = addDays(d, 1)) days.push(byDay.get(ymd(d)) ?? emptyDay(ymd(d)));
      return {
        known: terms.filter((t) => t.status === 4 && !t.term.includes(" ")).length,
        learning: terms.filter((t) => t.status <= 3).length,
        phrases: terms.filter((t) => t.term.includes(" ")).length,
        lessons: db.lessons.filter((l) => l.lang === lang).length,
        days,
        weeks,
        months,
        periods,
        streak: streakOf(rows, goal),
        records,
        first_day: first,
      };
    },
    async activityAdd(lang, wordsRead, listenSecs, learnSecs = 0) {
      bump(lang, "words_read", Math.max(0, wordsRead));
      bump(lang, "listen_secs", Math.max(0, listenSecs));
      bump(lang, "learn_secs", Math.max(0, learnSecs));
      let reached: GoalReached | null = null;
      const a = db.activity[`${today()}|${lang}`];
      if (learnSecs > 0 && a && !a.goal_met && a.learn_secs >= goalMin() * 60) {
        a.goal_met = true;
        const rows = Object.values(db.activity).filter((x) => x.lang === lang);
        reached = { streak: streakOf(rows, goalMin()).current, goal_min: goalMin() };
      }
      commit();
      return reached;
    },
    async exportVocab() {},
    async dictLookup(lang, word) {
      await sleep(30);
      const st = dictState(lang);
      if (st.exists && !st.ready) {
        dictFetch(lang);
        return { entries: [], pending: true };
      }
      const d = MOCK_DICT[normalize(word)];
      if (!d) return { entries: [] };
      return { entries: [{ word: normalize(word), pos: t(d[0], d[3]), ipa: d[1], glosses: [t(d[2], d[4])] }] };
    },
    async dictStatus(lang) {
      const st = dictState(lang);
      if (st.exists && !st.ready) {
        dictFetch(lang);
        return dictState(lang);
      }
      return st;
    },
    async dictListen(onChange) {
      dictListeners.add(onChange);
      return () => dictListeners.delete(onChange);
    },
    async aiWord(_lang, word, _sentence, onPiece) {
      if (!online("words") && !db.installed.some((m) => m.startsWith("qwen"))) throw t("NO_MODEL:Aucun modèle de traduction n'est installé. Ouvrez Réglages › IA.", "NO_MODEL:No translation model is installed. Open Settings › AI.");
      await sleep(250);
      const d = MOCK_DICT[normalize(word)];
      const tr = d ? t(d[2], d[4]).toLowerCase() : t(`« ${word} » (aperçu)`, `“${word}” (preview)`);
      const note = normalize(word).endsWith("ed") ? t("Prétérit, action terminée dans le passé.", "Past tense, a finished action.") : "";
      await fakeStream(t(`Sens : ${tr}\nNote : ${note || "-"}`, `Meaning: ${tr}\nNote: ${note || "-"}`), onPiece);
      return { translation: tr, note, cached: false };
    },
    async aiSentence(_lang, sentence, onPiece) {
      online("words");
      await sleep(200);
      return fakeStream(t(`[Traduction simulée] ${sentence}`, `[Simulated translation] ${sentence}`), onPiece);
    },
    async aiSimplify(_lang, text, _level, onPiece) {
      online("chat");
      await sleep(400);
      const simple = text.split(/(?<=[.!?])\s+/).slice(0, 8).join(" ");
      return fakeStream(simple, onPiece);
    },
    async aiWarmup() {
      return db.installed.some((m) => m.startsWith("qwen"));
    },
    async chatsList(lang) {
      return chats()
        .filter((c) => c.lang === lang)
        .sort((a, b) => b.updated_at - a.updated_at || b.id - a.id)
        .map(chatSummary);
    },
    async chatOpen(id) {
      const c = chats().find((x) => x.id === id);
      if (!c) throw t("Cette conversation n'existe plus.", "This conversation no longer exists.");
      return { chat: chatSummary(c), messages: c.messages };
    },
    async chatCreate(lang, lesson) {
      const id = Math.max(0, ...chats().map((c) => c.id)) + 1;
      const now = Date.now() / 1000;
      const lesson_id = lesson && db.lessons.some((l) => l.id === lesson && l.lang === lang) ? lesson : null;
      const c = { id, lang, title: "", lesson_id, created_at: now, updated_at: now, messages: [] };
      chats().push(c);
      commit();
      return chatSummary(c);
    },
    async chatUpdate(id, patch) {
      const c = chats().find((x) => x.id === id);
      if (!c) throw t("Cette conversation n'existe plus.", "This conversation no longer exists.");
      if (patch.title !== undefined) c.title = patch.title.trim().replace(/\s+/g, " ").slice(0, 120);
      if (patch.lesson !== undefined) c.lesson_id = db.lessons.some((l) => l.id === patch.lesson && l.lang === c.lang) ? patch.lesson : null;
      commit();
      return chatSummary(c);
    },
    async chatDelete(id) {
      db.chats = chats().filter((c) => c.id !== id);
      commit();
    },
    async chatSend(id, text, options, onEvent) {
      if (!online("chat") && !db.installed.some((m) => m.startsWith("qwen"))) throw t("NO_MODEL:Aucun modèle de traduction n'est installé. Ouvrez Réglages › IA.", "NO_MODEL:No translation model is installed. Open Settings › AI.");
      const c = chats().find((x) => x.id === id);
      if (!c) throw t("Cette conversation n'existe plus.", "This conversation no longer exists.");
      chatStops.set(id, false);
      const lesson = db.lessons.find((l) => l.id === c.lesson_id);
      const stream = async (full: string, type: "thought" | "answer", pace: number) => {
        let out = "";
        for (const part of full.match(/\S+\s*/g) ?? []) {
          if (chatStops.get(id)) break;
          await sleep(pace);
          out += part;
          onEvent({ type, text: part });
        }
        return out;
      };
      await sleep(350);
      let thought = "";
      let thoughtSecs = 0;
      if (options.think) {
        const t0 = Date.now();
        const len = options.effort === "low" ? 1 : options.effort === "high" ? 4 : 2;
        thought = await stream(
          Array.from({ length: len }, () => `The learner asks: "${text.slice(0, 60)}". I should answer in French, give the base form, the grammar, then one or two examples with their translation.`).join(" "),
          "thought",
          28,
        );
        thoughtSecs = (Date.now() - t0) / 1000;
      }
      const answer = await stream(
        t(
          `Voici ce que l'on peut dire${lesson ? ` à partir de « ${lesson.title} »` : ""} :\n\n- **Sens** : réponse simulée à « ${text.slice(0, 80)} ».\n- **Grammaire** : dans l'application, Qwen répond ici, calculé sur votre Mac.\n\n| Personne | Forme |\n| --- | --- |\n| io | salivo |\n| tu | salivi |\n\n*Exemple* : *Marta saliva le scale.* (Marta montait l'escalier.)`,
          `Here is what we can say${lesson ? ` based on “${lesson.title}”` : ""}:\n\n- **Meaning**: simulated answer to “${text.slice(0, 80)}”.\n- **Grammar**: in the app, Qwen answers here, computed on your Mac.\n\n| Person | Form |\n| --- | --- |\n| io | salivo |\n| tu | salivi |\n\n*Example*: *Marta saliva le scale.* (Marta was climbing the stairs.)`,
        ),
        "answer",
        34,
      );
      const stopped = !!chatStops.get(id);
      chatStops.delete(id);
      if (!answer.trim()) return { chat: chatSummary(c), user: null, assistant: null, stopped };
      const now = Date.now() / 1000;
      const nextId = Math.max(0, ...chats().flatMap((x) => x.messages.map((m) => m.id))) + 1;
      const user: ChatMessage = { id: nextId, role: "user", content: text.trim(), thought: "", thought_secs: 0, created_at: now };
      const assistant: ChatMessage = { id: nextId + 1, role: "assistant", content: answer.trim(), thought: thought.trim(), thought_secs: thoughtSecs, created_at: now };
      c.messages.push(user, assistant);
      if (!c.title) c.title = text.trim().split("\n")[0].slice(0, 60);
      c.updated_at = now;
      commit();
      return { chat: chatSummary(c), user, assistant, stopped };
    },
    async chatStop(id) {
      if (chatStops.has(id)) chatStops.set(id, true);
    },
    async modelsList() {
      const llm = db.settings.llm_model ?? "qwen3.5-2b";
      const asr = db.settings.asr_model ?? "whisper-turbo";
      return models.map((m) => ({ ...m, installed: db.installed.includes(m.id), active: m.id === llm || m.id === asr, downloading: false, partial: 0 }));
    },
    async modelDownload(id, onEvent) {
      const m = models.find((x) => x.id === id)!;
      for (let i = 1; i <= 40; i++) {
        await sleep(70);
        onEvent({ type: "progress", received: (m.size * i) / 40, total: m.size, speed: 42e6 });
      }
      db.installed.push(id);
      commit();
      onEvent({ type: "done" });
    },
    async modelCancel() {},
    async modelDelete(id) {
      db.installed = db.installed.filter((x) => x !== id);
      commit();
    },
    async lessonVoice(_id, onEvent) {
      for (let i = 1; i <= 10; i++) {
        await sleep(120);
        onEvent({ type: "progress", value: i * 10 });
      }
      throw t("NO_VOICE: la voix naturelle fonctionne dans l'application Mac", "NO_VOICE: the natural voice works in the Mac app");
    },
    async lessonVoiceCancel() {},
    async ttsSay() {
      // l'aperçu navigateur n'a pas le moteur : la voix du système prend le relais
      throw t("NO_VOICE: la voix naturelle fonctionne dans l'application Mac", "NO_VOICE: the natural voice works in the Mac app");
    },
    async readFile() {
      throw t("La lecture de fichiers fonctionne dans l'application Mac.", "Reading files works in the Mac app.");
    },
    async importMedia() {
      throw t("La transcription fonctionne dans l'application Mac.", "Transcription works in the Mac app.");
    },
    async linkProbe(url, onEvent) {
      onEvent({ type: "stage", stage: "probe" });
      await sleep(900);
      const raw = url.trim();
      if (!raw.includes(".")) throw t("Cette adresse n'est pas valide.", "This address isn't valid.");
      const page = /^https?:\/\//.test(raw) ? raw : `https://${raw}`;
      const u = page.toLowerCase();
      const empty: LinkInfo = { url: page, title: "", site: "", image: "", html: "", media: [], list: false, via: "", note: "" };
      const day = (i: number) => new Date(Date.now() - i * 7 * 86400e3).toISOString().slice(0, 10);
      const episodes = (show: string): LinkMedia[] =>
        ["The lighthouse keeper", "A night at sea", "Back to the village", "The storm", "Morning fishermen", "Last light"].map((title, i) => ({
          url: `https://cdn.example/ep${i}.mp3`,
          title,
          duration: 1260 + i * 137,
          video: false,
          direct: true,
          image: "",
          date: day(i),
          page,
          collection: show,
        }));
      if (u.includes("spotify") && /track|album|playlist/.test(u)) {
        const songs = ["Clair de lune", "La mer", "Le phare"].map((title) => ({ url: `ytsearch1:${title}`, title: `${title} · Martha`, duration: 214, video: false, direct: false, image: "", date: "", page, collection: "Lumière" }));
        return u.includes("track") ? { ...empty, title: "Clair de lune", site: "Martha", media: songs.slice(0, 1), via: "youtube" } : { ...empty, title: "Lumière", site: "Martha", media: songs, list: true, via: "youtube" };
      }
      if (u.includes("spotify")) return { ...empty, title: "The lighthouse keeper", site: "Slow Stories", media: episodes("Slow Stories").slice(0, 1), via: "rss" };
      if (u.includes("youtu"))
        return { ...empty, title: "A walk by the lighthouse", site: "Easy Stories", media: [{ url: page, title: "A walk by the lighthouse", duration: 754, video: true, direct: false, image: "", date: day(1), page, collection: "" }] };
      if (/podcast|rss|feed|apple/.test(u)) return { ...empty, title: "Slow Stories", site: "Slow Stories", media: episodes("Slow Stories"), list: true };
      const body = STARTERS_MOCK.split("\n\n").map((p) => `<p>${p}</p>`).join("");
      return {
        ...empty,
        title: "The lighthouse",
        site: "Stories",
        html: `<html><head><title>The lighthouse</title></head><body><article><h1>The lighthouse</h1>${body}${body}</article></body></html>`,
        media: [{ url: "https://cdn.example/lighthouse.mp3", title: "The lighthouse", duration: 0, video: false, direct: true, image: "", date: "", page, collection: "" }],
      };
    },
    async importLink(_lang, _item, _text, onEvent) {
      onEvent({ type: "stage", stage: "download" });
      for (let i = 1; i <= 10; i++) {
        await sleep(80);
        onEvent({ type: "progress", value: i * 10 });
      }
      throw t("La transcription fonctionne dans l'application Mac.", "Transcription works in the Mac app.");
    },
    async lessonFetchVideo() {
      throw t("Le téléchargement de vidéos fonctionne dans l'application Mac.", "Downloading videos works in the Mac app.");
    },
    async lessonResync(id, onEvent) {
      const l = db.lessons.find((x) => x.id === id);
      if (!l?.media_path) throw t("Cette leçon n'a pas d'audio à recaler.", "This lesson has no audio to realign.");
      onEvent({ type: "stage", stage: "transcribe" });
      for (let i = 1; i <= 20; i++) {
        await sleep(90);
        onEvent({ type: "progress", value: i * 5 });
      }
      l.timing_v = 2;
      commit();
      return l.timings ?? "[]";
    },
    async lingqScan(key) {
      await sleep(900);
      if (key.trim().length < 10) throw t("LingQ refuse cette clé API. Vérifiez-la sur lingq.com puis collez-la à nouveau.", "LingQ refuses this API key. Check it on lingq.com, then paste it again.");
      return [
        { lang: "en", known_words: 4210, lingqs: 812, courses: [{ id: 1, title: "Mini Stories", lessons: 60 }, { id: 2, title: "Mes imports", lessons: 14 }], lessons: 74 },
        { lang: "it", known_words: 930, lingqs: 205, courses: [{ id: 3, title: "Italiano per principianti", lessons: 24 }], lessons: 24 },
      ];
    },
    async lingqImport(_key, plan, onEvent) {
      lingqCancelled = false;
      const report = { words: 0, lessons: 0, skipped: 0, failed: 0, audio_failed: 0, cancelled: false };
      const sample: [string, number, string][] = [["lighthouse", 4, "phare"], ["narrow", 2, "étroit"], ["endless", 1, "sans fin"], ["damp", 3, "humide"]];
      for (const lang of plan.langs) {
        if (plan.vocab) {
          for (const stage of ["known", "ignored", "cards"] as const) {
            onEvent({ type: "stage", lang, stage });
            for (let i = 1; i <= 10 && !lingqCancelled; i++) {
              await sleep(90);
              onEvent({ type: "progress", done: i * 100, total: 1000 });
            }
          }
          for (const [term, status, translation] of sample) {
            if (!db.terms[`${lang}|${term}`]) {
              db.terms[`${lang}|${term}`] = { term, status: status as Term["status"], translation, note: "", lemma: "", context: "", updated_at: Date.now() / 1000 };
              report.words++;
            }
          }
        }
        if (plan.lessons && !lingqCancelled) {
          onEvent({ type: "stage", lang, stage: "lessons" });
          const titles = ["Une lettre dans le phare", "Le gardien", "La réponse"];
          for (let i = 0; i < titles.length && !lingqCancelled; i++) {
            onEvent({ type: "progress", done: i, total: titles.length });
            await sleep(400);
            await mock.lessonCreate({ lang, title: titles[i], text: STARTERS_MOCK, collection: "Mini Stories (LingQ)" });
            report.lessons++;
            onEvent({ type: "lesson", title: titles[i], course: "Mini Stories (LingQ)" });
          }
          onEvent({ type: "progress", done: titles.length, total: titles.length });
        }
      }
      report.cancelled = lingqCancelled;
      commit();
      return report;
    },
    async lingqCancel() {
      lingqCancelled = true;
    },
    async backupStatus() {
      return backupStatus();
    },
    async backupRun() {
      await sleep(900);
      db.backup = { last_at: Math.floor(Date.now() / 1000), size: 2.4e6 + db.lessons.length * 4e4 };
      commit();
      const s = backupStatus();
      backupListeners.forEach((f) => f(s));
      return s;
    },
    async backupList() {
      await sleep(700);
      const now = Math.floor(Date.now() / 1000);
      const out: BackupInfo[] = [];
      if (db.backup?.last_at && db.settings.backup_on === "1") {
        const me = db.settings;
        out.push({
          key: "mock-ce-mac",
          device_name: t("Ce navigateur", "This browser"),
          mine: true,
          this_device: true,
          saved_at: db.backup.last_at,
          app_version: "0.1.0",
          size: db.backup.size,
          media_size: 186e6,
          counts: backupCounts(),
          versions: [],
          newer: false,
          name: me.user_name ?? "",
          avatar: me.user_avatar ?? "",
          photo: (me.user_avatar ?? "").startsWith("photo:") ? (me.user_photo ?? "") : "",
        });
      }
      out.push({
        key: "mock-imac",
        device_name: "iMac du salon",
        mine: false,
        this_device: false,
        saved_at: now - 2 * 86400 - 5 * 3600,
        app_version: "0.1.0",
        size: 3.2e6,
        media_size: 412e6,
        counts: OTHER_MAC,
        versions: [3, 4, 6].map((n, i) => ({ day: dayAgo(n), saved_at: now - n * 86400, known: OTHER_MAC.known - 30 * (i + 1), lessons: OTHER_MAC.lessons - i })),
        newer: false,
        name: "Marta",
        avatar: "dawn:16:4242",
        photo: "",
      });
      return out.sort((a, b) => b.saved_at - a.saved_at);
    },
    async backupRestore(key, _day, onEvent) {
      onEvent({ type: "stage", stage: "download" });
      await sleep(700);
      onEvent({ type: "stage", stage: "media" });
      for (let i = 1; i <= 12; i++) {
        await sleep(110);
        onEvent({ type: "progress", value: i / 12 });
      }
      onEvent({ type: "stage", stage: "apply" });
      await sleep(400);
      if (key !== "mock-imac") return { counts: backupCounts(), missing_media: 0 };
      // la progression de l'autre Mac : quelques leçons et des mots, réglages de ce navigateur conservés
      db.settings = { ...db.settings, onboarded: "1", langs: "it,en", lang: "it", backup_on: db.settings.backup_on || "1", user_name: "Marta", user_avatar: "dawn:16:4242", user_photo: "", user_why: t("Lire les romans de Calvino dans le texte", "Read Calvino's novels in the original") };
      const lessons: [LangCode, string, string][] = [
        ["it", "Il faro", "Ogni mattina, Marta saliva le scale strette del vecchio faro. Dall'alto, il mare sembrava infinito e calmo."],
        ["it", "La lettera", "Un giorno trovò una lettera nascosta tra due pietre. La carta era umida, ma le parole si leggevano ancora."],
        ["en", "The lighthouse", STARTERS_MOCK],
      ];
      for (const [lang, title, text] of lessons) {
        if (!db.lessons.some((l) => l.lang === lang && l.title === title)) await mock.lessonCreate({ lang, title, text, collection: "Storie" });
      }
      for (const [term, status, translation] of [["faro", 4, "phare"], ["mare", 4, "mer"], ["lettera", 2, "lettre"], ["nascosta", 1, "cachée"]] as const) {
        db.terms[`it|${term}`] ??= { term, status, translation, note: "", lemma: "", context: "", updated_at: Date.now() / 1000 };
      }
      commit();
      return { counts: OTHER_MAC, missing_media: 0 };
    },
    async backupListen(onStatus) {
      backupListeners.add(onStatus);
      return () => backupListeners.delete(onStatus);
    },

    async backupPlaces() {
      await sleep(120);
      return mockPlaces();
    },

    async backupPlaceDir(path) {
      await sleep(200);
      // Google Drive : rien à la racine du compte, tout dans « Mon Drive »
      return path.includes("GoogleDrive-") ? `${path}/${t("Mon Drive", "My Drive")}` : path;
    },
    async discoverList(lang) {
      const d = discover.get(lang);
      const lessons = db.lessons.filter((l) => l.lang === lang);
      const items = (d?.items ?? [])
        .filter((it) => !d?.hidden.has(it.id))
        .map((it) => {
          // une leçon supprimée depuis ne compte plus
          const marked = lessons.find((l) => l.id === d?.marks.get(it.id));
          const same = lessons.find((l) => l.source && (l.source === it.page || l.source === it.url));
          return { ...it, lesson_id: (marked ?? same)?.id ?? null };
        });
      return { items, refreshed_at: d?.at ?? 0, refreshing: false, sources: new Set(items.map((i) => i.source)).size || 6 };
    },
    async discoverRefresh(lang, force, onEvent) {
      const was = discover.get(lang);
      // une lecture d'arrière-plan sans rien à relire : rien ne bouge
      if (was && !force && Date.now() / 1000 - was.at < 3 * 3600) return { added: 0, sources: 0, failed: [], skipped: true };
      if (was && force && Date.now() / 1000 - was.at < 5 * 60) return { added: 0, sources: 6, failed: [], skipped: true };
      onEvent({ type: "stage", stage: "discover" });
      for (let i = 1; i <= 10; i++) {
        await sleep(250);
        onEvent({ type: "progress", value: i * 10 });
      }
      const now = Math.round(Date.now() / 1000);
      const old = discover.get(lang);
      const items = discoverItems(lang as LangCode, old ? old.at : now);
      // à chaque nouvelle lecture, un élément de plus apparaît (« Nouveau »)
      if (old) {
        const fresh = { ...items[0], id: `fresh-${now}`, title: `${items[0].title} · ${t("nouvel épisode", "new episode")}`, published: now, fetched_at: now };
        items.unshift(...old.items.filter((it) => it.id.startsWith("fresh-")), fresh);
      }
      discover.set(lang, { items, at: now, hidden: old?.hidden ?? new Set(), marks: old?.marks ?? new Map() });
      discoverListeners.forEach((f) => f(lang));
      return { added: old ? 1 : items.length, sources: 6, failed: [], skipped: false };
    },
    async discoverMark(id, lesson) {
      for (const d of discover.values()) if (d.items.some((it) => it.id === id)) d.marks.set(id, lesson);
    },
    async discoverHide(id) {
      for (const d of discover.values()) d.hidden.add(id);
    },
    async discoverListen(onChange) {
      discoverListeners.add(onChange);
      return () => discoverListeners.delete(onChange);
    },
    async searchOnline(lang, platform, query, page, _filter, onEvent) {
      onEvent({ type: "stage", stage: "search" });
      await sleep(platform === "youtube" ? 1100 : platform === "music" ? 1500 : 600);
      if (!query.trim()) return { hits: [], more: false };
      return { hits: searchHits(lang as LangCode, platform, query, page), more: page < 1 && platform !== "podcast" };
    },
    async mediaStream(_url, audio) {
      await sleep(700);
      return { video: audio ? "" : SAMPLE_VIDEO, audio: audio ? SAMPLE_AUDIO : "", width: 1280, height: 720, language: "", title: "", description: "", author: "", duration: 0, published: 0, count: 0 };
    },
    async songFind(artist, title) {
      await sleep(500);
      return `https://www.youtube.com/watch?v=mock-${encodeURIComponent(`${artist}-${title}`)}`;
    },
    async lyricsFind() {
      await sleep(400);
      return mockLyrics(useLangFromDb());
    },
    async importSong(lang, song, onEvent) {
      for (const stage of ["lyrics", "download", "transcribe"]) {
        onEvent({ type: "stage", stage });
        for (let i = 1; i <= 5; i++) {
          await sleep(120);
          onEvent({ type: "progress", value: i * 20 });
        }
      }
      const lyrics = song.lyrics ?? mockLyrics(lang);
      const text = lyrics.synced
        .split("\n")
        .map((l) => l.replace(/^\[[^\]]*\]\s*/, "").trim())
        .join("\n")
        .replace(/\n{2,}/g, "\n\n")
        .trim();
      return mock.lessonCreate({ lang, title: song.title, text, collection: song.artist, kind: "text", source: song.page });
    },
    async geminiCheck(key) {
      await sleep(700);
      if (key.trim().length < 20)
        throw t(
          "Google refuse cette clé Gemini. Vérifiez-la dans Google AI Studio, puis collez-la à nouveau.",
          "Google refuses this Gemini key. Check it in Google AI Studio, then paste it again.",
        );
      return { text: ["gemini-3.8-flash", "gemini-3.5-flash"], tts: ["gemini-3.8-flash-tts", "gemini-3.8-flash-lite-tts"] };
    },
    async onlineCheck(provider, key, url, model) {
      await sleep(800);
      if (provider === "custom") {
        if (!/^https?:\/\/[^/]+/.test(url ?? "")) throw t("Cette adresse n'est pas valable. Exemple : http://localhost:11434/v1 pour Ollama.", "This address isn't valid. Example: http://localhost:11434/v1 for Ollama.");
        const models = ["llama3.2", "qwen3:8b", "gemma3:12b"];
        return { models, model: model && models.includes(model) ? model : models[0], ms: 1350 };
      }
      const name: Record<string, string> = { deepseek: "DeepSeek", gemini: "Gemini", mistral: "Mistral", openai: "OpenAI", anthropic: "Claude", openrouter: "OpenRouter" };
      if (key.trim().length < 12) throw t(`${name[provider] ?? provider} refuse cette clé. Vérifiez-la, puis collez-la à nouveau (Réglages › IA).`, `${name[provider] ?? provider} refuses this key. Check it, then paste it again (Settings › AI).`);
      const lists: Record<string, string[]> = {
        deepseek: ["deepseek-flash", "deepseek-v4-pro"],
        gemini: ["gemini-3.8-flash", "gemini-3.8-flash-lite", "gemini-3.8-pro"],
        mistral: ["mistral-large-latest", "mistral-medium-latest", "mistral-small-latest"],
        openai: ["gpt-5.4", "gpt-5.4-mini", "gpt-5.4-nano"],
        anthropic: ["claude-haiku-4-5", "claude-sonnet-4-6", "claude-opus-4-8"],
        openrouter: ["anthropic/claude-haiku-4.5", "deepseek/deepseek-chat", "google/gemini-2.5-flash", "meta-llama/llama-4-maverick"],
      };
      const picks: Record<string, string> = { deepseek: "deepseek-flash", gemini: "gemini-3.8-flash", mistral: "mistral-small-latest", openai: "gpt-5.4-mini", anthropic: "claude-haiku-4-5", openrouter: "deepseek/deepseek-chat" };
      const models = lists[provider] ?? [];
      return { models, model: model && models.includes(model) ? model : picks[provider] ?? models[0], ms: 640 };
    },
    async podcastCreate(lang, request, onEvent) {
      if (!db.settings.gemini_key) throw t("Ajoutez d'abord votre clé Gemini (Réglages › Podcasts).", "First add your Gemini key (Settings › Podcasts).");
      onEvent({ type: "stage", stage: "script" });
      await sleep(1400);
      for (const stage of ["studio", "timing"]) {
        onEvent({ type: "stage", stage });
        for (let i = 1; i <= 10; i++) {
          await sleep(stage === "studio" ? 260 : 90);
          onEvent({ type: "progress", value: i * 10 });
        }
      }
      onEvent({ type: "stage", stage: "lesson" });
      // pas de son dans le navigateur : les phrases du texte de départ, réparties en répliques
      const sentences = STARTERS[lang].text.split(/(?<=[.!?。])\s+/).filter((s) => s.trim().length > 3);
      const turns = request.format === "story" ? 3 : 2;
      const paras: string[] = [];
      for (let i = 0; i < sentences.length; i += turns) paras.push(sentences.slice(i, i + turns).join(" "));
      return mock.lessonCreate({ lang, title: request.topic.trim(), text: paras.join("\n\n"), collection: t("Mes podcasts", "My podcasts"), kind: "text", source: "" });
    },
    async levelEstimate(lang) {
      const forms = Object.entries(db.terms).filter(([k, x]) => k.startsWith(`${lang}|`) && x.status === 4 && !x.term.includes(" ")).length;
      return { ...roughEstimate(lang as LangCode, forms), exact: true };
    },
    async textStats(lang, text) {
      const keys = tokenize(text, lang as LangCode).filter((x) => x.w).map((x) => x.k);
      const distinct = new Set(keys);
      const fresh = new Set(keys.filter((k) => !db.terms[`${lang}|${k}`]));
      const rec = keys.filter((k) => db.terms[`${lang}|${k}`]).length;
      return { words: keys.length, unique: distinct.size, new_words: fresh.size, new_pct: newShare(fresh.size, distinct.size), known_pct: keys.length ? Math.floor((rec * 100) / keys.length) : 100 };
    },
    mediaUrl(path) {
      return path;
    },
  };
  /** langue étudiée dans l'aperçu (paroles simulées) */
  const useLangFromDb = () => (db.settings.lang || "en") as LangCode;
  return mock;
}

// Pour réinitialiser l'aperçu : localStorage.removeItem("lumen-mock-db")
export const MOCK_LANG_DEFAULT: LangCode = "en";
