export type LangCode =
  | "en" | "es" | "it" | "de" | "pt" | "ru"
  | "fr" | "nl" | "sv" | "da" | "fi" | "et" | "lv" | "lt" | "pl" | "cs" | "sk" | "sl" | "hr" | "hu" | "ro"
  | "bg" | "uk" | "el" | "tr" | "ar" | "hi" | "id" | "vi" | "ko" | "ja";

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
  /** seconde atteinte dans l'audio ou la vidéo, et durée totale (0 si inconnue) */
  position: number;
  duration: number;
  cover_path: string | null;
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
  position: number;
  /** jeton (mot) où la lecture s'était arrêtée */
  anchor: number;
  duration: number;
  cover_path: string | null;
  /** 2 : mots calés précisément sur la voix ; 0 : minutage approximatif (ancien Whisper, LingQ) */
  timing_v: number;
}

/** Leçons d'une langue dans l'ordre choisi. */
export interface Playlist {
  id: number;
  lang: LangCode;
  name: string;
  /** identifiants des leçons, dans l'ordre de lecture */
  lessons: number[];
  /** leçon où la lecture de la playlist en est (null : elle repart du début) */
  current: number | null;
  created_at: number;
}

export interface PlaylistPatch {
  name?: string;
  lessons?: number[];
  /** 0 : la playlist repart du début */
  current?: number;
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
  /** asr : Whisper (transcription et minutage des mots) ; asrtext : Qwen3-ASR (texte plus juste) */
  kind: "llm" | "asr" | "asrtext" | "tts";
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

/** Conversation du chat. */
export interface ChatSummary {
  id: number;
  lang: LangCode;
  /** vide tant que la première question n'est pas posée */
  title: string;
  lesson_id: number | null;
  lesson_title: string | null;
  updated_at: number;
  count: number;
  /** début du dernier message */
  preview: string;
}

export interface ChatMessage {
  id: number;
  role: "user" | "assistant";
  content: string;
  /** réflexion du modèle avant sa réponse (vide sans réflexion) */
  thought: string;
  thought_secs: number;
  created_at: number;
}

export interface ChatThread {
  chat: ChatSummary;
  messages: ChatMessage[];
}

export interface ChatPatch {
  title?: string;
  /** leçon jointe ; 0 la retire */
  lesson?: number;
}

/** Longueur permise à la réflexion. */
export type ChatEffort = "low" | "medium" | "high";

export interface ChatOptions {
  think: boolean;
  effort: ChatEffort;
  /** passage lu dans la leçon jointe (position UTF-16), pour les longues leçons */
  focus: number | null;
}

export type ChatEvent = { type: "thought"; text: string } | { type: "answer"; text: string };

export interface ChatReply {
  chat: ChatSummary;
  /** question et réponse enregistrées (aucune si l'arrêt est venu avant le premier mot) */
  user: ChatMessage | null;
  assistant: ChatMessage | null;
  stopped: boolean;
}

export type DownloadEvent =
  | { type: "progress"; received: number; total: number; speed: number }
  | { type: "done" };

export type ImportEvent = { type: "stage"; stage: string } | { type: "progress"; value: number };

/** Nombres d'une sauvegarde : ce qui sera retrouvé en la restaurant. */
export interface BackupCounts {
  known: number;
  learning: number;
  phrases: number;
  lessons: number;
  langs: LangCode[];
}

/** État d'envoi du fichier de progression dans iCloud. */
export type CloudState = "uploaded" | "uploading" | "waiting" | "error" | "local" | "unknown";

/** Sauvegarde de ce Mac (iCloud Drive ou dossier choisi). */
export interface BackupStatus {
  /** réglage backup_on : "1" active, "0" coupée, "" pas encore choisie */
  enabled: boolean;
  decided: boolean;
  /** dossier « Lumen » de la sauvegarde ; null : ni iCloud Drive ni dossier choisi */
  dir: string | null;
  icloud: boolean;
  icloud_available: boolean;
  running: boolean;
  last_at: number | null;
  size: number;
  media_size: number;
  media_count: number;
  counts: BackupCounts;
  cloud: CloudState;
  cloud_error: string | null;
  error: string | null;
  /** audio et vidéos de ce Mac, pour les interrupteurs */
  local_audio: number;
  local_video: number;
}

export interface BackupVersion {
  day: string;
  saved_at: number;
  known: number;
  lessons: number;
}

/** Sauvegarde trouvée dans le dossier (de ce Mac ou d'un autre). */
export interface BackupInfo {
  key: string;
  device_name: string;
  /** sauvegarde de ce Mac et du profil en cours */
  mine: boolean;
  this_device: boolean;
  saved_at: number;
  app_version: string;
  size: number;
  media_size: number;
  counts: BackupCounts;
  /** versions des jours précédents, de la plus récente à la plus ancienne */
  versions: BackupVersion[];
  /** faite par une version plus récente de Lumen */
  newer: boolean;
}

export interface BackupRestored {
  counts: BackupCounts;
  /** médias absents de la sauvegarde (leçons restaurées sans leur audio ou leur vidéo) */
  missing_media: number;
}

/** Audio créé par la voix naturelle pour une leçon de texte. */
export interface VoicedLesson {
  media_path: string;
  timings: string;
  timing_v: number;
  duration: number;
}

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
