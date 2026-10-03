// Playlists : des leçons d'une langue dans l'ordre choisi, qui s'enchaînent
// d'elles-mêmes à l'écoute (comme sur LingQ).

import { count, t } from "./i18n";
import { useApp } from "./store";
import type { LessonSummary, Playlist } from "./types";

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

/** Lance la playlist : la leçon s'ouvre et sa lecture démarre ; les suivantes s'enchaînent. */
export function playPlaylist(p: Playlist, from?: number) {
  const id = from ?? startOf(p);
  if (id) useApp.getState().openLesson(id, { playlist: p.id, autoplay: true });
}
