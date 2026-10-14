// Bibliothèque : ce qu'elle montre (leçons, et livres réunis en un seul élément),
// leur tri, et le menu « … » commun aux cartes et à la liste.
import { useState } from "react";
import { KIND } from "../../components/Cover";
import { Icon } from "../../components/Icon";
import { Menu } from "../../components/ui";
import { count, locale, t } from "../../lib/i18n";
import { formatLength, progressOf, type BookView } from "../../lib/playlists";
import { formatDuration } from "../../lib/store";
import type { LessonSummary } from "../../lib/types";

/** Présentation de la bibliothèque (réglage `library_view`). */
export type LibraryView = "large" | "small" | "list";

export const LIBRARY_VIEWS: LibraryView[] = ["large", "small", "list"];

export function viewOf(v: string | undefined): LibraryView {
  return LIBRARY_VIEWS.includes(v as LibraryView) ? (v as LibraryView) : "large";
}

/** Une leçon seule, ou un livre (tous ses chapitres en un). */
export type Entry = { key: string; l: LessonSummary; b?: undefined } | { key: string; b: BookView; l?: undefined };

export type SortKey = "date" | "title" | "kind" | "length" | "new" | "progress";

export interface Sort {
  key: SortKey;
  desc: boolean;
}

const SORT_KEYS: SortKey[] = ["date", "title", "kind", "length", "new", "progress"];

/** Réglage `library_sort` (« date:desc ») ; par défaut, la plus récemment lue d'abord. */
export function parseSort(v: string | undefined): Sort {
  const [key, dir] = (v ?? "").split(":");
  return SORT_KEYS.includes(key as SortKey) ? { key: key as SortKey, desc: dir !== "asc" } : { key: "date", desc: true };
}

export const sortText = (s: Sort) => `${s.key}:${s.desc ? "desc" : "asc"}`;

/** Sens d'un premier clic sur une colonne : le plus récent, le plus long, le plus avancé d'abord ; les noms de A à Z. */
export const firstDesc = (key: SortKey) => key !== "title" && key !== "kind";

export const titleOf = (e: Entry) => (e.b ? e.b.p.name : e.l.title);
export const recentOf = (e: Entry) => (e.b ? e.b.recent : (e.l.opened_at ?? e.l.created_at));
export const kindOf = (e: Entry) => (e.b ? t("Livre", "Book") : (KIND[e.l.kind] ?? KIND.text).label);
export const progressOfEntry = (e: Entry) => (e.b ? e.b.progress : progressOf(e.l));
export const newOf = (e: Entry) => (e.b ? e.b.newPct : e.l.new_pct);
export const wordsOf = (e: Entry) => (e.b ? e.b.words : e.l.word_count);
export const lessonsIn = (e: Entry) => (e.b ? e.b.ls.length : 1);
export const completedOf = (e: Entry) => (e.b ? e.b.completed : e.l.completed);
/** Leçons d'un élément : la leçon, ou tous les chapitres du livre. */
export const idsOf = (e: Entry) => (e.b ? e.b.ls.map((l) => l.id) : [e.l.id]);

/** Longueur affichée : la durée du son, sinon le nombre de mots. */
export function lengthText(e: Entry): string {
  if (e.b) return e.b.hasMedia && e.b.duration > 0 ? formatLength(e.b.duration) : count(e.b.words, "mot", "mots", "word", "words");
  return e.l.has_media && e.l.duration > 0 ? formatDuration(e.l.duration) : count(e.l.word_count, "mot", "mots", "word", "words");
}

/** Tri de la liste ; à égalité, la plus récente d'abord. */
export function sortEntries(list: Entry[], s: Sort): Entry[] {
  const coll = new Intl.Collator(locale(), { sensitivity: "base", numeric: true });
  const value = (e: Entry): number | string => {
    switch (s.key) {
      case "title":
        return titleOf(e);
      case "kind":
        return kindOf(e);
      case "length":
        return wordsOf(e);
      case "new":
        return newOf(e);
      case "progress":
        return progressOfEntry(e);
      default:
        return recentOf(e);
    }
  };
  const sign = s.desc ? -1 : 1;
  return [...list].sort((a, b) => {
    const x = value(a);
    const y = value(b);
    const c = typeof x === "string" ? coll.compare(x, y as string) : x - (y as number);
    return c ? c * sign : recentOf(b) - recentOf(a);
  });
}

