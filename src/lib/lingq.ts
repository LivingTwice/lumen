// Import depuis LingQ : analyse du compte, puis import en arrière-plan.
// L'état vit ici (et non dans les Réglages) pour que l'import continue
// pendant qu'on lit ou qu'on change de vue.
import { create } from "zustand";
import { api, errorText } from "./api";
import { count, t } from "./i18n";
import { langInfo } from "./langs";
import { useApp } from "./store";
import type { LangCode, LingqLang, LingqReport, LingqStage } from "./types";

export const LINGQ_KEY_URL = "https://www.lingq.com/accounts/apikey/";

export function stageLabel(stage: LingqStage): string {
  return {
    known: t("Mots connus", "Known words"),
    ignored: t("Mots ignorés", "Ignored words"),
    cards: "LingQ",
    lessons: t("Leçons", "Lessons"),
  }[stage];
}

type Phase = "idle" | "scanning" | "ready" | "importing" | "done";
type Option = "vocab" | "lessons" | "audio";

interface LingqState {
  phase: Phase;
  error: string;
  account: LingqLang[];
  chosen: LangCode[];
  vocab: boolean;
  lessons: boolean;
  audio: boolean;
  stage: { lang: LangCode; stage: LingqStage } | null;
  done: number;
  total: number;
  lastLesson: string;
  report: LingqReport | null;
  scan(key: string): Promise<void>;
  toggleLang(lang: LangCode): void;
  setOption(option: Option, value: boolean): void;
  start(key: string): Promise<void>;
  cancel(): Promise<void>;
  reset(): void;
}

/** « 1 leçon », « 3 leçons » : nombre et nom accordés (français, puis anglais). */
export const plural = count;

/** « 120 mots et 3 leçons importés », ou chaîne vide si rien n'a changé. */
export function summary(r: LingqReport): string {
  const parts: string[] = [];
  if (r.words) parts.push(plural(r.words, "mot", "mots", "word", "words"));
  if (r.lessons) parts.push(plural(r.lessons, "leçon", "leçons", "lesson", "lessons"));
  if (!parts.length) return "";
  const fem = !r.words;
  return t(`${parts.join(" et ")} import${fem ? "ée" : "é"}${r.words + r.lessons > 1 ? "s" : ""}`, `${parts.join(" and ")} imported`);
}

export const useLingq = create<LingqState>((set, get) => ({
  phase: "idle",
  error: "",
  account: [],
  chosen: [],
  vocab: true,
  lessons: true,
  audio: true,
  stage: null,
  done: 0,
  total: 0,
  lastLesson: "",
  report: null,

  async scan(key) {
    set({ phase: "scanning", error: "", report: null });
    try {
      const account = await api().lingqScan(key);
      if (!account.length) {
        set({ phase: "idle", error: t("Ce compte LingQ ne contient rien dans les langues que Lumen propose.", "This LingQ account has nothing in the languages Lumen offers.") });
        return;
      }
      // par défaut, seulement les langues déjà étudiées dans Lumen (les autres se cochent à la main)
      const studied = useApp.getState().langs();
      const mine = account.map((a) => a.lang).filter((l) => studied.includes(l));
      set({ phase: "ready", account, chosen: mine.length ? mine : account.map((a) => a.lang) });
    } catch (e) {
      set({ phase: "idle", error: errorText(e) });
    }
  },

  toggleLang(lang) {
    const { chosen } = get();
    set({ chosen: chosen.includes(lang) ? chosen.filter((l) => l !== lang) : [...chosen, lang] });
  },

  setOption(option, value) {
    set({ [option]: value } as Pick<LingqState, Option>);
  },

  async start(key) {
    const { chosen, vocab, lessons, audio } = get();
    if (!chosen.length || (!vocab && !lessons)) return;
    set({ phase: "importing", error: "", stage: null, done: 0, total: 0, lastLesson: "", report: null });
    const app = useApp.getState();
    // langues étudiées au départ : l'import n'ajoute que des langues nouvelles,
    // jamais une langue retirée dans les Réglages pendant qu'il tournait
    const before = app.langs();
    try {
      const report = await api().lingqImport(key, { langs: chosen, vocab, lessons, audio: lessons && audio }, (e) => {
        if (e.type === "stage") set({ stage: { lang: e.lang, stage: e.stage }, done: 0, total: 0 });
        else if (e.type === "progress") set({ done: e.done, total: e.total });
        else set({ lastLesson: e.title });
      });
      // les langues importées rejoignent celles étudiées dans Lumen
      const langs = useApp.getState().langs();
      const added = chosen.filter((l) => !langs.includes(l) && !before.includes(l));
      if (added.length) await app.setSetting("langs", [...langs, ...added].join(","));
      set({ phase: "done", report, stage: null });
      app.bumpLibrary();
      await app.refreshKnown();
      const s = summary(report);
      const fallback = report.skipped ? t("Tout était déjà dans Lumen", "Everything was already in Lumen") : t("Rien de nouveau à importer", "Nothing new to import");
      app.toast(report.cancelled ? t(`Import interrompu${s ? ` : ${s}` : ""}`, `Import stopped${s ? `: ${s}` : ""}`) : s || fallback, "light");
    } catch (e) {
      set({ phase: "ready", error: errorText(e), stage: null });
      app.toast(t(`Import LingQ impossible : ${errorText(e)}`, `LingQ import failed: ${errorText(e)}`), "error");
    }
  },

  async cancel() {
    await api().lingqCancel();
  },

  reset() {
    set({ phase: "idle", error: "", account: [], chosen: [], report: null, stage: null });
  },
}));

/** Ligne de progression lisible : « Anglais · Mots connus ». */
export function stageText(stage: LingqState["stage"]): string {
  if (!stage) return t("Connexion à LingQ…", "Connecting to LingQ…");
  return `${langInfo(stage.lang).name} · ${stageLabel(stage.stage)}`;
}
