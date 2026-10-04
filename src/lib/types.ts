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
  /** dictionnaire en cours de téléchargement */
  pending?: boolean;
}

/** Dictionnaire d'une langue, dans la langue de l'interface. */
export interface DictStatus {
  /** un dictionnaire existe pour cette langue */
  exists: boolean;
  /** il est sur ce Mac */
  ready: boolean;
  downloading: boolean;
  /** livré avec l'application */
  bundled: boolean;
}

export interface DayStat {
  day: string;
  words_read: number;
  known_added: number;
  lingqs: number;
  listen_secs: number;
  /** temps actif passé dans les leçons */
  learn_secs: number;
  /** objectif du jour atteint : la journée compte dans la série */
  goal_met: boolean;
}

/** Activité cumulée sur une période (jour, semaine, mois, tout). */
export interface Span {
  /** premier jour de la période (AAAA-MM-JJ) */
  start: string;
  words_read: number;
  known_added: number;
  lingqs: number;
  listen_secs: number;
  learn_secs: number;
  active_days: number;
  goal_days: number;
}

export interface Streak {
  /** jours de suite où l'objectif est atteint (aujourd'hui compris s'il l'est déjà) */
  current: number;
  best: number;
  today_done: boolean;
  goal_min: number;
  today_secs: number;
}

export interface Stats {
  known: number;
  learning: number;
  phrases: number;
  lessons: number;
  /** 26 semaines entières (depuis un lundi) jusqu'à aujourd'hui */
  days: DayStat[];
  /** 12 dernières semaines */
  weeks: Span[];
  /** mois par mois depuis le début (12 au moins, 36 au plus) */
  months: Span[];
  periods: { today: Span; yesterday: Span; week: Span; last_week: Span; month: Span; last_month: Span; total: Span };
  streak: Streak;
  records: { words_read: number; words_day: string; learn_secs: number; learn_day: string };
  first_day: string | null;
}

/** Le temps d'apprentissage vient de faire atteindre l'objectif du jour. */
export interface GoalReached {
  streak: number;
  goal_min: number;
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

/** Son ou vidéo trouvé derrière un lien (miroir de `link::MediaItem`). */
export interface LinkMedia {
  /** fichier direct, ou adresse lue par yt-dlp (« ytsearch1: » pour un morceau) */
  url: string;
  title: string;
  /** secondes, 0 si inconnue */
  duration: number;
  video: boolean;
  /** fichier téléchargé tel quel, sans yt-dlp */
  direct: boolean;
  image: string;
  /** AAAA-MM-JJ, vide si inconnue */
  date: string;
  /** page d'origine (source de la leçon) */
  page: string;
  /** émission, playlist ou album */
  collection: string;
}

/** Ce que Lumen a trouvé derrière un lien (miroir de `link::LinkInfo`). */
export interface LinkInfo {
  url: string;
  title: string;
  /** site, émission, chaîne ou artiste */
  site: string;
  image: string;
  /** page HTML, pour en extraire l'article (vide pour un flux ou un fichier) */
  html: string;
  media: LinkMedia[];
  /** liste (podcast, playlist, album) : on choisit ses éléments */
  list: boolean;
  /** Spotify : le son vient du flux public du podcast ("rss") ou de YouTube */
  via: "" | "rss" | "youtube";
  /** à montrer si rien d'autre n'est trouvé */
  note: string;
}

/** Ce que propose Découvrir : une vidéo, un épisode ou un article d'une source
 *  choisie pour la langue (miroir de `discover::Item`). */
export interface DiscoverItem {
  id: string;
  source: string;
  source_name: string;
  /** rayon : pour apprenants, actualités, culture */
  shelf: "learn" | "news" | "culture";
  kind: "video" | "audio" | "text";
  title: string;
  /** vidéo YouTube, fichier son, ou article */
  url: string;
  /** page de l'épisode ou de l'article (vide si inconnue) */
  page: string;
  image: string;
  summary: string;
  /** secondes, 0 si inconnue */
  duration: number;
  /** secondes depuis 1970, 0 si inconnue */
  published: number;
  /** fourchette de niveaux, de 1 (A1) à 5 (C1) */
  lo: number;
  hi: number;
  /** la page porte aussi le texte de l'épisode (l'import propose le son avec ce texte) */
  page_text: boolean;
  /** première apparition dans Lumen (secondes) */
  fetched_at: number;
  /** leçon déjà créée à partir de cet élément */
  lesson_id: number | null;
}

export interface DiscoverFeed {
  items: DiscoverItem[];
  /** dernière lecture réussie des sources (secondes, 0 : jamais) */
  refreshed_at: number;
  refreshing: boolean;
  /** nombre de sources de la langue */
  sources: number;
}

export interface DiscoverReport {
  added: number;
  sources: number;
  /** sources qui n'ont pas répondu */
  failed: string[];
  /** une lecture venait de se faire : rien n'a été relu */
  skipped: boolean;
}

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
