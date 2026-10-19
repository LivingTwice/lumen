import { AnimatePresence, motion } from "motion/react";
import { useEffect, useMemo, useRef, useState } from "react";
import { AddToPlaylist } from "../components/AddToPlaylist";
import { BookCover, Cover } from "../components/Cover";
import { Icon } from "../components/Icon";
import { useCompact } from "../components/MobileNav";
import { Orb, ScrollTop, Segmented, Sheet, useGlow } from "../components/ui";
import { api, errorText } from "../lib/api";
import { confirmAsk } from "../lib/dialogs";
import { count, isEn, t } from "../lib/i18n";
import { STARTERS, inLang, starterCollection } from "../lib/langs";
import { bookView, isBook, markRead, pagesOf, progressOf, readBook, type BookView } from "../lib/playlists";
import { formatDuration, formatNumber, useApp } from "../lib/store";
import { isWindows } from "../lib/platform";
import type { LessonSummary, Playlist } from "../lib/types";
import { useUserName } from "../lib/user";
import { LibraryList } from "./library/LibraryList";
import {
  EntryMenu,
  completedOf,
  firstDesc,
  idsOf,
  lessonsIn,
  parseSort,
  sortEntries,
  sortText,
  useEntryMenu,
  viewOf,
  type Entry,
  type EntryActions,
  type LibraryView,
  type SortKey,
} from "./library/entries";

type Filter = "all" | "text" | "audio" | "book" | "done";

function greeting() {
  const h = new Date().getHours();
  if (isEn()) return h >= 5 && h < 12 ? "Good morning" : h >= 12 && h < 18 ? "Good afternoon" : "Good evening";
  if (h < 5) return "Bonne nuit";
  if (h < 18) return "Bonjour";
  return "Bonsoir";
}

// Les cartes glissent vers leur nouvelle place (recherche, filtre, nouvelle leçon)
// sans attendre leur tour : seule l'apparition est décalée de l'une à l'autre.
const SLIDE = { type: "spring", stiffness: 420, damping: 40, mass: 0.8 } as const;

// Une très grande bibliothèque (import LingQ) ne s'affiche pas d'un bloc : les leçons
// les plus récentes d'abord, les autres par « Voir plus ». Des rangées pleines : 24 pour
// une, deux ou trois grandes cartes par rangée, 48 jusqu'à six petites ; la liste, plus dense.
const BATCH: Record<LibraryView, number> = { large: 24, small: 48, list: 60 };

/** Enveloppe animée d'une carte (apparition, glissement) : la carte garde son survol en CSS. */
function Cell({ rise, children, ref }: { rise?: number; children: React.ReactNode; ref?: React.Ref<HTMLDivElement> }) {
  return (
    // deux animations sur la même propriété transform se contrarieraient
    <motion.div
      ref={ref}
      layout="position"
      className="lesson-cell"
      initial={rise !== undefined ? { opacity: 0, y: 14 } : { opacity: 0, scale: 0.97 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, scale: 0.97, transition: { duration: 0.16, ease: "easeOut" } }}
      transition={{
        default:
          rise !== undefined ? { delay: Math.min(rise, 12) * 0.035, type: "spring", stiffness: 260, damping: 28 } : { duration: 0.22, ease: [0.2, 0.8, 0.2, 1] },
        layout: SLIDE,
      }}
    >
      {children}
    </motion.div>
  );
}

/** Coche de la sélection, en haut à gauche de la couverture. */
function Check({ selecting, picked }: { selecting: boolean; picked: boolean }) {
  return (
    <AnimatePresence>
      {selecting && (
        <motion.span
          className={`lesson-check ${picked ? "on" : ""}`}
          aria-hidden="true"
          initial={{ opacity: 0, scale: 0.5 }}
          // cochée : un petit rebond de lumière
          animate={{ opacity: 1, scale: picked ? [null, 1.22, 1] : 1 }}
          exit={{ opacity: 0, scale: 0.6, transition: { duration: 0.14 } }}
          transition={{ opacity: { duration: 0.16 }, scale: { duration: 0.32, ease: [0.2, 0.8, 0.2, 1] } }}
        >
          <Icon name="check" size={14} stroke={2.8} />
        </motion.span>
      )}
    </AnimatePresence>
  );
}

interface CardProps {
  small: boolean;
  /** rang de la carte quand elle se lève avec les autres (première apparition de la bibliothèque, « Voir plus ») */
  rise?: number;
  /** mode sélection : toucher la carte la coche au lieu de l'ouvrir */
  selecting: boolean;
  picked: boolean;
  onPick(e: React.MouseEvent): void;
  actions: EntryActions;
  ref?: React.Ref<HTMLDivElement>;
}

