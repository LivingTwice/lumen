// Backend simulé, utilisé uniquement quand l'interface tourne dans un
// navigateur (développement et tests visuels). Dans l'application, tout passe
// par le backend Rust.
import type { Api, TermUpdate } from "./api";
import { tokenize, normalize } from "./tokenize";
import type {
  DayStat,
  LangCode,
  Lesson,
  LessonSummary,
  ModelRow,
  NewLesson,
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

const today = () => new Date().toISOString().slice(0, 10);
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

let lingqCancelled = false;
const STARTERS_MOCK =
  "Every morning, Martha climbed the narrow stairs of the old lighthouse. From the top, the sea looked endless and calm.\n\nOne day, she found a letter hidden between two stones. The paper was damp, but the words could still be read.";

const MOCK_DICT: Record<string, [string, string, string]> = {
  lighthouse: ["Nom commun", "ˈlaɪt.haʊs", "Phare"],
  stairs: ["Nom commun", "stɛəz", "Escalier"],
  narrow: ["Adjectif", "ˈnæɹ.əʊ", "Étroit"],
  endless: ["Adjectif", "ˈɛnd.ləs", "Sans fin, infini"],
  letter: ["Nom commun", "ˈlɛt.ə", "Lettre"],
  damp: ["Adjectif", "dæmp", "Humide"],
  faro: ["Nom commun", "ˈfa.ɾo", "Phare"],
  carta: ["Nom commun", "ˈkaɾ.ta", "Lettre"],
};

export function createMockApi(): Api {
  let db = load();
  const commit = () => save(db);

  const bump = (lang: string, field: keyof DayStat, n: number) => {
    const k = `${today()}|${lang}`;
    const a = db.activity[k] ?? { day: today(), lang, words_read: 0, known_added: 0, lingqs: 0, listen_secs: 0 };
    (a[field] as number) += n;
    db.activity[k] = a;
  };

  const models: Omit<ModelRow, "installed" | "active" | "downloading" | "partial">[] = [
    { id: "qwen3.5-0.8b", kind: "llm", name: "Qwen3.5 0.8B", detail: "Très rapide, pour les Mac avec 8 Go de mémoire", size: 533e6, url: "", file: "", ram_gb: 8 },
    { id: "qwen3.5-2b", kind: "llm", name: "Qwen3.5 2B", detail: "L'équilibre idéal entre qualité et vitesse", size: 1281e6, url: "", file: "", ram_gb: 8 },
    { id: "qwen3.5-4b", kind: "llm", name: "Qwen3.5 4B", detail: "Les traductions les plus fines, à partir de 16 Go", size: 2741e6, url: "", file: "", ram_gb: 16 },
    { id: "whisper-small", kind: "asr", name: "Whisper Small", detail: "Transcription légère et rapide", size: 190e6, url: "", file: "", ram_gb: 8 },
    { id: "whisper-turbo", kind: "asr", name: "Whisper Large v3 Turbo", detail: "Transcription très précise, 99 langues", size: 574e6, url: "", file: "", ram_gb: 8 },
  ];

  const summary = (l: Lesson): LessonSummary => {
    const keys = tokenize(l.text, l.lang).filter((t) => t.w).map((t) => t.k);
    const uniq = new Set<string>();
    let rec = 0;
    for (const k of keys) {
      if (db.terms[`${l.lang}|${k}`]) rec++;
      else uniq.add(k);
    }
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
      known_pct: keys.length ? Math.floor((rec * 100) / keys.length) : 100,
      excerpt: l.text.slice(0, 220).replace(/\n/g, " "),
    };
  };

  const fakeStream = async (text: string, onPiece: (t: string) => void) => {
    for (const part of text.match(/.{1,4}/gsu) ?? []) {
      await sleep(18);
      onPiece(part);
    }
    return text;
  };

  const mock: Api = {
    async appInfo() {
      return { version: "0.1.0 (aperçu navigateur)", data_dir: "(navigateur)", platform: "web", ytdlp: false, transcriber: false, dict_langs: ["en", "es", "it", "de", "pt", "ru"] };
    },
    async settingsGet() {
      return { ...db.settings };
    },
    async settingsSet(key, value) {
      db.settings[key] = value;
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
      if (!l) throw "Leçon introuvable";
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
      commit();
    },
    async termSet(u: TermUpdate) {
      const term = normalize(u.term);
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
        const k = `${lang}|${normalize(raw)}`;
        if (!db.terms[k]) {
          db.terms[k] = { term: normalize(raw), status: 4, translation: "", note: "", lemma: "", context: "", updated_at: Date.now() / 1000 };
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
      const days: DayStat[] = [];
      for (let i = 29; i >= 0; i--) {
        const d = new Date(Date.now() - i * 86400000).toISOString().slice(0, 10);
        const a = db.activity[`${d}|${lang}`];
        days.push({ day: d, words_read: a?.words_read ?? 0, known_added: a?.known_added ?? 0, lingqs: a?.lingqs ?? 0, listen_secs: a?.listen_secs ?? 0 });
      }
      const all = Object.values(db.activity).filter((a) => a.lang === lang);
      return {
        known: terms.filter((t) => t.status === 4 && !t.term.includes(" ")).length,
        learning: terms.filter((t) => t.status <= 3).length,
        phrases: terms.filter((t) => t.term.includes(" ")).length,
        lessons: db.lessons.filter((l) => l.lang === lang).length,
        words_read_total: all.reduce((s, a) => s + a.words_read, 0),
        listen_secs_total: all.reduce((s, a) => s + a.listen_secs, 0),
        today: days[days.length - 1],
        days,
      };
    },
    async activityAdd(lang, wordsRead, listenSecs) {
      bump(lang, "words_read", wordsRead);
      bump(lang, "listen_secs", listenSecs);
      commit();
    },
    async exportVocab() {},
    async dictLookup(_lang, word) {
      await sleep(30);
      const d = MOCK_DICT[normalize(word)];
      if (!d) return { entries: [] };
      return { entries: [{ word: normalize(word), pos: d[0], ipa: d[1], glosses: [d[2]] }] };
    },
    async aiWord(_lang, word, _sentence, onPiece) {
      if (!db.installed.some((m) => m.startsWith("qwen"))) throw "NO_MODEL:Aucun modèle de traduction n'est installé. Ouvrez Réglages › IA locale.";
      await sleep(250);
      const d = MOCK_DICT[normalize(word)];
      const tr = d ? d[2].toLowerCase() : `« ${word} » (aperçu)`;
      const note = normalize(word).endsWith("ed") ? "Prétérit, action terminée dans le passé." : "";
      await fakeStream(`Sens : ${tr}\nNote : ${note || "-"}`, onPiece);
      return { translation: tr, note, cached: false };
    },
    async aiSentence(_lang, sentence, onPiece) {
      await sleep(200);
      return fakeStream(`[Traduction simulée] ${sentence}`, onPiece);
    },
    async aiSimplify(_lang, text, _level, onPiece) {
      await sleep(400);
      const simple = text.split(/(?<=[.!?])\s+/).slice(0, 8).join(" ");
      return fakeStream(simple, onPiece);
    },
    async aiWarmup() {
      return db.installed.some((m) => m.startsWith("qwen"));
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
    async fetchUrl() {
      throw "L'import de pages web fonctionne dans l'application Mac.";
    },
    async readFile() {
      throw "La lecture de fichiers fonctionne dans l'application Mac.";
    },
    async importMedia() {
      throw "La transcription fonctionne dans l'application Mac.";
    },
    async importYoutube() {
      throw "L'import YouTube fonctionne dans l'application Mac.";
    },
    async lessonFetchVideo() {
      throw "Le téléchargement de vidéos fonctionne dans l'application Mac.";
    },
    async lingqScan(key) {
      await sleep(900);
      if (key.trim().length < 10) throw "LingQ refuse cette clé API. Vérifiez-la sur lingq.com puis collez-la à nouveau.";
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
    mediaUrl(path) {
      return path;
    },
  };
  return mock;
}

// Pour réinitialiser l'aperçu : localStorage.removeItem("lumen-mock-db")
export const MOCK_LANG_DEFAULT: LangCode = "en";
