// Chat avec l'IA locale : conversations, réponse en cours, leçon jointe.
// L'état vit ici (et non dans une vue) pour que la réponse continue de
// s'écrire pendant qu'on passe du lecteur à la vue Chat.
import { create } from "zustand";
import { api, errorText, isNoModel } from "./api";
import { langInfo } from "./langs";
import { useApp } from "./store";
import type { ChatEffort, ChatMessage, ChatSummary, ChatThread, LangCode } from "./types";

export const EFFORTS: { value: ChatEffort; label: string; hint: string }[] = [
  { value: "low", label: "Rapide", hint: "Une courte réflexion, pour vérifier une intuition" },
  { value: "medium", label: "Équilibré", hint: "Le bon compromis entre justesse et rapidité" },
  { value: "high", label: "Approfondi", hint: "La réflexion la plus longue, pour les questions difficiles" },
];

export interface LessonRef {
  id: number;
  title: string;
}

export interface Pending {
  chatId: number;
  question: string;
  thought: string;
  answer: string;
  /** l'IA lit la conversation, réfléchit, puis répond */
  phase: "read" | "think" | "answer";
  think: boolean;
  /** début de la réflexion (ms) */
  since: number;
  /** durée de la réflexion, connue au premier mot de la réponse */
  thoughtSecs: number;
}

interface ChatState {
  /** langue des conversations chargées */
  lang: LangCode | null;
  list: ChatSummary[];
  /** conversation ouverte ; null : nouvelle conversation, créée à la première question */
  thread: ChatThread | null;
  /** leçon jointe à la nouvelle conversation, avant sa création */
  draftLesson: LessonRef | null;
  pending: Pending | null;
  error: { text: string; noModel: boolean } | null;
  draft: string;
  /** demande de mise au point du champ de saisie (compteur) */
  focusTick: number;
  /** passage lu dans une leçon : cadre l'extrait confié à l'IA pour les longues leçons */
  reading: { lesson: number; offset: number } | null;

  load(lang: LangCode): Promise<void>;
  open(id: number): Promise<void>;
  fresh(lesson?: LessonRef | null): void;
  /** conversation de cette leçon : la plus récente, sinon une nouvelle */
  forLesson(lesson: LessonRef & { lang: LangCode }): Promise<void>;
  /** question posée depuis le lecteur, dans la conversation de la leçon */
  ask(lesson: LessonRef & { lang: LangCode }, text: string): Promise<void>;
  attach(lesson: LessonRef | null): Promise<void>;
  send(text: string): Promise<void>;
  stop(): Promise<void>;
  remove(id: number): Promise<void>;
  rename(id: number, title: string): Promise<void>;
  setDraft(text: string): void;
  focus(): void;
}

/** Leçon jointe à la conversation affichée. */
export function attachedLesson(s: Pick<ChatState, "thread" | "draftLesson">): LessonRef | null {
  if (s.thread) return s.thread.chat.lesson_id ? { id: s.thread.chat.lesson_id, title: s.thread.chat.lesson_title ?? "Leçon" } : null;
  return s.draftLesson;
}

/** « l'italien », « le russe » : nom de la langue avec son article. */
export function langWithArticle(lang: LangCode): string {
  const n = langInfo(lang).name.toLowerCase();
  // h muet dans « l'hindi », aspiré dans « le hongrois »
  return /^[aeéiou]/.test(n) || n === "hindi" ? `l'${n}` : `le ${n}`;
}

/** Questions proposées sur une conversation vide. */
export function suggestions(lang: LangCode, withLesson: boolean): string[] {
  const name = langInfo(lang).name.toLowerCase();
  if (withLesson)
    return [
      "Résume cette leçon en quelques phrases",
      "Explique-moi les points de grammaire importants du texte",
      "Quels mots de cette leçon dois-je retenir en priorité ?",
      `Pose-moi trois questions en ${name} sur le texte`,
    ];
  return [
    `Discutons en ${name}, à mon niveau`,
    `Quelles sont les difficultés de ${langWithArticle(lang)} pour un francophone ?`,
    `Écris-moi une courte histoire en ${name} pour débutant`,
    "Donne-moi dix mots utiles pour la vie de tous les jours",
  ];
}

/** Question préparée sur un mot, une expression ou un passage du lecteur ;
 *  `sense` : la traduction en contexte déjà trouvée, qui guide l'explication. */
export function askAbout(surface: string, sentence: string, passage: boolean, sense = ""): string {
  if (passage) return `Explique-moi ce passage : « ${surface} »`;
  const word = sense ? `« ${surface} » (traduit ici par « ${sense} »)` : `« ${surface} »`;
  const s = sentence.trim();
  return s && s !== surface ? `Explique-moi ${word} dans cette phrase : « ${s} »` : `Explique-moi ${word}`;
}

/** conversation de leçon en cours de recherche (évite d'en ouvrir deux à la fois) */
let finding: { lesson: number; done: Promise<void> } | null = null;

