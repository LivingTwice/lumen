export type LangCode = "en" | "es" | "it" | "de" | "pt" | "ru";

export interface Token {
  /** texte tel qu'il apparaît */
  t: string;
  /** vrai si c'est un mot à apprendre */
  w: boolean;
  /** clé normalisée */
  k: string;
  /** positions de début et de fin (unités UTF-16) */
  s: number;
  e: number;
}

/** 1 à 3 : en apprentissage, 4 : connu, 5 : ignoré. Absent : nouveau. */
export type Status = 1 | 2 | 3 | 4 | 5;

export interface Term {
  term: string;
  status: Status;
  translation: string;
  note: string;
  lemma: string;
  context: string;
  updated_at: number;
}

export interface LessonSummary {
  id: number;
  lang: LangCode;
  title: string;
  collection: string;
  kind: string;
  source: string;
  hue: number;
  word_count: number;
  page: number;
  completed: boolean;
  has_media: boolean;
  created_at: number;
  opened_at: number | null;
  new_words: number;
  known_pct: number;
  excerpt: string;
}

export interface Lesson {
  id: number;
  lang: LangCode;
  title: string;
  collection: string;
  kind: string;
  source: string;
  text: string;
  media_path: string | null;
  timings: string | null;
  video_path: string | null;
  hue: number;
  word_count: number;
  page: number;
  completed: boolean;
}

export interface OpenedLesson {
  lesson: Lesson;
  tokens: Token[];
  terms: Record<string, Term>;
}

export interface NewLesson {
  lang: LangCode;
  title: string;
  collection?: string;
  kind?: string;
  source?: string;
  text: string;
  media_path?: string | null;
  timings?: string | null;
  video_path?: string | null;
}

export interface DictEntry {
  word: string;
  pos: string;
  ipa: string;
  glosses: string[];
}

export interface DictResult {
  entries: DictEntry[];
  lemma?: string | null;
  form_note?: string | null;
}

export interface DayStat {
  day: string;
  words_read: number;
  known_added: number;
  lingqs: number;
  listen_secs: number;
}

export interface Stats {
  known: number;
  learning: number;
  phrases: number;
  lessons: number;
  words_read_total: number;
  listen_secs_total: number;
  today: DayStat;
  days: DayStat[];
}

export interface ModelRow {
  id: string;
  kind: "llm" | "asr";
  name: string;
  detail: string;
  size: number;
  url: string;
  file: string;
  ram_gb: number;
  installed: boolean;
  active: boolean;
  downloading: boolean;
  partial: number;
}

export interface AppInfo {
  version: string;
  data_dir: string;
  platform: string;
  ytdlp: boolean;
  transcriber: boolean;
  dict_langs: string[];
}

export interface WordAnswer {
  translation: string;
  note: string;
  cached: boolean;
}

export type DownloadEvent =
  | { type: "progress"; received: number; total: number; speed: number }
  | { type: "done" };

export type ImportEvent = { type: "stage"; stage: string } | { type: "progress"; value: number };

export interface TermQuery {
  lang: LangCode;
  filter: "all" | "learning" | "known" | "ignored" | "phrases";
  search?: string;
  limit: number;
  offset: number;
}

// ---------- import LingQ ----------

export interface LingqCourse {
  id: number;
  title: string;
  lessons: number;
}

/** Contenu du compte LingQ pour une langue. */
export interface LingqLang {
  lang: LangCode;
  known_words: number;
  lingqs: number;
  courses: LingqCourse[];
  lessons: number;
}

export interface LingqPlan {
  langs: LangCode[];
  vocab: boolean;
  lessons: boolean;
  audio: boolean;
}

export type LingqStage = "known" | "ignored" | "cards" | "lessons";

export type LingqEvent =
  | { type: "stage"; lang: LangCode; stage: LingqStage }
  | { type: "progress"; done: number; total: number }
  | { type: "lesson"; title: string; course: string };

export interface LingqReport {
  words: number;
  lessons: number;
  skipped: number;
  failed: number;
  audio_failed: number;
  cancelled: boolean;
}