function LessonCard({ l, small, rise, selecting, picked, onPick, actions, ref }: CardProps & { l: LessonSummary }) {
  const openLesson = useApp((s) => s.openLesson);
  const glow = useGlow<HTMLDivElement>();
  const { menu, setMenu, onContextMenu } = useEntryMenu();
  return (
    <Cell ref={ref} rise={rise}>
      <div
        className={`lesson-card glow-surface ${small ? "small" : ""} ${selecting ? "selecting" : ""} ${picked ? "picked" : ""}`}
        ref={glow.ref}
        onMouseMove={glow.onMouseMove}
        onContextMenu={selecting ? undefined : onContextMenu}
      >
        {selecting ? (
          <button className="lesson-hit" onClick={onPick} aria-pressed={picked} aria-label={t(`Sélectionner ${l.title}`, `Select ${l.title}`)} />
        ) : (
          <button className="lesson-hit" onClick={() => openLesson(l.id)} aria-label={t(`Ouvrir ${l.title}`, `Open ${l.title}`)} />
        )}
        <Check selecting={selecting} picked={picked} />
        <Cover lesson={l} progress={progressOf(l)} duration={l.has_media ? l.duration : 0} editable={!small} />
        <div className="lesson-body">
          {l.collection && !small && <span className="lesson-collection">{l.collection}</span>}
          <h3 className="lesson-title">{l.title}</h3>
          {!small && <p className="lesson-excerpt">{l.excerpt}</p>}
          <div className="lesson-meta">
            {!small && (
              <>
                <span className="num">{count(l.word_count, "mot", "mots", "word", "words")}</span>
                <span className="sep" />
              </>
            )}
            <span className="num new-dot">
              {formatNumber(l.new_words)} {t("nouveaux", "new")}
            </span>
            <span
              className="num new-pct"
              title={t(`${l.new_pct} % des mots différents de la leçon sont nouveaux pour vous`, `${l.new_pct}% of the lesson's distinct words are new to you`)}
            >
              {t(`${l.new_pct} %`, `${l.new_pct}%`)}
            </span>
            {l.completed && (
              <span className="done-pill">
                <Icon name="check" size={12} stroke={2.4} /> {t("Lu", "Read")}
              </span>
            )}
          </div>
          {!small && (
            <div
              className="lesson-known"
              title={t(
                `${l.known_pct} % des mots différents de la leçon sont déjà connus ou en cours d'apprentissage`,
                `${l.known_pct}% of the lesson's distinct words are already known or being learned`,
              )}
            >
              <div className="bar">
                <i style={{ width: `${l.known_pct}%` }} />
              </div>
              <span className="num">{t(`${l.known_pct} %`, `${l.known_pct}%`)}</span>
            </div>
          )}
        </div>
        <div className="lesson-menu">
          <EntryMenu e={{ key: "", l }} actions={actions} open={menu} setOpen={setMenu} />
        </div>
      </div>
    </Cell>
  );
}

