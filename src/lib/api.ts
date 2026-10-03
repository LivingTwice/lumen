import type { AppInfo, BackupInfo, BackupRestored, BackupStatus, ChatEvent, ChatOptions, ChatPatch, ChatReply, ChatSummary, ChatThread, DictResult, DownloadEvent, ImportEvent, LangCode, LessonSummary, LingqEvent, LingqLang, LingqPlan, LingqReport, ModelRow, NewLesson, OpenedLesson, Playlist, PlaylistPatch, Stats, Term, TermQuery, WordAnswer, VoicedLesson } from "./types";

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export interface TermUpdate {
  lang: LangCode;
  term: string;
  status: number;
  translation?: string | null;
  note?: string | null;
  lemma?: string | null;
  context?: string | null;
}

export interface LessonPatch {
  title?: string;
  collection?: string;
  page?: number;
  completed?: boolean;
  position?: number;
  anchor?: number;
  duration?: number;
}

export interface Api {
  appInfo(): Promise<AppInfo>;
  settingsGet(): Promise<Record<string, string>>;
  settingsSet(key: string, value: string): Promise<void>;
  lessonsList(lang: LangCode): Promise<LessonSummary[]>;
  lessonOpen(id: number): Promise<OpenedLesson>;
  lessonCreate(lesson: NewLesson): Promise<number>;
  lessonUpdate(id: number, patch: LessonPatch): Promise<void>;
  lessonDelete(id: number): Promise<void>;
  /** Couverture (image déjà réduite) ; `null` la retire. Renvoie le chemin enregistré. */
  lessonSetCover(id: number, data: Uint8Array | null, ext: string | null): Promise<string | null>;
  playlistsList(lang: LangCode): Promise<Playlist[]>;
  /** crée une playlist (avec ses premières leçons) ; renvoie son identifiant */
  playlistCreate(lang: LangCode, name: string, lessons: number[]): Promise<number>;
  playlistUpdate(id: number, patch: PlaylistPatch): Promise<void>;
  playlistDelete(id: number): Promise<void>;
  termSet(update: TermUpdate): Promise<void>;
  termsMarkKnown(lang: LangCode, keys: string[], wordsRead: number): Promise<number>;
  termsList(query: TermQuery): Promise<{ items: Term[]; total: number }>;
  stats(lang: LangCode): Promise<Stats>;
  activityAdd(lang: LangCode, wordsRead: number, listenSecs: number): Promise<void>;
  exportVocab(lang: LangCode, path: string): Promise<void>;
  dictLookup(lang: LangCode, word: string): Promise<DictResult>;
  aiWord(lang: LangCode, word: string, sentence: string, onPiece: (t: string) => void): Promise<WordAnswer>;
  aiSentence(lang: LangCode, sentence: string, onPiece: (t: string) => void): Promise<string>;
  aiSimplify(lang: LangCode, text: string, level: string, onPiece: (t: string) => void): Promise<string>;
  aiWarmup(): Promise<boolean>;
  chatsList(lang: LangCode): Promise<ChatSummary[]>;
  chatOpen(id: number): Promise<ChatThread>;
  /** nouvelle conversation, avec une leçon jointe ou non */
  chatCreate(lang: LangCode, lesson: number | null): Promise<ChatSummary>;
  chatUpdate(id: number, patch: ChatPatch): Promise<ChatSummary>;
  chatDelete(id: number): Promise<void>;
  /** pose une question : la réflexion puis la réponse arrivent au fil de l'eau */
  chatSend(id: number, text: string, options: ChatOptions, onEvent: (e: ChatEvent) => void): Promise<ChatReply>;
  /** arrête la réponse en cours (ce qui est écrit est gardé) */
  chatStop(id: number): Promise<void>;
  modelsList(): Promise<ModelRow[]>;
  modelDownload(id: string, onEvent: (e: DownloadEvent) => void): Promise<void>;
  modelCancel(id: string): Promise<void>;
  modelDelete(id: string): Promise<void>;
  /** prononce avec la voix naturelle ; renvoie le chemin du fichier WAV (mis en cache) */
  ttsSay(lang: LangCode, text: string, prefetch: boolean): Promise<string>;
  /** crée l'audio d'une leçon de texte avec la voix naturelle */
  lessonVoice(id: number, onEvent: (e: ImportEvent) => void): Promise<VoicedLesson>;
  lessonVoiceCancel(id: number): Promise<void>;
  fetchUrl(url: string): Promise<string>;
  readFile(path: string): Promise<ArrayBuffer>;
  importMedia(lang: LangCode, path: string, title: string | null, onEvent: (e: ImportEvent) => void): Promise<number>;
  importYoutube(lang: LangCode, url: string, onEvent: (e: ImportEvent) => void): Promise<number>;
  lessonFetchVideo(id: number, onEvent: (e: ImportEvent) => void): Promise<string>;
  /** Réécoute l'audio et recale la lanterne sur le texte existant. Renvoie les horodatages. */
  lessonResync(id: number, onEvent: (e: ImportEvent) => void): Promise<string>;
  lingqScan(key: string): Promise<LingqLang[]>;
  lingqImport(key: string, plan: LingqPlan, onEvent: (e: LingqEvent) => void): Promise<LingqReport>;
  lingqCancel(): Promise<void>;
  /** état de la sauvegarde (le dossier n'est lu qu'une fois la sauvegarde activée) */
  backupStatus(): Promise<BackupStatus>;
  /** sauvegarde maintenant ; renvoie l'état à jour */
  backupRun(): Promise<BackupStatus>;
  /** sauvegardes trouvées dans le dossier, de ce Mac et des autres */
  backupList(): Promise<BackupInfo[]>;
  /** remplace la progression de ce Mac par une sauvegarde (`day` : version d'un jour précédent) */
  backupRestore(key: string, day: string | null, onEvent: (e: ImportEvent) => void): Promise<BackupRestored>;
  /** suit les sauvegardes automatiques ; renvoie de quoi arrêter l'écoute */
  backupListen(onStatus: (s: BackupStatus) => void): Promise<() => void>;
  mediaUrl(path: string): string;
}

