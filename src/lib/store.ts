import { create } from "zustand";
import { api, errorText } from "./api";
import type { AppInfo, DownloadEvent, LangCode, ModelRow } from "./types";

export type View = "library" | "reader" | "vocab" | "progress" | "settings";

export interface Toast {
  id: number;
  text: string;
  kind: "info" | "error" | "light";
}

export interface DownloadState {
  received: number;
  total: number;
  speed: number;
  error?: string;
}

export const DEFAULTS: Record<string, string> = {
  theme: "system",
  font_size: "23",
  line_height: "1.75",
  word_style: "tint",
  auto_sentence: "1",
  finish_marks_known: "1",
  tts_rate: "0.95",
  llm_model: "qwen3.5-2b",
  asr_model: "whisper-turbo",
  langs: "",
  lang: "",
  onboarded: "",
};

interface AppStore {
  ready: boolean;
  info: AppInfo | null;
  settings: Record<string, string>;
  view: View;
  lessonId: number | null;
  importOpen: boolean;
  importFiles: string[] | null;
  toasts: Toast[];
  models: ModelRow[];
  downloads: Record<string, DownloadState>;
  libraryVersion: number;
  knownCount: number;
  /** rejoue l'écran d'accueil depuis les Réglages */
  replay: boolean;

  init(): Promise<void>;
  setReplay(v: boolean): void;
  setting(key: string): string;
  setSetting(key: string, value: string): Promise<void>;
  lang(): LangCode;
  langs(): LangCode[];
  go(view: View): void;
  openLesson(id: number): void;
  openImport(files?: string[] | null): void;
  closeImport(): void;
  toast(text: string, kind?: Toast["kind"]): void;
  refreshModels(): Promise<void>;
  download(id: string): Promise<void>;
  cancelDownload(id: string): Promise<void>;
  bumpLibrary(): void;
  refreshKnown(): Promise<void>;
}

let toastId = 0;

export const useApp = create<AppStore>((set, get) => ({
  ready: false,
  info: null,
  settings: { ...DEFAULTS },
  view: "library",
  lessonId: null,
  importOpen: false,
  importFiles: null,
  toasts: [],
  models: [],
  downloads: {},
  libraryVersion: 0,
  knownCount: 0,
  replay: false,

  setReplay(v) {
    set({ replay: v });
  },

  async init() {
    const [info, settings] = await Promise.all([api().appInfo(), api().settingsGet()]);
    set({ info, settings: { ...DEFAULTS, ...settings }, ready: true });
    await get().refreshModels();
    await get().refreshKnown();
    // prépare l'IA en arrière-plan dès l'ouverture de l'application
    if (get().models.some((m) => m.kind === "llm" && m.installed)) void api().aiWarmup().catch(() => {});
  },

  setting(key) {
    return get().settings[key] ?? DEFAULTS[key] ?? "";
  },

  async setSetting(key, value) {
    set((s) => ({ settings: { ...s.settings, [key]: value } }));
    await api().settingsSet(key, value);
  },

  lang() {
    const l = get().setting("lang");
    return (l || get().langs()[0] || "en") as LangCode;
  },

  langs() {
    return get()
      .setting("langs")
      .split(",")
      .filter(Boolean) as LangCode[];
  },

  go(view) {
    set({ view });
  },

  openLesson(id) {
    set({ lessonId: id, view: "reader" });
  },

  openImport(files = null) {
    set({ importOpen: true, importFiles: files });
  },

  closeImport() {
    set({ importOpen: false, importFiles: null });
  },

  toast(text, kind = "info") {
    const id = ++toastId;
    set((s) => ({ toasts: [...s.toasts, { id, text, kind }] }));
    setTimeout(() => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })), kind === "error" ? 6000 : 3600);
  },

  async refreshModels() {
    try {
      const models = await api().modelsList();
      set({ models });
    } catch {
      /* ignoré */
    }
  },

  async download(id) {
    if (get().downloads[id] && !get().downloads[id].error) return;
    const m = get().models.find((x) => x.id === id);
    set((s) => ({ downloads: { ...s.downloads, [id]: { received: m?.partial ?? 0, total: m?.size ?? 1, speed: 0 } } }));
    try {
      await api().modelDownload(id, (e: DownloadEvent) => {
        if (e.type === "progress") {
          set((s) => ({ downloads: { ...s.downloads, [id]: { received: e.received, total: e.total, speed: e.speed } } }));
        }
      });
      set((s) => {
        const d = { ...s.downloads };
        delete d[id];
        return { downloads: d };
      });
      get().toast(`${m?.name ?? "Modèle"} est prêt`, "light");
      // charge le modèle tout de suite pour que la première traduction soit immédiate
      if (m?.kind === "llm") void api().aiWarmup().catch(() => {});
    } catch (e) {
      const msg = errorText(e);
      set((s) => {
        const d = { ...s.downloads };
        if (msg === "annulé") delete d[id];
        else d[id] = { ...(d[id] ?? { received: 0, total: 1, speed: 0 }), error: msg };
        return { downloads: d };
      });
      if (msg !== "annulé") get().toast(`Téléchargement interrompu : ${msg}`, "error");
    }
    await get().refreshModels();
  },

  async cancelDownload(id) {
    await api().modelCancel(id);
    set((s) => {
      const d = { ...s.downloads };
      delete d[id];
      return { downloads: d };
    });
  },

  bumpLibrary() {
    set((s) => ({ libraryVersion: s.libraryVersion + 1 }));
  },

  async refreshKnown() {
    try {
      const st = await api().stats(get().lang());
      set({ knownCount: st.known });
    } catch {
      /* ignoré */
    }
  },
}));

export function formatBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1).replace(".", ",")} Go`;
  if (n >= 1e6) return `${Math.round(n / 1e6)} Mo`;
  return `${Math.round(n / 1e3)} Ko`;
}

export function formatNumber(n: number): string {
  return n.toLocaleString("fr-FR");
}

export function formatDuration(secs: number): string {
  const s = Math.max(0, Math.round(secs));
  const m = Math.floor(s / 60);
  const r = s % 60;
  return `${m}:${String(r).padStart(2, "0")}`;
}