/** Un livre : une seule carte pour tous ses chapitres ; la toucher reprend la lecture. */
function BookCard({ b, small, rise, selecting, picked, onPick, actions, ref }: CardProps & { b: BookView }) {
  const openPlaylist = useApp((s) => s.openPlaylist);
  const glow = useGlow<HTMLDivElement>();
  const { menu, setMenu, onContextMenu } = useEntryMenu();
  const chapters = count(b.ls.length, "chapitre", "chapitres", "chapter", "chapters");
  const pct = Math.round(b.progress * 100);
  const where = b.completed
    ? t("Lu en entier.", "Read cover to cover.")
    : b.started
      ? t(`Chapitre ${b.ls.indexOf(b.at) + 1} sur ${b.ls.length} : ${b.at.title}`, `Chapter ${b.ls.indexOf(b.at) + 1} of ${b.ls.length}: ${b.at.title}`)
      : b.ls[0].excerpt;
  return (
    <Cell ref={ref} rise={rise}>
      <div
        className={`lesson-card book-card glow-surface ${small ? "small" : ""} ${selecting ? "selecting" : ""} ${picked ? "picked" : ""}`}
        ref={glow.ref}
        onMouseMove={glow.onMouseMove}
        onContextMenu={selecting ? undefined : onContextMenu}
      >
        {selecting ? (
          <button className="lesson-hit" onClick={onPick} aria-pressed={picked} aria-label={t(`Sélectionner ${b.p.name}`, `Select ${b.p.name}`)} />
        ) : (
          <button
            className="lesson-hit"
            onClick={() => readBook(b)}
            aria-label={b.started ? t(`Reprendre ${b.p.name}`, `Continue ${b.p.name}`) : t(`Commencer ${b.p.name}`, `Start ${b.p.name}`)}
          />
        )}
        <Check selecting={selecting} picked={picked} />
        <BookCover
          lesson={b.ls[0]}
          title={b.p.name}
          label={small ? t("Livre", "Book") : `${t("Livre", "Book")} · ${chapters}`}
          progress={b.progress}
          editable={!small}
          also={b.ls.slice(1).map((l) => l.id)}
        />
        <div className="lesson-body">
          {!small && <span className="lesson-collection">{b.started && !b.completed ? t("En cours de lecture", "Reading now") : t("Livre", "Book")}</span>}
          <h3 className="lesson-title">{b.p.name}</h3>
          {!small && <p className="lesson-excerpt">{where}</p>}
          <div className="lesson-meta">
            {/* au-dessus de la zone qui ouvre le livre : mène à ses chapitres */}
            <button className="book-chapters" onClick={() => openPlaylist(b.p.id)} disabled={selecting} title={t("Voir les chapitres", "See the chapters")}>
              <Icon name="playlist" size={13} /> {chapters}
            </button>
            {!small && (
              <>
                <span className="sep" />
                <span className="num">{count(b.words, "mot", "mots", "word", "words")}</span>
              </>
            )}
            {b.completed && (
              <span className="done-pill">
                <Icon name="check" size={12} stroke={2.4} /> {t("Lu", "Read")}
              </span>
            )}
          </div>
          {!small && (
            <div className="lesson-known" title={t(`${pct} % du livre lu, ${b.done} chapitre${b.done > 1 ? "s" : ""} terminé${b.done > 1 ? "s" : ""}`, `${pct}% of the book read, ${b.done} chapter${b.done === 1 ? "" : "s"} finished`)}>
              <div className="bar book-bar">
                <i style={{ width: `${pct}%` }} />
              </div>
              <span className="num">{t(`${pct} %`, `${pct}%`)}</span>
            </div>
          )}
        </div>
        <div className="lesson-menu">
          <EntryMenu e={{ key: "", b }} actions={actions} open={menu} setOpen={setMenu} />
        </div>
      </div>
    </Cell>
  );
}

/** Choix de la présentation : grandes cartes, petites cartes, liste. */
function ViewSwitch({ value, onChange }: { value: LibraryView; onChange(v: LibraryView): void }) {
  return (
    <Segmented
      id="lib-view"
      label={t("Présentation", "View")}
      value={value}
      onChange={onChange}
      options={[
        { value: "large", label: <Icon name="bigcards" size={17} />, title: t("Grandes cartes", "Large cards") },
        { value: "small", label: <Icon name="smallcards" size={17} />, title: t("Petites cartes", "Small cards") },
        { value: "list", label: <Icon name="rows" size={17} />, title: t("Liste", "List") },
      ]}
    />
  );
}