export const useChat = create<ChatState>((set, get) => ({
  lang: null,
  list: [],
  thread: null,
  draftLesson: null,
  pending: null,
  error: null,
  draft: "",
  focusTick: 0,
  reading: null,

  async load(lang) {
    const changed = get().lang !== lang;
    if (changed) set({ lang, list: [], thread: null, draftLesson: null, error: null });
    try {
      const list = await api().chatsList(lang);
      if (get().lang === lang) set({ list });
    } catch {
      /* liste indisponible : on garde l'état */
    }
  },

  async open(id) {
    try {
      const thread = await api().chatOpen(id);
      set({ thread, draftLesson: null, error: null });
    } catch (e) {
      useApp.getState().toast(errorText(e), "error");
      set((s) => ({ list: s.list.filter((c) => c.id !== id) }));
    }
  },

  fresh(lesson = null) {
    set({ thread: null, draftLesson: lesson, error: null });
  },

  async forLesson(lesson) {
    if (finding?.lesson === lesson.id) return finding.done;
    const run = async () => {
      if (get().lang !== lesson.lang || !get().list.length) await get().load(lesson.lang);
      const s = get();
      if (s.thread ? s.thread.chat.lesson_id === lesson.id : s.draftLesson?.id === lesson.id) return;
      const last = s.list.find((c) => c.lesson_id === lesson.id);
      if (last) await get().open(last.id);
      else get().fresh(lesson);
    };
    const done = run().finally(() => {
      if (finding?.done === done) finding = null;
    });
    finding = { lesson: lesson.id, done };
    return done;
  },

  async ask(lesson, text) {
    await get().forLesson(lesson);
    // une réponse s'écrit déjà : la question attend dans le champ
    if (get().pending) {
      set({ draft: text });
      get().focus();
      return;
    }
    await get().send(text);
  },

  async attach(lesson) {
    const t = get().thread;
    if (!t) {
      set({ draftLesson: lesson });
      return;
    }
    try {
      const chat = await api().chatUpdate(t.chat.id, { lesson: lesson?.id ?? 0 });
      set((s) => ({
        thread: s.thread?.chat.id === chat.id ? { ...s.thread, chat } : s.thread,
        list: s.list.map((c) => (c.id === chat.id ? chat : c)),
      }));
    } catch (e) {
      useApp.getState().toast(errorText(e), "error");
    }
  },

  async send(text) {
    const question = text.trim();
    if (!question || get().pending) return;
    const app = useApp.getState();
    const lang = get().lang ?? app.lang();
    set({ error: null, draft: "" });
    let thread = get().thread;
    if (!thread) {
      try {
        const chat = await api().chatCreate(lang, get().draftLesson?.id ?? null);
        thread = { chat, messages: [] };
        set((s) => ({ thread, draftLesson: null, list: [chat, ...s.list.filter((c) => c.id !== chat.id)] }));
      } catch (e) {
        set({ error: { text: errorText(e), noModel: false }, draft: question });
        return;
      }
    }
    const id = thread.chat.id;
    const think = app.setting("chat_think") === "1";
    const effort = (app.setting("chat_effort") || "medium") as ChatEffort;
    const reading = get().reading;
    const focus = reading && reading.lesson === thread.chat.lesson_id ? reading.offset : null;
    set({ pending: { chatId: id, question, thought: "", answer: "", phase: "read", think, since: 0, thoughtSecs: 0 } });
    try {
      const reply = await api().chatSend(id, question, { think, effort, focus }, (e) => {
        const p = get().pending;
        if (!p || p.chatId !== id) return;
        if (e.type === "thought") set({ pending: { ...p, phase: "think", since: p.since || Date.now(), thought: p.thought + e.text } });
        else
          set({
            pending: {
              ...p,
              phase: "answer",
              thoughtSecs: p.phase === "think" ? (Date.now() - p.since) / 1000 : p.thoughtSecs,
              answer: p.answer + e.text,
            },
          });
      });
      set((s) => {
        const added = [reply.user, reply.assistant].filter((m): m is ChatMessage => !!m);
        const here = s.thread?.chat.id === id;
        return {
          pending: null,
          thread: here && s.thread ? { chat: reply.chat, messages: [...s.thread.messages, ...added] } : s.thread,
          list: reply.chat.lang === s.lang ? [reply.chat, ...s.list.filter((c) => c.id !== id)] : s.list,
          // arrêt avant le premier mot : la question revient dans le champ
          draft: !reply.user && here && !s.draft ? question : s.draft,
        };
      });
    } catch (e) {
      set((s) => ({
        pending: null,
        error: s.thread?.chat.id === id ? { text: errorText(e), noModel: isNoModel(e) } : s.error,
        draft: s.thread?.chat.id === id && !s.draft ? question : s.draft,
      }));
    }
  },

  async stop() {
    const p = get().pending;
    if (p) await api().chatStop(p.chatId).catch(() => {});
  },

  async remove(id) {
    if (get().pending?.chatId === id) await get().stop();
    try {
      await api().chatDelete(id);
      set((s) => ({
        list: s.list.filter((c) => c.id !== id),
        ...(s.thread?.chat.id === id ? { thread: null, draftLesson: null, error: null } : {}),
      }));
    } catch (e) {
      useApp.getState().toast(errorText(e), "error");
    }
  },

  async rename(id, title) {
    try {
      const chat = await api().chatUpdate(id, { title });
      set((s) => ({
        thread: s.thread?.chat.id === id ? { ...s.thread, chat } : s.thread,
        list: s.list.map((c) => (c.id === id ? chat : c)),
      }));
    } catch (e) {
      useApp.getState().toast(errorText(e), "error");
    }
  },

  setDraft(text) {
    set({ draft: text });
  },

  focus() {
    set((s) => ({ focusTick: s.focusTick + 1 }));
  },
}));