/** « Aujourd'hui », « Hier », « 12 oct. », « 3 mars 2025 ». */
export function dayText(ts: number): string {
  const d = new Date(ts * 1000);
  const now = new Date();
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const ago = Math.round((day(now) - day(d)) / 864e5);
  if (ago === 0) return t("Aujourd'hui", "Today");
  if (ago === 1) return t("Hier", "Yesterday");
  return d.toLocaleDateString(locale(), { day: "numeric", month: "short", year: d.getFullYear() === now.getFullYear() ? undefined : "numeric" });
}

/** Ce que le menu « … » d'un élément permet. */
export interface EntryActions {
  /** « Sélectionner plusieurs » : la sélection commence par cet élément */
  onSelect(): void;
  onDelete(): void;
  onRename(): void;
  /** lue (tout le livre), ou de nouveau à lire */
  onMark(): void;
  /** leçon : l'ajouter à une playlist */
  onPlaylist?(): void;
  /** livre : reprendre la lecture, voir ses chapitres */
  onRead?(): void;
  onChapters?(): void;
}

/** Bouton « … » et son menu, pour une carte ou une ligne de la liste. */
export function EntryMenu({ e, actions, open, setOpen }: { e: Entry; actions: EntryActions; open: boolean; setOpen(v: boolean): void }) {
  const run = (fn?: () => void) => () => {
    setOpen(false);
    fn?.();
  };
  return (
    <Menu
      open={open}
      onClose={() => setOpen(false)}
      align="right"
      anchor={
        <button className="icon-btn" onClick={() => setOpen(!open)} aria-label={e.b ? t("Options du livre", "Book options") : t("Options de la leçon", "Lesson options")}>
          <Icon name="more" size={18} stroke={2.6} />
        </button>
      }
    >
      {e.b ? (
        <>
          <button className="menu-item" onClick={run(actions.onRead)}>
            <Icon name="book" size={16} /> {e.b.started ? t("Reprendre la lecture", "Continue reading") : t("Commencer la lecture", "Start reading")}
          </button>
          <button className="menu-item" onClick={run(actions.onChapters)}>
            <Icon name="playlist" size={16} /> {t("Voir les chapitres", "See the chapters")}
          </button>
          <button className="menu-item" onClick={run(actions.onRename)}>
            <Icon name="edit" size={16} /> {t("Renommer le livre", "Rename the book")}
          </button>
        </>
      ) : (
        <>
          <button className="menu-item" onClick={run(actions.onPlaylist)}>
            <Icon name="playlist" size={16} /> {t("Ajouter à une playlist…", "Add to a playlist…")}
          </button>
          <button className="menu-item" onClick={run(actions.onRename)}>
            <Icon name="edit" size={16} /> {t("Renommer", "Rename")}
          </button>
        </>
      )}
      <button className="menu-item" onClick={run(actions.onMark)}>
        <Icon name={completedOf(e) ? "unread" : "read"} size={16} /> {completedOf(e) ? t("Marquer comme non lu", "Mark as unread") : t("Marquer comme lu", "Mark as read")}
      </button>
      <button className="menu-item" onClick={run(actions.onSelect)}>
        <Icon name="check" size={16} /> {t("Sélectionner plusieurs éléments", "Select several items")}
      </button>
      <div className="menu-sep" />
      <button className="menu-item danger" onClick={run(actions.onDelete)}>
        <Icon name="trash" size={16} /> {e.b ? t("Supprimer le livre", "Delete the book") : t("Supprimer", "Delete")}
      </button>
    </Menu>
  );
}

/** Menu « … » ouvert aussi d'un clic droit sur l'élément, comme dans le Finder. */
export function useEntryMenu() {
  const [menu, setMenu] = useState(false);
  const onContextMenu = (ev: React.MouseEvent) => {
    ev.preventDefault();
    setMenu(true);
  };
  return { menu, setMenu, onContextMenu };
}