export function Library() {
  const lang = useApp((s) => s.lang)();
  const langSetting = useApp((s) => s.settings.lang);
  const version = useApp((s) => s.libraryVersion);
  const bump = useApp((s) => s.bumpLibrary);
  const openImport = useApp((s) => s.openImport);
  const openLesson = useApp((s) => s.openLesson);
  const openPlaylist = useApp((s) => s.openPlaylist);
  const go = useApp((s) => s.go);
  const toast = useApp((s) => s.toast);
  const known = useApp((s) => s.knownCount);
  const view = viewOf(useApp((s) => s.settings.library_view));
  const sort = parseSort(useApp((s) => s.settings.library_sort));
  const setSetting = useApp((s) => s.setSetting);
  const name = useUserName();
  const compact = useCompact();
  const [lessons, setLessons] = useState<LessonSummary[] | null>(null);
  const [playlists, setPlaylists] = useState<Playlist[]>([]);
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const hero = useGlow<HTMLDivElement>();
  const scroller = useRef<HTMLDivElement>(null);
  // l'entrée en cascade ne vaut que pour la première apparition des leçons (et pour
  // une autre présentation) ; ensuite (recherche, filtre), les cartes apparaissent et glissent sans attendre
  const [intro, setIntro] = useState(true);
  useEffect(() => {
    if (!lessons || !intro) return;
    const timer = window.setTimeout(() => setIntro(false), 900);
    return () => window.clearTimeout(timer);
  }, [lessons, intro]);
  const firstView = useRef(view);
  useEffect(() => {
    if (firstView.current === view) return;
    firstView.current = view;
    setIntro(true);
  }, [view]);

  useEffect(() => {
    Promise.all([api().lessonsList(lang), api().playlistsList(lang)])
      .then(([ls, ps]) => {
        setLessons(ls);
        setPlaylists(ps);
      })
      .catch((e) => toast(errorText(e), "error"));
  }, [lang, langSetting, version, toast]);

  // « Voir plus » : nombre d'éléments affichés, et d'où se lèvent ceux qui viennent d'arriver
  const [limit, setLimit] = useState(BATCH[view]);
  const [risen, setRisen] = useState<number | null>(null);
  useEffect(() => {
    setLimit(BATCH[view]);
    setRisen(null);
  }, [lang, filter, query, view]);
  useEffect(() => {
    if (risen === null) return;
    const timer = window.setTimeout(() => setRisen(null), 900);
    return () => window.clearTimeout(timer);
  }, [risen]);
  const showMore = () => {
    setRisen(limit);
    setLimit((n) => n + BATCH[view]);
  };

  // livres : leurs chapitres ne font qu'une carte, à la place du plus récemment lu
  const books = useMemo(() => {
    if (!lessons) return [];
    const byId = new Map(lessons.map((l) => [l.id, l]));
    return playlists
      .filter(isBook)
      .map((p) => bookView(p, byId))
      .filter((b): b is BookView => !!b);
  }, [lessons, playlists]);
  const bookOf = useMemo(() => {
    const m = new Map<number, BookView>();
    for (const b of books) for (const l of b.ls) if (!m.has(l.id)) m.set(l.id, b);
    return m;
  }, [books]);
  const entries = useMemo<Entry[]>(() => {
    if (!lessons) return [];
    const out: Entry[] = [];
    const placed = new Set<number>();
    for (const l of lessons) {
      const b = bookOf.get(l.id);
      if (!b) out.push({ key: `l${l.id}`, l });
      else if (!placed.has(b.p.id)) {
        placed.add(b.p.id);
        out.push({ key: `b${b.p.id}`, b });
      }
    }
    return out;
  }, [lessons, bookOf]);

  const resume = useMemo(() => lessons?.find((l) => l.opened_at && !l.completed), [lessons]);
  const resumeBook = resume ? bookOf.get(resume.id) : undefined;

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    const has = (s: string) => s.toLowerCase().includes(q);
    const list = entries.filter((e) => {
      if (e.b) {
        if (q && !(has(e.b.p.name) || e.b.ls.some((l) => has(l.title)))) return false;
        if (filter === "text") return false;
        if (filter === "audio") return e.b.hasMedia;
        if (filter === "done") return e.b.completed;
        return true;
      }
      const l = e.l;
      if (q && !(has(l.title) || has(l.collection))) return false;
      if (filter === "text") return ["text", "web", "pdf", "subtitles", "simplified"].includes(l.kind);
      if (filter === "audio") return l.has_media;
      if (filter === "book") return l.kind === "book";
      if (filter === "done") return l.completed;
      return true;
    });
    // les cartes gardent l'ordre de lecture ; la liste se trie par colonne
    return view === "list" ? sortEntries(list, sort) : list;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [entries, filter, query, view, sort.key, sort.desc]);

  const onSort = (key: SortKey) => {
    const desc = sort.key === key ? !sort.desc : firstDesc(key);
    void setSetting("library_sort", sortText({ key, desc }));
  };

  const addStarter = async () => {
    const s = STARTERS[lang];
    const id = await api().lessonCreate({ lang, title: s.title, text: s.text, collection: starterCollection(), kind: "text" });
    bump();
    openLesson(id);
  };

  /** Supprime des leçons et des livres (avec tous leurs chapitres), après confirmation. */
  const removeEntries = async (list: Entry[]) => {
    if (!list.length) return false;
    const lone = list.filter((e) => e.l).map((e) => e.l!);
    const bks = list.filter((e) => e.b).map((e) => e.b!);
    const ids = [...lone.map((l) => l.id), ...bks.flatMap((b) => b.ls.map((l) => l.id))];
    const what = [
      lone.length ? count(lone.length, "leçon", "leçons", "lesson", "lessons") : "",
      bks.length ? count(bks.length, "livre", "livres", "book", "books") : "",
    ]
      .filter(Boolean)
      .join(t(" et ", " and "));
    const ok = await confirmAsk(
      list.length === 1 && lone.length
        ? t(`Supprimer « ${lone[0].title} » ? Les mots appris sont conservés.`, `Delete “${lone[0].title}”? The words you learned are kept.`)
        : list.length === 1
          ? t(
              `Supprimer le livre « ${bks[0].p.name} » et ses ${count(bks[0].ls.length, "leçon", "leçons", "", "")} ? Les mots appris sont conservés.`,
              `Delete the book “${bks[0].p.name}” and its ${count(bks[0].ls.length, "", "", "lesson", "lessons")}? The words you learned are kept.`,
            )
          : t(`Supprimer ${what} ? Les mots appris sont conservés.`, `Delete ${what}? The words you learned are kept.`),
      list.length === 1 && bks.length ? t("Supprimer le livre", "Delete the book") : ids.length === 1 ? t("Supprimer la leçon", "Delete the lesson") : t("Supprimer", "Delete"),
      t("Supprimer", "Delete"),
    );
    if (!ok) return false;
    try {
      await api().lessonsDelete(ids);
      // un livre sans chapitre n'a plus de raison d'être dans les playlists
      for (const b of bks) await api().playlistDelete(b.p.id);
    } catch (e) {
      toast(errorText(e), "error");
      bump();
      return false;
    }
    const app = useApp.getState();
    for (const id of ids) app.forgetLesson(id);
    if (bks.some((b) => b.p.id === app.queue)) useApp.setState({ queue: null });
    toast(
      list.length === 1 && bks.length
        ? t("Livre supprimé", "Book deleted")
        : bks.length
          ? t(`${what} supprimé${list.length > 1 ? "s" : ""}`, `${what} deleted`)
          : count(ids.length, "leçon supprimée", "leçons supprimées", "lesson deleted", "lessons deleted"),
    );
    bump();
    return true;
  };

  /** Leçons et livres lus, ou de nouveau à lire ; un livre se reprend alors à son premier chapitre. */
  const markEntries = async (list: Entry[], read: boolean) => {
    await Promise.all(list.filter((e) => e.b).map((e) => api().playlistUpdate(e.b!.p.id, { current: 0 }).catch(() => {})));
    const b = list.length === 1 ? list[0].b : undefined;
    return markRead(
      list.flatMap(idsOf),
      read,
      b && (read ? t(`« ${b.p.name} » est marqué comme lu`, `“${b.p.name}” marked as read`) : t(`« ${b.p.name} » est marqué comme non lu`, `“${b.p.name}” marked as unread`)),
    );
  };

  // ---------- sélection de plusieurs éléments (pour les supprimer ou les marquer lus d'un coup) ----------
  const [selecting, setSelecting] = useState(false);
  const [picked, setPicked] = useState<Set<string>>(() => new Set());
  // dernier élément touché : Maj + clic coche tout l'intervalle jusqu'à lui
  const pickFrom = useRef<string | null>(null);
  const startSelect = (first?: string) => {
    setSelecting(true);
    setPicked(new Set(first === undefined ? [] : [first]));
    pickFrom.current = first ?? null;
  };
  const stopSelect = () => {
    setSelecting(false);
    setPicked(new Set());
    pickFrom.current = null;
  };
  const pick = (key: string, range: boolean) => {
    // lu avant la mise à jour : React l'applique plus tard, pickFrom aura déjà changé
    const fromKey = pickFrom.current;
    pickFrom.current = key;
    setPicked((now) => {
      const next = new Set(now);
      const from = fromKey === null ? -1 : shown.findIndex((x) => x.key === fromKey);
      const to = shown.findIndex((x) => x.key === key);
      if (range && from >= 0 && to >= 0) {
        for (let i = Math.min(from, to); i <= Math.max(from, to); i++) next.add(shown[i].key);
      } else if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  };
  const allPicked = shown.length > 0 && shown.every((e) => picked.has(e.key));
  const pickAll = () => setPicked(allPicked ? new Set() : new Set(shown.map((e) => e.key)));
  // ce qui est coché reste visible : un filtre ou une recherche décoche ce qu'il cache
  useEffect(() => {
    setPicked((now) => {
      const keys = new Set(shown.map((e) => e.key));
      const kept = [...now].filter((k) => keys.has(k));
      return kept.length === now.size ? now : new Set(kept);
    });
  }, [shown]);
  const pickedEntries = shown.filter((e) => picked.has(e.key));
  const pickedLessons = pickedEntries.reduce((n, e) => n + lessonsIn(e), 0);
  const pickedRead = pickedEntries.length > 0 && pickedEntries.every(completedOf);
  const removePicked = async () => {
    if (await removeEntries(shown.filter((e) => picked.has(e.key)))) stopSelect();
  };
  useEffect(() => {
    if (!selecting) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return;
      const typing = e.target instanceof HTMLElement && !!e.target.closest("input, textarea, select, [contenteditable]");
      if (e.key === "Escape") {
        e.preventDefault();
        stopSelect();
      } else if (!typing && (isWindows ? e.ctrlKey : e.metaKey) && e.key.toLowerCase() === "a") {
        e.preventDefault();
        setPicked(new Set(shown.map((x) => x.key)));
      } else if (!typing && (e.key === "Backspace" || e.key === "Delete") && picked.size) {
        e.preventDefault();
        void removePicked();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });
  // menu Édition › Sélectionner des leçons
  useEffect(() => {
    const onMenu = () => startSelect();
    window.addEventListener("lumen:select-lessons", onMenu);
    return () => window.removeEventListener("lumen:select-lessons", onMenu);
  }, []);

  const [adding, setAdding] = useState<LessonSummary | null>(null);
  const [renaming, setRenaming] = useState<Entry | null>(null);
  const [newTitle, setNewTitle] = useState("");
  const rename = (e: Entry) => {
    setNewTitle(e.b ? e.b.p.name : e.l.title);
    setRenaming(e);
  };
  const saveRename = async () => {
    const title = newTitle.trim();
    if (!renaming || !title) return;
    try {
      if (renaming.b) {
        // le livre, et la collection de ses chapitres (affichée dans le lecteur)
        await api().playlistUpdate(renaming.b.p.id, { name: title });
        await Promise.all(renaming.b.ls.map((l) => api().lessonUpdate(l.id, { collection: title })));
      } else await api().lessonUpdate(renaming.l.id, { title });
    } catch (e) {
      toast(errorText(e), "error");
    }
    setRenaming(null);
    bump();
  };

  const actions = (e: Entry): EntryActions => ({
    onSelect: () => startSelect(e.key),
    onDelete: () => void removeEntries([e]),
    onRename: () => rename(e),
    onMark: () => void markEntries([e], !completedOf(e)),
    onPlaylist: e.l ? () => setAdding(e.l) : undefined,
    onRead: e.b ? () => readBook(e.b) : undefined,
    onChapters: e.b ? () => openPlaylist(e.b.p.id) : undefined,
  });

  const rise = (i: number) => (intro ? i : risen !== null && i >= risen ? i - risen : undefined);
  const page = shown.slice(0, limit);
  const more = shown.slice(limit).reduce((n, e) => n + lessonsIn(e), 0);

  return (
    <>
      <div className="titlebar drag" data-tauri-drag-region>
        <div style={{ flex: 1 }} data-tauri-drag-region />
        <label className="search no-drag">
          <Icon name="search" size={16} />
          <input data-find placeholder={t("Rechercher une leçon", "Search lessons")} value={query} onChange={(e) => setQuery(e.target.value)} aria-label={t("Rechercher une leçon", "Search lessons")} />
        </label>
      </div>
      <div className="view" ref={scroller}>
        <div className="view-inner">
          <header className="page-head">
            <div>
              <h1>
                {greeting()}
                {name ? `, ${name}` : ""}.
              </h1>
              <p>
                {lessons
                  ? // les chapitres d'un livre ne comptent pas comme des leçons : le livre compte une fois
                    `${[
                      lessons.length - bookOf.size || !books.length ? count(lessons.length - bookOf.size, "leçon", "leçons", "lesson", "lessons") : "",
                      books.length ? count(books.length, "livre", "livres", "book", "books") : "",
                    ]
                      .filter(Boolean)
                      .join(" · ")} ${inLang(lang)}`
                  : "…"}{" "}
                · {count(known, "mot connu", "mots connus", "known word", "known words")}
              </p>
            </div>
            <button className="btn soft" onClick={() => openImport()}>
              <Icon name="import" size={16} /> {t("Importer du contenu", "Import content")}
            </button>
          </header>

          {resume && (
            <motion.div
              className="hero glow-surface"
              ref={hero.ref}
              onMouseMove={hero.onMouseMove}
              initial={{ opacity: 0, y: 12 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ type: "spring", stiffness: 220, damping: 26 }}
            >
              <Cover lesson={resume} big progress={progressOf(resume)} duration={resume.has_media ? resume.duration : 0} />
              <div className="hero-body">
                <span className="eyebrow">{resumeBook ? t(`Reprendre la lecture · ${resumeBook.p.name}`, `Continue reading · ${resumeBook.p.name}`) : t("Reprendre la lecture", "Continue reading")}</span>
                <h2 className="display">{resume.title}</h2>
                <p className="hero-excerpt">{resume.excerpt}…</p>
                <div className="hero-foot">
                  <button className="btn primary lg glow" onClick={() => openLesson(resume.id, resumeBook ? { playlist: resumeBook.p.id } : undefined)}>
                    <Icon name="book" size={17} /> {t("Continuer", "Continue")}
                  </button>
                  <span className="muted num">
                    {resume.has_media && resume.position > 1
                      ? t(
                          `Reprise à ${formatDuration(resume.position)}${resume.duration ? ` sur ${formatDuration(resume.duration)}` : ""}`,
                          `Resume at ${formatDuration(resume.position)}${resume.duration ? ` of ${formatDuration(resume.duration)}` : ""}`,
                        )
                      : t(
                          `Page ${Math.min(resume.page + 1, pagesOf(resume.word_count))} sur ${pagesOf(resume.word_count)}`,
                          `Page ${Math.min(resume.page + 1, pagesOf(resume.word_count))} of ${pagesOf(resume.word_count)}`,
                        )}
                    {resumeBook
                      ? t(
                          ` · chapitre ${resumeBook.ls.findIndex((l) => l.id === resume.id) + 1} sur ${resumeBook.ls.length}`,
                          ` · chapter ${resumeBook.ls.findIndex((l) => l.id === resume.id) + 1} of ${resumeBook.ls.length}`,
                        )
                      : ""}{" "}
                    · {count(resume.new_words, "mot nouveau", "mots nouveaux", "new word", "new words")}
                  </span>
                </div>
              </div>
            </motion.div>
          )}

          {lessons && lessons.length > 0 && (
            <div className="lib-toolbar">
              <Segmented
                id="lib-filter"
                label={t("Filtrer", "Filter")}
                value={filter}
                onChange={(v) => setFilter(v as Filter)}
                options={[
                  { value: "all", label: t("Tout", "All") },
                  { value: "text", label: t("Textes", "Texts") },
                  // sur iPhone, des libellés courts : les cinq tiennent sur la largeur
                  { value: "audio", label: compact ? t("Audio", "Audio") : t("Audio et vidéo", "Audio and video") },
                  { value: "book", label: t("Livres", "Books") },
                  { value: "done", label: compact ? t("Lues", "Read") : t("Terminées", "Finished") },
                ]}
              />
              <div className="lib-tools">
                <ViewSwitch value={view} onChange={(v) => void setSetting("library_view", v)} />
                <button className={`btn sm ${selecting ? "soft" : "ghost"} lib-select`} onClick={() => (selecting ? stopSelect() : startSelect())} aria-pressed={selecting}>
                  <Icon name="check" size={15} stroke={2.4} /> {selecting ? t("Terminé", "Done") : t("Sélectionner", "Select")}
                </button>
              </div>
            </div>
          )}

          {lessons && lessons.length === 0 && (
            <div className="empty">
              <Orb size={48} />
              <h3>{t("Votre bibliothèque attend sa première lumière", "Your library is waiting for its first light")}</h3>
              <p>
                {t(
                  "Importez un article, un livre, un podcast ou une vidéo, ou commencez par une courte histoire écrite pour vous.",
                  "Import an article, a book, a podcast or a video, or start with a short story written for you.",
                )}
              </p>
              <div className="empty-actions" style={{ display: "flex", gap: 10, marginTop: 8 }}>
                <button className="btn primary lg glow" onClick={addStarter}>
                  {t(`Lire « ${STARTERS[lang].title} »`, `Read “${STARTERS[lang].title}”`)}
                </button>
                <button className="btn outline lg" onClick={() => openImport()}>
                  <Icon name="import" size={16} /> {t("Importer", "Import")}
                </button>
              </div>
              <button className="btn ghost disc-invite" onClick={() => go("discover")}>
                <Icon name="sparkle" size={15} /> {t("Ou découvrez des vidéos, des podcasts et des articles à votre niveau", "Or discover videos, podcasts and articles at your level")}
              </button>
            </div>
          )}

          {/* d'une présentation à l'autre, un fondu ; puis les éléments se lèvent en cascade */}
          <AnimatePresence mode="wait" initial={false}>
            <motion.div key={view} className={`lib-shelf ${selecting ? "selecting" : ""}`} initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0, transition: { duration: 0.12 } }}>
              {view === "list" ? (
                shown.length > 0 && (
                  <LibraryList entries={page} sort={sort} onSort={onSort} query={query} selecting={selecting} picked={picked} onPick={pick} actions={actions} rise={rise} />
                )
              ) : (
                // « popLayout » : une carte qui s'en va quitte aussitôt la grille, les autres prennent sa place en glissant
                <div className={`lesson-grid ${view === "small" ? "small" : ""}`}>
                  <AnimatePresence mode="popLayout">
                    {page.map((e, i) => {
                      const props = {
                        small: view === "small",
                        rise: rise(i),
                        selecting,
                        picked: picked.has(e.key),
                        onPick: (ev: React.MouseEvent) => pick(e.key, ev.shiftKey),
                        actions: actions(e),
                      };
                      return e.b ? <BookCard key={e.key} b={e.b} {...props} /> : <LessonCard key={e.key} l={e.l} {...props} />;
                    })}
                  </AnimatePresence>
                </div>
              )}
            </motion.div>
          </AnimatePresence>
          {more > 0 && (
            <div className="lib-more">
              <button className="btn outline lg" onClick={showMore}>
                <Icon name="chevron" size={17} /> {t("Voir plus", "Show more")}
              </button>
              <span className="muted num">{count(more, "autre leçon", "autres leçons", "more lesson", "more lessons")}</span>
            </div>
          )}
          {lessons && lessons.length > 0 && shown.length === 0 && <p className="muted" style={{ padding: "40px 0", textAlign: "center" }}>{t("Aucune leçon ne correspond.", "No lesson matches.")}</p>}
        </div>
      </div>
      <ScrollTop target={scroller} />
      <div className="lib-selbar-wrap">
        <AnimatePresence>
          {selecting && (
            <motion.div
              className="lib-selbar"
              role="toolbar"
              aria-label={t("Leçons sélectionnées", "Selected lessons")}
              initial={{ opacity: 0, y: 22, scale: 0.96 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, y: 14, scale: 0.97, transition: { duration: 0.16, ease: "easeOut" } }}
              transition={{ type: "spring", stiffness: 380, damping: 30 }}
            >
              <span className="lib-selcount num" aria-live="polite">
                {picked.size
                  ? count(pickedLessons, "leçon sélectionnée", "leçons sélectionnées", "lesson selected", "lessons selected")
                  : t("Touchez les leçons à sélectionner", "Click the lessons to select")}
              </span>
              <button className="btn sm ghost" onClick={pickAll} disabled={!shown.length}>
                {allPicked ? t("Tout désélectionner", "Deselect all") : t("Tout sélectionner", "Select all")}
              </button>
              <button className="btn sm ghost" onClick={() => void markEntries(pickedEntries, !pickedRead)} disabled={!picked.size}>
                <Icon name={pickedRead ? "unread" : "read"} size={15} /> {pickedRead ? t("Marquer comme non lu", "Mark as unread") : t("Marquer comme lu", "Mark as read")}
              </button>
              <button className="btn sm danger" onClick={() => void removePicked()} disabled={!picked.size}>
                <Icon name="trash" size={15} /> {t("Supprimer", "Delete")}
              </button>
              <button className="icon-btn" onClick={stopSelect} aria-label={t("Terminer la sélection", "Done selecting")} title={t("Terminer la sélection (Échap)", "Done selecting (Esc)")}>
                <Icon name="close" size={16} />
              </button>
            </motion.div>
          )}
        </AnimatePresence>
      </div>
      <Sheet
        open={!!renaming}
        onClose={() => setRenaming(null)}
        title={renaming?.b ? t("Renommer le livre", "Rename the book") : t("Renommer la leçon", "Rename the lesson")}
        width={520}
        footer={
          <>
            <button className="btn ghost" onClick={() => setRenaming(null)}>
              {t("Annuler", "Cancel")}
            </button>
            <button className="btn primary" onClick={saveRename} disabled={!newTitle.trim()}>
              {t("Enregistrer", "Save")}
            </button>
          </>
        }
      >
        <input className="input" autoFocus value={newTitle} onChange={(e) => setNewTitle(e.target.value)} onKeyDown={(e) => e.key === "Enter" && saveRename()} aria-label={t("Titre", "Title")} />
      </Sheet>
      <AddToPlaylist lesson={adding} all={lessons ?? []} onClose={() => setAdding(null)} />
    </>
  );
}
