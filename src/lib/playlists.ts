// Playlists : des leçons d'une langue dans l'ordre choisi, qui s'enchaînent
// d'elles-mêmes à l'écoute (comme sur LingQ).

import { api, errorText } from "./api";
import { count, t } from "./i18n";
import { useApp } from "./store";
import type { LessonSummary, Playlist } from "./types";

export function pagesOf(words: number) {
  return Math.max(1, Math.round(words / 230));
}

/** Avancement (0 à 1) : à la seconde près pour l'audio et la vidéo, à la page près sinon. */
export function progressOf(l: LessonSummary): number {
  if (l.completed) return 1;
  if (l.has_media && l.duration > 0) return Math.min(1, l.position / l.duration);
  return l.page / pagesOf(l.word_count);
}

/** Un livre importé : la playlist de ses chapitres, vue comme un tout. */
export interface BookView {
  p: Playlist;
  /** chapitres, dans l'ordre du livre */
  ls: LessonSummary[];
  /** chapitre où la lecture reprend */
  at: LessonSummary;
  words: number;
  /** avancement (0 à 1), chaque chapitre compté selon sa longueur */
  progress: number;
  /** chapitres terminés */
  done: number;
  completed: boolean;
  started: boolean;
  hasMedia: boolean;
  /** durée du son des chapitres, en secondes */
  duration: number;
  /** dernière ouverture d'un chapitre (sinon création du livre) : sa place dans la bibliothèque */
  recent: number;
  /** part des mots nouveaux, moyenne des chapitres pondérée par leur longueur */
  newPct: number;
}

export const isBook = (p: Playlist) => p.kind === "book";

/** Le livre d'une playlist « book » ; null s'il n'a plus de chapitre. */
export function bookView(p: Playlist, all: LessonSummary[] | Map<number, LessonSummary>): BookView | null {
  const byId = all instanceof Map ? all : new Map(all.map((l) => [l.id, l]));
  const ls = p.lessons.map((id) => byId.get(id)).filter((l): l is LessonSummary => !!l);
  if (!ls.length) return null;
  const words = ls.reduce((n, l) => n + l.word_count, 0);
  const weight = (l: LessonSummary) => (words > 0 ? l.word_count / words : 1 / ls.length);
  const done = ls.filter((l) => l.completed).length;
  // reprise : le chapitre en cours, ou le suivant s'il vient d'être terminé
  const from = Math.max(0, ls.findIndex((l) => l.id === p.current));
  const at = ls[from].completed ? (ls.slice(from + 1).find((l) => !l.completed) ?? ls.find((l) => !l.completed) ?? ls[from]) : ls[from];
  return {
    p,
    ls,
    at,
    words,
    progress: Math.min(1, ls.reduce((s, l) => s + progressOf(l) * weight(l), 0)),
    done,
    completed: done === ls.length,
    started: ls.some((l) => l.opened_at),
    hasMedia: ls.some((l) => l.has_media),
    duration: listenSecs(ls),
    recent: Math.max(...ls.map((l) => l.opened_at ?? l.created_at)),
    newPct: Math.round(ls.reduce((s, l) => s + l.new_pct * weight(l), 0)),
  };
}

/** Ouvre le livre là où la lecture en est (le son ne démarre pas de lui-même). */
export function readBook(b: BookView, from?: number) {
  useApp.getState().openLesson(from ?? b.at.id, { playlist: b.p.id });
}

/** Leçons d'une playlist, dans son ordre (une leçon introuvable est ignorée). */
export function lessonsOf(p: Playlist, all: LessonSummary[]): LessonSummary[] {
  const byId = new Map(all.map((l) => [l.id, l]));
  return p.lessons.map((id) => byId.get(id)).filter((l): l is LessonSummary => !!l);
}

/** Durée d'écoute connue (audio et vidéo), en secondes. */
export function listenSecs(ls: LessonSummary[]): number {
  return ls.reduce((s, l) => s + (l.has_media ? l.duration : 0), 0);
}

/** « 42 min », « 1 h 12 min », « moins d'une minute ». */
export function formatLength(secs: number): string {
  const m = Math.round(secs / 60);
  if (m < 1) return t("moins d'une minute", "less than a minute");
  const h = Math.floor(m / 60);
  if (!h) return `${m} min`;
  return m % 60 ? t(`${h} h ${m % 60} min`, `${h} h ${m % 60} min`) : `${h} h`;
}

/** « 1 leçon », « 8 leçons » : mot français puis anglais, pluriel en « s » dans les deux langues. */
export function plural(n: number, fr: string, en: string): string {
  return count(n, fr, `${fr}s`, en, `${en}s`);
}

/** Leçon par laquelle la playlist reprend (là où elle en était), sinon la première. */
export function startOf(p: Playlist): number | null {
  return p.current ?? p.lessons[0] ?? null;
}

/** Leçon qui suit ; en boucle, la première après la dernière (une playlist d'une leçon : elle-même). */
export function nextIn(p: Playlist, id: number): number | undefined {
  const i = p.lessons.indexOf(id);
  if (i < 0) return undefined;
  return p.lessons[i + 1] ?? (p.looping && !isBook(p) ? p.lessons[0] : undefined);
}

/**
 * Marque des leçons lues ou non lues (les mots ne changent pas de statut) ; la
 * bibliothèque et les playlists se relisent. `said` : la notification, sinon le nombre de leçons.
 */
export async function markRead(ids: number[], read: boolean, said?: string): Promise<boolean> {
  const app = useApp.getState();
  if (!ids.length) return false;
  try {
    await api().lessonsSetCompleted(ids, read);
  } catch (e) {
    app.toast(errorText(e), "error");
    return false;
  }
  app.bumpLibrary();
  app.toast(
    said ??
      (read
        ? count(ids.length, "leçon marquée comme lue", "leçons marquées comme lues", "lesson marked as read", "lessons marked as read")
        : count(ids.length, "leçon marquée comme non lue", "leçons marquées comme non lues", "lesson marked as unread", "lessons marked as unread")),
    "light",
  );
  return true;
}

/** Lance la playlist : la leçon s'ouvre et sa lecture démarre ; les suivantes s'enchaînent. */
export function playPlaylist(p: Playlist, from?: number) {
  const id = from ?? startOf(p);
  if (id) useApp.getState().openLesson(id, { playlist: p.id, autoplay: true });
}