async function createTauriApi(): Promise<Api> {
  const core = await import("@tauri-apps/api/core");
  const { invoke, Channel, convertFileSrc } = core;
  const ch = <T,>(fn: (v: T) => void) => {
    const c = new Channel<T>();
    c.onmessage = fn;
    return c;
  };
  return {
    appInfo: () => invoke("app_info"),
    settingsGet: () => invoke("settings_get"),
    settingsSet: (key, value) => invoke("settings_set", { key, value }),
    lessonsList: (lang) => invoke("lessons_list", { lang }),
    lessonOpen: (id) => invoke("lesson_open", { id }),
    lessonCreate: (lesson) => invoke("lesson_create", { lesson }),
    lessonUpdate: (id, patch) => invoke("lesson_update", { id, patch }),
    lessonDelete: (id) => invoke("lesson_delete", { id }),
    lessonSetCover: (id, data, ext) => invoke("lesson_set_cover", { id, data: data ? Array.from(data) : null, ext }),
    playlistsList: (lang) => invoke("playlists_list", { lang }),
    playlistCreate: (lang, name, lessons) => invoke("playlist_create", { lang, name, lessons }),
    playlistUpdate: (id, patch) => invoke("playlist_update", { id, patch }),
    playlistDelete: (id) => invoke("playlist_delete", { id }),
    termSet: (update) => invoke("term_set", { update }),
    termsMarkKnown: (lang, keys, wordsRead) => invoke("terms_mark_known", { lang, keys, wordsRead }),
    termsList: (query) => invoke("terms_list", { query }),
    stats: (lang) => invoke("stats", { lang }),
    activityAdd: (lang, wordsRead, listenSecs) => invoke("activity_add", { lang, wordsRead, listenSecs }),
    exportVocab: (lang, path) => invoke("export_vocab", { lang, path }),
    dictLookup: (lang, word) => invoke("dict_lookup", { lang, word }),
    aiWord: (lang, word, sentence, onPiece) =>
      invoke("ai_word", { lang, word, sentence, onEvent: ch<{ type: string; text: string }>((e) => onPiece(e.text)) }),
    aiSentence: (lang, sentence, onPiece) =>
      invoke("ai_sentence", { lang, sentence, onEvent: ch<{ type: string; text: string }>((e) => onPiece(e.text)) }),
    aiSimplify: (lang, text, level, onPiece) =>
      invoke("ai_simplify", { lang, text, level, onEvent: ch<{ type: string; text: string }>((e) => onPiece(e.text)) }),
    aiWarmup: () => invoke("ai_warmup"),
    chatsList: (lang) => invoke("chats_list", { lang }),
    chatOpen: (id) => invoke("chat_open", { id }),
    chatCreate: (lang, lesson) => invoke("chat_create", { lang, lesson }),
    chatUpdate: (id, patch) => invoke("chat_update", { id, patch }),
    chatDelete: (id) => invoke("chat_delete", { id }),
    chatSend: (id, text, options, onEvent) => invoke("chat_send", { id, text, options, onEvent: ch<ChatEvent>(onEvent) }),
    chatStop: (id) => invoke("model_cancel", { id: `chat:${id}` }),
    modelsList: () => invoke("models_list"),
    modelDownload: (id, onEvent) => invoke("model_download", { id, onEvent: ch<DownloadEvent>(onEvent) }),
    modelCancel: (id) => invoke("model_cancel", { id }),
    modelDelete: (id) => invoke("model_delete", { id }),
    ttsSay: (lang, text, prefetch) => invoke("tts_say", { lang, text, prefetch }),
    lessonVoice: (id, onEvent) => invoke("lesson_voice", { id, onEvent: ch<ImportEvent>(onEvent) }),
    lessonVoiceCancel: (id) => invoke("model_cancel", { id: `voice:${id}` }),
    fetchUrl: (url) => invoke("fetch_url", { url }),
    readFile: (path) => invoke<ArrayBuffer>("read_file", { path }),
    importMedia: (lang, path, title, onEvent) =>
      invoke("import_media", { lang, path, title, onEvent: ch<ImportEvent>(onEvent) }),
    importYoutube: (lang, url, onEvent) => invoke("import_youtube", { lang, url, onEvent: ch<ImportEvent>(onEvent) }),
    lessonFetchVideo: (id, onEvent) => invoke("lesson_fetch_video", { id, onEvent: ch<ImportEvent>(onEvent) }),
    lessonResync: (id, onEvent) => invoke("lesson_resync", { id, onEvent: ch<ImportEvent>(onEvent) }),
    lingqScan: (key) => invoke("lingq_scan", { key }),
    lingqImport: (key, plan, onEvent) => invoke("lingq_import", { key, plan, onEvent: ch<LingqEvent>(onEvent) }),
    lingqCancel: () => invoke("lingq_cancel"),
    backupStatus: () => invoke("backup_status"),
    backupRun: () => invoke("backup_run"),
    backupList: () => invoke("backup_list"),
    backupRestore: (key, day, onEvent) => invoke("backup_restore", { key, day, onEvent: ch<ImportEvent>(onEvent) }),
    backupListen: async (onStatus) => {
      const { listen } = await import("@tauri-apps/api/event");
      return listen<BackupStatus>("backup", (e) => onStatus(e.payload));
    },
    mediaUrl: (path) => convertFileSrc(path),
  };
}

let instance: Api | null = null;

export async function initApi(): Promise<Api> {
  if (instance) return instance;
  if (isTauri) {
    instance = await createTauriApi();
  } else {
    const { createMockApi } = await import("./mock");
    instance = createMockApi();
  }
  return instance;
}

/** À n'utiliser qu'après initApi() (appelé au démarrage). */
export function api(): Api {
  if (!instance) throw new Error("API non initialisée");
  return instance;
}

/** Message d'erreur lisible (les commandes renvoient des chaînes). */
export function errorText(e: unknown): string {
  const s = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
  return s.replace(/^NO_(MODEL|VOICE):\s*/, "");
}

export function isNoModel(e: unknown): boolean {
  return typeof e === "string" && e.startsWith("NO_MODEL:");
}
