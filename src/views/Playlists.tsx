import { AnimatePresence, Reorder, motion, useDragControls } from "motion/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { BookCover, Cover, KIND, PlaylistCover } from "../components/Cover";
import { Icon } from "../components/Icon";
import { Menu, Orb, Segmented, Sheet, useGlow } from "../components/ui";
import { api, errorText } from "../lib/api";
import { confirmAsk } from "../lib/dialogs";
import { count, t } from "../lib/i18n";
import { inLang, langInfo } from "../lib/langs";
import { isWindows } from "../lib/platform";
import { bookView, formatLength, isBook, lessonsOf, listenSecs, markRead, playPlaylist, plural, progressOf, readBook } from "../lib/playlists";
import { formatDuration, useApp } from "../lib/store";
import type { LessonSummary, Playlist, PlaylistPatch } from "../lib/types";

const SPRING = { type: "spring", stiffness: 260, damping: 28 } as const;

/** Durée (audio, vidéo) ou longueur (texte) d'une leçon, en quelques caractères. */
function lengthOf(l: LessonSummary): string {
  return l.has_media && l.duration > 0 ? formatDuration(l.duration) : count(l.word_count, "mot", "mots", "word", "words");
}

/** `inBook` : chapitre d'un livre, son genre et son livre vont sans dire. */
function subtitleOf(l: LessonSummary, inBook = false): string {
  if (inBook) return lengthOf(l);
  const kind = (KIND[l.kind] ?? KIND.text).label;
  return [kind, lengthOf(l), l.collection].filter(Boolean).join(" · ");
}

export function Playlists() {
  const lang = useApp((s) => s.lang)();
  const version = useApp((s) => s.libraryVersion);
  const playlistId = useApp((s) => s.playlistId);
  const openPlaylist = useApp((s) => s.openPlaylist);
  const toast = useApp((s) => s.toast);
  const [all, setAll] = useState<LessonSummary[] | null>(null);
  const [lists, setLists] = useState<Playlist[] | null>(null);
  const [naming, setNaming] = useState(false);
  const [name, setName] = useState("");
  const [picker, setPicker] = useState<number | null>(null);

  const reload = useCallback(
    () =>
      Promise.all([api().lessonsList(lang), api().playlistsList(lang)])
        .then(([l, p]) => {
          setAll(l);
          setLists(p);
        })
        .catch((e) => toast(errorText(e), "error")),
    [lang, toast],
  );
  useEffect(() => {
    void reload();
  }, [reload, version]);

  const open = lists?.find((p) => p.id === playlistId) ?? null;
  // playlist supprimée ou d'une autre langue : retour à la liste
  useEffect(() => {
    if (lists && playlistId && !open) openPlaylist(null);
  }, [lists, playlistId, open, openPlaylist]);

  /** Modification affichée tout de suite, enregistrée ensuite. */
  const update = async (id: number, patch: PlaylistPatch) => {
    setLists(
      (ls) =>
        ls?.map((p) =>
          p.id === id
            ? {
                ...p,
                ...(patch.name !== undefined && { name: patch.name.trim() || p.name }),
                ...(patch.lessons && { lessons: patch.lessons }),
                ...(patch.looping !== undefined && { looping: patch.looping }),
              }
            : p,
        ) ?? ls,
    );
    try {
      await api().playlistUpdate(id, patch);
    } catch (e) {
      toast(errorText(e), "error");
      void reload();
    }
  };

  const create = async () => {
    try {
      const id = await api().playlistCreate(lang, name.trim(), []);
      setNaming(false);
      await reload();
      openPlaylist(id);
      // une playlist vide n'attend qu'une chose : ses premières leçons
      window.setTimeout(() => setPicker(id), 260);
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const remove = async (p: Playlist) => {
    const ok = await confirmAsk(
      isBook(p)
        ? t(
            `Défaire le livre « ${p.name} » ? Ses chapitres restent dans votre bibliothèque, chacun sur sa carte. Pour supprimer le livre et ses chapitres, passez par la bibliothèque.`,
            `Ungroup the book “${p.name}”? Its chapters stay in your library, each on its own card. To delete the book and its chapters, use the library.`,
          )
        : t(`Supprimer la playlist « ${p.name} » ? Ses leçons restent dans votre bibliothèque.`, `Delete the playlist “${p.name}”? Its lessons stay in your library.`),
      isBook(p) ? t("Défaire le livre", "Ungroup the book") : t("Supprimer la playlist", "Delete the playlist"),
      isBook(p) ? t("Défaire", "Ungroup") : t("Supprimer", "Delete"),
    );
    if (!ok) return;
    try {
      await api().playlistDelete(p.id);
      if (useApp.getState().queue === p.id) useApp.setState({ queue: null });
      openPlaylist(null);
      setLists((ls) => ls?.filter((x) => x.id !== p.id) ?? ls);
      toast(isBook(p) ? t("Livre défait : ses chapitres sont dans la bibliothèque", "Book ungrouped: its chapters are in the library") : t("Playlist supprimée", "Playlist deleted"));
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const startNaming = () => {
    setName("");
    setNaming(true);
  };

  const picking = lists?.find((p) => p.id === picker) ?? null;

  return (
    <>
      <AnimatePresence mode="wait" initial={false}>
        {open && all ? (
          <motion.div key={`pl-${open.id}`} className="pl-view" initial={{ opacity: 0, x: 18 }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0, x: 18 }} transition={{ duration: 0.22, ease: [0.2, 0.8, 0.2, 1] }}>
            <PlaylistPage p={open} all={all} onUpdate={(patch) => update(open.id, patch)} onDelete={() => remove(open)} onAdd={() => setPicker(open.id)} />
          </motion.div>
        ) : (
          <motion.div key="home" className="pl-view" initial={{ opacity: 0, x: -18 }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0, x: -18 }} transition={{ duration: 0.22, ease: [0.2, 0.8, 0.2, 1] }}>
            <PlaylistsHome lists={lists} all={all} onCreate={startNaming} onDelete={remove} />
          </motion.div>
        )}
      </AnimatePresence>

      <Sheet
        open={naming}
        onClose={() => setNaming(false)}
        title={t("Nouvelle playlist", "New playlist")}
        width={520}
        footer={
          <>
            <button className="btn ghost" onClick={() => setNaming(false)}>
              {t("Annuler", "Cancel")}
            </button>
            <button className="btn primary" onClick={create}>
              {t("Créer", "Create")}
            </button>
          </>
        }
      >
        <input
          className="input"
          autoFocus
          placeholder={t("Par exemple : Podcasts du matin", "For example: Morning podcasts")}
          value={name}
          maxLength={120}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && create()}
          aria-label={t("Nom de la playlist", "Playlist name")}
        />
      </Sheet>

      <LessonPicker
        p={picking}
        all={all ?? []}
        onClose={() => setPicker(null)}
        onChange={(lessons) => picking && update(picking.id, { lessons })}
      />
    </>
  );
}

/** Toute la playlist (ou tout le livre) lue, ou de nouveau à lire : elle repart du début. */
async function markPlaylist(p: Playlist, ls: LessonSummary[], read: boolean) {
  await api()
    .playlistUpdate(p.id, { current: 0 })
    .catch(() => {});
  const book = isBook(p);
  await markRead(
    ls.map((l) => l.id),
    read,
    read
      ? book
        ? t(`« ${p.name} » est marqué comme lu`, `“${p.name}” marked as read`)
        : t(`« ${p.name} » est marquée comme lue`, `“${p.name}” marked as read`)
      : book
        ? t(`« ${p.name} » est marqué comme non lu`, `“${p.name}” marked as unread`)
        : t(`« ${p.name} » est marquée comme non lue`, `“${p.name}” marked as unread`),
  );
}

// ---------- toutes les playlists ----------

function PlaylistsHome({ lists, all, onCreate, onDelete }: { lists: Playlist[] | null; all: LessonSummary[] | null; onCreate(): void; onDelete(p: Playlist): void }) {
  const lang = useApp((s) => s.lang)();
  return (
    <>
      <div className="titlebar drag" data-tauri-drag-region />
      <div className="view">
        <div className="view-inner">
          <header className="page-head">
            <div>
              <h1>Playlists</h1>
              <p>{lists && lists.length ? `${plural(lists.length, "playlist", "playlist")} ${inLang(lang)}` : t("Vos leçons, dans l'ordre qui vous plaît", "Your lessons, in the order you like")}</p>
            </div>
            {lists && lists.length > 0 && (
              <button className="btn soft" onClick={onCreate}>
                <Icon name="plus" size={16} /> {t("Nouvelle playlist", "New playlist")}
              </button>
            )}
          </header>

          {lists && all && lists.length === 0 && (
            <div className="empty">
              <Orb size={48} />
              <h3>{t("Des leçons qui s'enchaînent", "Lessons that flow into one another")}</h3>
              <p>
                {t(
                  "Rassemblez des leçons dans une playlist : à l'écoute, chacune laisse place à la suivante, comme une émission. Idéal pour une série de podcasts, un livre audio ou vos révisions du matin.",
                  "Gather lessons in a playlist: as you listen, each one gives way to the next, like a show. Perfect for a podcast series, an audiobook or your morning review.",
                )}
              </p>
              <button className="btn primary lg glow" onClick={onCreate} style={{ marginTop: 8 }}>
                <Icon name="plus" size={16} stroke={2} /> {t("Créer une playlist", "Create a playlist")}
              </button>
            </div>
          )}

          {lists && all && lists.length > 0 && (
            <motion.div className="pl-grid" layout>
              <AnimatePresence>
                {lists.map((p, i) => (
                  <PlaylistCard key={p.id} p={p} ls={lessonsOf(p, all)} index={i} onDelete={() => onDelete(p)} />
                ))}
              </AnimatePresence>
              <motion.button
                layout
                className="pl-new"
                onClick={onCreate}
                initial={{ opacity: 0, y: 14 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ ...SPRING, delay: Math.min(lists.length, 12) * 0.035 }}
              >
                <span className="pl-new-plus">
                  <Icon name="plus" size={20} stroke={1.8} />
                </span>
                {t("Nouvelle playlist", "New playlist")}
              </motion.button>
            </motion.div>
          )}
        </div>
      </div>
    </>
  );
}

function PlaylistCard({ p, ls, index, onDelete }: { p: Playlist; ls: LessonSummary[]; index: number; onDelete(): void }) {
  const openPlaylist = useApp((s) => s.openPlaylist);
  const playing = useApp((s) => s.queue === p.id && s.lessonId !== null);
  const glow = useGlow<HTMLDivElement>();
  const [menu, setMenu] = useState(false);
  const done = ls.filter((l) => l.completed).length;
  const allDone = ls.length > 0 && done === ls.length;
  const secs = listenSecs(ls);
  // un livre : ses chapitres se lisent (le son ne démarre pas de lui-même)
  const book = isBook(p) ? bookView(p, ls) : null;
  return (
    <motion.div
      layout
      initial={{ opacity: 0, y: 14 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, scale: 0.96 }}
      transition={{ ...SPRING, delay: Math.min(index, 12) * 0.035 }}
      className="lesson-card pl-card glow-surface"
      ref={glow.ref}
      onMouseMove={glow.onMouseMove}
    >
      {/* deux feuillets dépassent derrière la couverture : une pile de leçons */}
      <span className="pl-layer one" aria-hidden="true" />
      <span className="pl-layer two" aria-hidden="true" />
      <button className="lesson-hit" onClick={() => openPlaylist(p.id)} aria-label={t(`Ouvrir la playlist ${p.name}`, `Open the playlist ${p.name}`)} />
      <div className="pl-cover-wrap">
        {book ? <BookCover lesson={book.ls[0]} title={p.name} /> : <PlaylistCover lessons={ls} seed={p.id} />}
        {book ? (
          <button className="pl-play" onClick={() => readBook(book)} aria-label={t(`Lire ${p.name}`, `Read ${p.name}`)} title={book.started ? t("Reprendre la lecture", "Continue reading") : t("Commencer la lecture", "Start reading")}>
            <Icon name="book" size={18} />
          </button>
        ) : (
          ls.length > 0 && (
            <button className="pl-play" onClick={() => playPlaylist(p)} aria-label={t(`Écouter la playlist ${p.name}`, `Play the playlist ${p.name}`)}
              title={p.current ? t("Reprendre l'écoute", "Resume listening") : t("Écouter la playlist", "Play the playlist")}
            >
              <Icon name="play" size={18} />
            </button>
          )
        )}
      </div>
      <div className="lesson-body">
        <span className={`lesson-collection ${playing ? "pl-live" : ""}`}>{playing ? (book ? t("En cours de lecture", "Reading now") : t("En cours d'écoute", "Now playing")) : book ? t("Livre", "Book") : "Playlist"}</span>
        <h3 className="lesson-title">{p.name}</h3>
        <div className="lesson-meta">
          <span className="num">{book ? count(ls.length, "chapitre", "chapitres", "chapter", "chapters") : plural(ls.length, "leçon", "lesson")}</span>
          {secs > 0 && (
            <>
              <span className="sep" />
              <span className="num">{formatLength(secs)}</span>
            </>
          )}
          {p.looping && !book && (
            <>
              <span className="sep" />
              <span className="pl-looping" title={t("En boucle", "Looping")}>
                <Icon name="repeat" size={13} stroke={2} /> {t("En boucle", "Loop")}
              </span>
            </>
          )}
        </div>
        {ls.length > 0 && (
          <div className="lesson-known" title={t(`${done} leçon${done > 1 ? "s" : ""} terminée${done > 1 ? "s" : ""} sur ${ls.length}`, `${done} of ${ls.length} lessons finished`)}>
            <div className="bar">
              <i style={{ width: `${(done / ls.length) * 100}%` }} />
            </div>
            <span className="num">
              {done} / {ls.length}
            </span>
          </div>
        )}
      </div>
      <div className="lesson-menu">
        <Menu
          open={menu}
          onClose={() => setMenu(false)}
          align="right"
          anchor={
            <button className="icon-btn" onClick={() => setMenu((m) => !m)} aria-label={t("Options de la playlist", "Playlist options")}>
              <Icon name="more" size={18} stroke={2.6} />
            </button>
          }
        >
          <button className="menu-item" onClick={() => (setMenu(false), openPlaylist(p.id))}>
            <Icon name="playlist" size={16} /> {t("Ouvrir", "Open")}
          </button>
          {ls.length > 0 && (
            <button className="menu-item" onClick={() => (setMenu(false), void markPlaylist(p, ls, !allDone))}>
              <Icon name={allDone ? "unread" : "read"} size={16} /> {allDone ? t("Marquer comme non lu", "Mark as unread") : t("Marquer comme lu", "Mark as read")}
            </button>
          )}
          <div className="menu-sep" />
          <button className="menu-item danger" onClick={() => (setMenu(false), onDelete())}>
            <Icon name="trash" size={16} /> {book ? t("Défaire le livre", "Ungroup the book") : t("Supprimer la playlist", "Delete the playlist")}
          </button>
        </Menu>
      </div>
    </motion.div>
  );
}

// ---------- une playlist ----------

function PlaylistPage({ p, all, onUpdate, onDelete, onAdd }: { p: Playlist; all: LessonSummary[]; onUpdate(patch: PlaylistPatch): void; onDelete(): void; onAdd(): void }) {
  const openPlaylist = useApp((s) => s.openPlaylist);
  const openLesson = useApp((s) => s.openLesson);
  const bump = useApp((s) => s.bumpLibrary);
  const lang = useApp((s) => s.lang)();
  const toast = useApp((s) => s.toast);
  const ls = useMemo(() => lessonsOf(p, all), [p, all]);
  const byId = useMemo(() => new Map(ls.map((l) => [l.id, l])), [ls]);
  const [menu, setMenu] = useState(false);
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(p.name);

  // ordre affiché : suit le glisser, puis s'enregistre au lâcher
  const ids = useMemo(() => ls.map((l) => l.id), [ls]);
  const [order, setOrder] = useState(ids);
  const orderRef = useRef(order);
  orderRef.current = order;
  const idsKey = ids.join(",");
  useEffect(() => setOrder(idsKey ? idsKey.split(",").map(Number) : []), [idsKey]);
  const saveOrder = () => {
    if (orderRef.current.join(",") !== idsKey) onUpdate({ lessons: orderRef.current });
  };

  const done = ls.filter((l) => l.completed).length;
  const allDone = ls.length > 0 && done === ls.length;
  const secs = listenSecs(ls);
  const words = ls.reduce((s, l) => s + l.word_count, 0);
  // un livre : on y lit ses chapitres dans l'ordre, sans lancer le son
  const book = isBook(p) ? bookView(p, ls) : null;
  const resume = book ? (book.started ? book.at : undefined) : p.current ? byId.get(p.current) : undefined;

  const saveTitle = () => {
    setEditing(false);
    const name = title.trim();
    if (name && name !== p.name) onUpdate({ name });
    else setTitle(p.name);
  };

  /** Retire des leçons de la playlist (elles restent dans la bibliothèque). */
  const removeLessons = (gone: LessonSummary[]) => {
    if (!gone.length) return;
    const out = new Set(gone.map((l) => l.id));
    onUpdate({ lessons: p.lessons.filter((x) => !out.has(x)) });
    toast(
      gone.length === 1
        ? book
          ? t(`« ${gone[0].title} » a quitté le livre`, `“${gone[0].title}” left the book`)
          : t(`« ${gone[0].title} » a quitté la playlist`, `“${gone[0].title}” left the playlist`)
        : book
          ? t(`${count(gone.length, "leçon", "leçons", "", "")} ont quitté le livre`, `${count(gone.length, "", "", "lesson", "lessons")} left the book`)
          : t(`${count(gone.length, "leçon", "leçons", "", "")} ont quitté la playlist`, `${count(gone.length, "", "", "lesson", "lessons")} left the playlist`),
    );
  };

  /** Supprime des leçons de la bibliothèque, après confirmation (les mots appris restent). */
  const deleteLessons = async (gone: LessonSummary[]) => {
    if (!gone.length) return false;
    const ok = await confirmAsk(
      gone.length === 1
        ? t(`Supprimer « ${gone[0].title} » de votre bibliothèque ? Les mots appris sont conservés.`, `Delete “${gone[0].title}” from your library? The words you learned are kept.`)
        : t(
            `Supprimer ${count(gone.length, "leçon", "leçons", "", "")} de votre bibliothèque ? Elles quittent aussi les autres playlists. Les mots appris sont conservés.`,
            `Delete ${count(gone.length, "", "", "lesson", "lessons")} from your library? They also leave your other playlists. The words you learned are kept.`,
          ),
      gone.length === 1 ? t("Supprimer la leçon", "Delete the lesson") : t("Supprimer les leçons", "Delete the lessons"),
      t("Supprimer", "Delete"),
    );
    if (!ok) return false;
    const out = gone.map((l) => l.id);
    try {
      await api().lessonsDelete(out);
      // un livre dont il ne reste aucun chapitre n'a plus de raison d'être
      if (book && out.length >= ls.length) {
        await api().playlistDelete(p.id);
        if (useApp.getState().queue === p.id) useApp.setState({ queue: null });
        openPlaylist(null);
      }
    } catch (e) {
      toast(errorText(e), "error");
      bump();
      return false;
    }
    const app = useApp.getState();
    for (const id of out) app.forgetLesson(id);
    toast(count(out.length, "leçon supprimée", "leçons supprimées", "lesson deleted", "lessons deleted"));
    bump();
    return true;
  };

  // ---------- sélection de plusieurs leçons ----------
  const [selecting, setSelecting] = useState(false);
  const [picked, setPicked] = useState<Set<number>>(() => new Set());
  // dernière leçon touchée : Maj + clic coche tout l'intervalle jusqu'à elle
  const pickFrom = useRef<number | null>(null);
  const startSelect = (first?: number) => {
    setSelecting(true);
    setPicked(new Set(first === undefined ? [] : [first]));
    pickFrom.current = first ?? null;
  };
  const stopSelect = () => {
    setSelecting(false);
    setPicked(new Set());
    pickFrom.current = null;
  };
  const pick = (id: number, range: boolean) => {
    const fromId = pickFrom.current;
    pickFrom.current = id;
    setPicked((now) => {
      const next = new Set(now);
      const from = fromId === null ? -1 : order.indexOf(fromId);
      const to = order.indexOf(id);
      if (range && from >= 0 && to >= 0) {
        for (let i = Math.min(from, to); i <= Math.max(from, to); i++) next.add(order[i]);
      } else if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };
  // une leçon qui a quitté la playlist quitte aussi la sélection
  useEffect(() => {
    setPicked((now) => {
      const kept = [...now].filter((id) => byId.has(id));
      return kept.length === now.size ? now : new Set(kept);
    });
  }, [byId]);
  const pickedLessons = order.filter((id) => picked.has(id)).map((id) => byId.get(id)!).filter(Boolean);
  const allPicked = order.length > 0 && order.every((id) => picked.has(id));
  const pickedRead = pickedLessons.length > 0 && pickedLessons.every((l) => l.completed);
  const pickAll = () => setPicked(allPicked ? new Set() : new Set(order));
  const removePicked = () => {
    removeLessons(pickedLessons);
    stopSelect();
  };
  const deletePicked = async () => {
    if (await deleteLessons(pickedLessons)) stopSelect();
  };
  const markPicked = () => void markRead(pickedLessons.map((l) => l.id), !pickedRead);
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
        setPicked(new Set(order));
      } else if (!typing && (e.key === "Backspace" || e.key === "Delete") && picked.size) {
        // dans une playlist, la touche retire les leçons sans les supprimer
        e.preventDefault();
        removePicked();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const li = langInfo(lang);

  return (
    <>
      <div className="titlebar drag" data-tauri-drag-region>
        <button className="btn sm ghost no-drag pl-back" onClick={() => openPlaylist(null)}>
          <Icon name="back" size={15} /> Playlists
        </button>
      </div>
      <motion.div className="view" layoutScroll>
        {/* halo des couvertures derrière l'en-tête */}
        <div className="pl-ambient" aria-hidden="true">
          {book ? <BookCover lesson={book.ls[0]} title={p.name} bare /> : <PlaylistCover lessons={ls} seed={p.id} />}
        </div>
        <div className="view-inner">
          <header className="pl-hero">
            {book ? (
              <div className="pl-cover pl-big book-big">
                <BookCover lesson={book.ls[0]} title={p.name} progress={book.progress} label={count(ls.length, "chapitre", "chapitres", "chapter", "chapters")} editable also={book.ls.slice(1).map((l) => l.id)} />
              </div>
            ) : (
              <PlaylistCover lessons={ls} seed={p.id} big />
            )}
            <div className="pl-hero-body">
              <span className="eyebrow">
                {book ? t("Livre", "Book") : "Playlist"} · {li.name}
              </span>
              {editing ? (
                <input
                  className="pl-title-input display"
                  autoFocus
                  value={title}
                  maxLength={120}
                  onChange={(e) => setTitle(e.target.value)}
                  onBlur={saveTitle}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") e.currentTarget.blur();
                    if (e.key === "Escape") {
                      setTitle(p.name);
                      setEditing(false);
                    }
                  }}
                  aria-label={t("Nom de la playlist", "Playlist name")}
                />
              ) : (
                <h1 className="display pl-title" onDoubleClick={() => (setTitle(p.name), setEditing(true))} title={t("Double-cliquez pour renommer", "Double-click to rename")}>
                  {p.name}
                </h1>
              )}
              <p className="pl-meta num">
                {book ? count(ls.length, "chapitre", "chapitres", "chapter", "chapters") : plural(ls.length, "leçon", "lesson")}
                {secs > 0 && t(` · ${formatLength(secs)} d'écoute`, ` · ${formatLength(secs)} of listening`)}
                {words > 0 && ` · ${count(words, "mot", "mots", "word", "words")}`}
                {done > 0 && (book ? t(` · ${done} lu${done > 1 ? "s" : ""}`, ` · ${done} read`) : t(` · ${done} terminée${done > 1 ? "s" : ""}`, ` · ${done} finished`))}
                {book && book.progress > 0.005 && t(` · ${Math.round(book.progress * 100)} % du livre`, ` · ${Math.round(book.progress * 100)}% of the book`)}
              </p>
              <div className="pl-actions">
                {book ? (
                  <button className="btn primary lg glow" onClick={() => readBook(book)}>
                    <Icon name="book" size={16} /> {book.started ? t("Reprendre la lecture", "Continue reading") : t("Commencer la lecture", "Start reading")}
                  </button>
                ) : (
                  <>
                    <button className="btn primary lg glow" onClick={() => playPlaylist(p)} disabled={!ls.length}>
                      <Icon name="play" size={15} /> {resume ? t("Reprendre", "Resume") : t("Écouter", "Play")}
                    </button>
                    <button className="btn outline lg" onClick={onAdd}>
                      <Icon name="plus" size={16} /> {t("Ajouter des leçons", "Add lessons")}
                    </button>
                    <button
                      className={`btn lg pl-loop ${p.looping ? "soft on" : "outline"}`}
                      onClick={() => onUpdate({ looping: !p.looping })}
                      aria-pressed={p.looping}
                      title={
                        p.looping
                          ? t("En boucle : après la dernière leçon, l'écoute reprend à la première", "Looping: after the last lesson, listening starts again from the first")
                          : t("Lire la playlist en boucle", "Loop the playlist")
                      }
                    >
                      <Icon name="repeat" size={16} /> {t("En boucle", "Loop")}
                    </button>
                  </>
                )}
                {book && book.hasMedia && (
                  <button className="btn outline lg" onClick={() => playPlaylist(p, book.at.id)}>
                    <Icon name="play" size={15} /> {t("Écouter", "Play")}
                  </button>
                )}
                <Menu
                  open={menu}
                  onClose={() => setMenu(false)}
                  anchor={
                    <button className="icon-btn pl-more" onClick={() => setMenu((m) => !m)} aria-label={t("Options de la playlist", "Playlist options")}>
                      <Icon name="more" size={20} stroke={2.6} />
                    </button>
                  }
                >
                  <button className="menu-item" onClick={() => (setMenu(false), setTitle(p.name), setEditing(true))}>
                    <Icon name="edit" size={16} /> {t("Renommer", "Rename")}
                  </button>
                  {book && (
                    <button className="menu-item" onClick={() => (setMenu(false), onAdd())}>
                      <Icon name="plus" size={16} /> {t("Ajouter des leçons", "Add lessons")}
                    </button>
                  )}
                  {ls.length > 0 && (
                    <>
                      <button className="menu-item" onClick={() => (setMenu(false), void markPlaylist(p, ls, !allDone))}>
                        <Icon name={allDone ? "unread" : "read"} size={16} /> {allDone ? t("Tout marquer comme non lu", "Mark all as unread") : t("Tout marquer comme lu", "Mark all as read")}
                      </button>
                      <button className="menu-item" onClick={() => (setMenu(false), startSelect())}>
                        <Icon name="check" size={16} /> {t("Sélectionner des leçons", "Select lessons")}
                      </button>
                    </>
                  )}
                  <div className="menu-sep" />
                  <button className="menu-item danger" onClick={() => (setMenu(false), onDelete())}>
                    <Icon name="trash" size={16} /> {book ? t("Défaire le livre", "Ungroup the book") : t("Supprimer la playlist", "Delete the playlist")}
                  </button>
                </Menu>
              </div>
              {resume && <p className="pl-resume muted">{t(`Reprise à « ${resume.title} »`, `Resumes at “${resume.title}”`)}</p>}
            </div>
          </header>

          {ls.length === 0 ? (
            <div className="pl-empty">
              <p>{t("Cette playlist attend ses premières leçons.", "This playlist is waiting for its first lessons.")}</p>
              <button className="btn soft" onClick={onAdd}>
                <Icon name="plus" size={15} /> {t("Ajouter des leçons", "Add lessons")}
              </button>
            </div>
          ) : (
            <>
              <div className="pl-listbar">
                <button className={`btn sm ${selecting ? "soft" : "ghost"}`} onClick={() => (selecting ? stopSelect() : startSelect())} aria-pressed={selecting}>
                  <Icon name="check" size={15} stroke={2.4} /> {selecting ? t("Terminé", "Done") : t("Sélectionner", "Select")}
                </button>
              </div>
              <Reorder.Group as="ol" axis="y" values={order} onReorder={setOrder} className={`pl-list ${selecting ? "selecting" : ""}`}>
                <AnimatePresence initial={false}>
                  {order.map((id, i) => {
                    const l = byId.get(id);
                    return l ? (
                      <Row
                        key={id}
                        l={l}
                        index={i}
                        current={book ? !!resume && id === resume.id : id === p.current}
                        book={!!book}
                        selecting={selecting}
                        picked={picked.has(id)}
                        onPick={(ev) => pick(id, ev.shiftKey)}
                        onOpen={() => openLesson(id, { playlist: p.id })}
                        onPlay={() => (book ? readBook(book, id) : playPlaylist(p, id))}
                        onMark={() => void markRead([id], !l.completed)}
                        onRemove={() => removeLessons([l])}
                        onSelect={() => startSelect(id)}
                        onDelete={() => void deleteLessons([l])}
                        onDrop={saveOrder}
                      />
                    ) : null;
                  })}
                </AnimatePresence>
              </Reorder.Group>
            </>
          )}
        </div>
      </motion.div>
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
                  ? count(picked.size, "leçon sélectionnée", "leçons sélectionnées", "lesson selected", "lessons selected")
                  : t("Touchez les leçons à sélectionner", "Click the lessons to select")}
              </span>
              <button className="btn sm ghost" onClick={pickAll}>
                {allPicked ? t("Tout désélectionner", "Deselect all") : t("Tout sélectionner", "Select all")}
              </button>
              <button className="btn sm ghost" onClick={markPicked} disabled={!picked.size}>
                <Icon name={pickedRead ? "unread" : "read"} size={15} /> {pickedRead ? t("Marquer comme non lu", "Mark as unread") : t("Marquer comme lu", "Mark as read")}
              </button>
              <button
                className="btn sm ghost"
                onClick={removePicked}
                disabled={!picked.size}
                title={
                  book
                    ? t("Retirer du livre (les leçons restent dans la bibliothèque)", "Remove from the book (the lessons stay in the library)")
                    : t("Retirer de la playlist (les leçons restent dans la bibliothèque)", "Remove from the playlist (the lessons stay in the library)")
                }
              >
                <Icon name="close" size={15} /> {t("Retirer", "Remove")}
              </button>
              <button className="btn sm danger" onClick={() => void deletePicked()} disabled={!picked.size} title={t("Supprimer de la bibliothèque", "Delete from the library")}>
                <Icon name="trash" size={15} /> {t("Supprimer", "Delete")}
              </button>
              <button className="icon-btn" onClick={stopSelect} aria-label={t("Terminer la sélection", "Done selecting")} title={t("Terminer la sélection (Échap)", "Done selecting (Esc)")}>
                <Icon name="close" size={16} />
              </button>
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    </>
  );
}

function Row({
  l,
  index,
  current,
  book,
  selecting,
  picked,
  onPick,
  onOpen,
  onPlay,
  onMark,
  onRemove,
  onSelect,
  onDelete,
  onDrop,
}: {
  l: LessonSummary;
  index: number;
  current: boolean;
  /** chapitre d'un livre : on le lit, on ne l'écoute pas */
  book: boolean;
  /** sélection de plusieurs leçons : toucher la ligne la coche */
  selecting: boolean;
  picked: boolean;
  onPick(ev: React.MouseEvent): void;
  onOpen(): void;
  onPlay(): void;
  /** lue ou de nouveau à lire */
  onMark(): void;
  onRemove(): void;
  onSelect(): void;
  onDelete(): void;
  onDrop(): void;
}) {
  const controls = useDragControls();
  const [dragging, setDragging] = useState(false);
  const [menu, setMenu] = useState(false);
  const progress = progressOf(l);
  const run = (fn: () => void) => () => {
    setMenu(false);
    fn();
  };
  return (
    <Reorder.Item
      as="li"
      value={l.id}
      dragListener={false}
      dragControls={controls}
      className={`pl-row ${current ? "current" : ""} ${dragging ? "dragging" : ""} ${picked ? "picked" : ""} ${menu ? "menu-open" : ""}`}
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0, transition: { ...SPRING, delay: Math.min(index, 14) * 0.025 } }}
      exit={{ opacity: 0, x: -24, transition: { duration: 0.2 } }}
      onDragStart={() => setDragging(true)}
      onDragEnd={() => {
        setDragging(false);
        onDrop();
      }}
      onContextMenu={(e: React.MouseEvent) => {
        if (selecting) return;
        e.preventDefault();
        setMenu(true);
      }}
    >
      {selecting ? (
        <span className="pl-grip" aria-hidden="true" />
      ) : (
        <button
          className="pl-grip"
          onPointerDown={(e) => {
            e.preventDefault();
            controls.start(e);
          }}
          aria-label={t("Déplacer la leçon", "Move the lesson")}
          title={t("Glisser pour changer l'ordre", "Drag to change the order")}
        >
          <Icon name="grip" size={16} stroke={2.8} />
        </button>
      )}
      <span className="pl-index num">
        {selecting ? (
          <span className={`lib-check ${picked ? "on" : ""}`} aria-hidden="true">
            <Icon name="check" size={11} stroke={3} />
          </span>
        ) : current ? (
          <Orb size={10} />
        ) : (
          index + 1
        )}
      </span>
      {selecting ? (
        <button className="pl-row-hit" onClick={onPick} aria-pressed={picked} aria-label={t(`Sélectionner ${l.title}`, `Select ${l.title}`)} />
      ) : (
        <button className="pl-row-hit" onClick={onOpen} aria-label={t(`Ouvrir ${l.title}`, `Open ${l.title}`)} />
      )}
      <div className="pl-thumb">
        <Cover lesson={l} bare />
        {!selecting && (
          <button
            className="pl-thumb-play"
            onClick={onPlay}
            aria-label={book ? t(`Lire à partir de ${l.title}`, `Read from ${l.title}`) : t(`Écouter à partir de ${l.title}`, `Play from ${l.title}`)}
            title={book ? t("Lire le livre à partir d'ici", "Read the book from here") : t("Écouter la playlist à partir d'ici", "Play the playlist from here")}
          >
            <Icon name={book ? "book" : "play"} size={14} />
          </button>
        )}
      </div>
      <div className="pl-row-body">
        <strong>{l.title}</strong>
        <span>{current ? (book ? t(`Reprise ici · ${subtitleOf(l, true)}`, `Resumes here · ${subtitleOf(l, true)}`) : t(`En cours · ${subtitleOf(l)}`, `Now playing · ${subtitleOf(l)}`)) : subtitleOf(l, book)}</span>
      </div>
      <div className="pl-row-end">
        {l.completed ? (
          <span className="done-pill">
            <Icon name="check" size={12} stroke={2.4} /> {t("Lu", "Read")}
          </span>
        ) : (
          progress > 0.005 && (
            <span className="pl-row-progress" title={t(`${Math.round(progress * 100)} %`, `${Math.round(progress * 100)}%`)}>
              <i style={{ width: `${progress * 100}%` }} />
            </span>
          )
        )}
        {!selecting && (
          <Menu
            open={menu}
            onClose={() => setMenu(false)}
            align="right"
            anchor={
              <button className="icon-btn pl-row-more" onClick={() => setMenu((m) => !m)} aria-label={t("Options de la leçon", "Lesson options")}>
                <Icon name="more" size={17} stroke={2.6} />
              </button>
            }
          >
            <button className="menu-item" onClick={run(onMark)}>
              <Icon name={l.completed ? "unread" : "read"} size={16} /> {l.completed ? t("Marquer comme non lu", "Mark as unread") : t("Marquer comme lu", "Mark as read")}
            </button>
            <button className="menu-item" onClick={run(onRemove)}>
              <Icon name="close" size={16} /> {book ? t("Retirer du livre", "Remove from the book") : t("Retirer de la playlist", "Remove from the playlist")}
            </button>
            <button className="menu-item" onClick={run(onSelect)}>
              <Icon name="check" size={16} /> {t("Sélectionner plusieurs leçons", "Select several lessons")}
            </button>
            <div className="menu-sep" />
            <button className="menu-item danger" onClick={run(onDelete)}>
              <Icon name="trash" size={16} /> {t("Supprimer de la bibliothèque", "Delete from the library")}
            </button>
          </Menu>
        )}
      </div>
    </Reorder.Item>
  );
}

// ---------- ajouter des leçons ----------

function LessonPicker({ p, all, onClose, onChange }: { p: Playlist | null; all: LessonSummary[]; onClose(): void; onChange(lessons: number[]): void }) {
  const [q, setQ] = useState("");
  const [filter, setFilter] = useState<"all" | "media" | "todo">("all");
  // la feuille garde son contenu pendant qu'elle se referme
  const [shown, setShown] = useState<Playlist | null>(p);
  const startCount = useRef(0);
  const wasOpen = useRef(false);
  useEffect(() => {
    // à chaque ouverture : recherche vide, et le compte des ajouts repart de zéro
    if (p && !wasOpen.current) {
      setQ("");
      setFilter("all");
      startCount.current = p.lessons.length;
    }
    wasOpen.current = !!p;
    if (p) setShown(p);
  }, [p]);

  const list = useMemo(() => {
    const s = q.trim().toLowerCase();
    return all.filter((l) => {
      if (s && !(l.title.toLowerCase().includes(s) || l.collection.toLowerCase().includes(s))) return false;
      if (filter === "media") return l.has_media;
      if (filter === "todo") return !l.completed;
      return true;
    });
  }, [all, q, filter]);

  if (!shown) return null;
  const inList = new Set(shown.lessons);
  const added = shown.lessons.length - startCount.current;
  const toggle = (id: number) => onChange(inList.has(id) ? shown.lessons.filter((x) => x !== id) : [...shown.lessons, id]);

  return (
    <Sheet
      open={!!p}
      onClose={onClose}
      title={t("Ajouter des leçons", "Add lessons")}
      width={660}
      footer={
        <>
          <span className="muted pick-count num">
            {added > 0
              ? t(`${plural(added, "leçon", "lesson")} ajoutée${added > 1 ? "s" : ""} à « ${shown.name} »`, `${plural(added, "leçon", "lesson")} added to “${shown.name}”`)
              : t(`Dans « ${shown.name} » : ${plural(shown.lessons.length, "leçon", "lesson")}`, `In “${shown.name}”: ${plural(shown.lessons.length, "leçon", "lesson")}`)}
          </span>
          <button className="btn primary" onClick={onClose}>
            {t("Terminé", "Done")}
          </button>
        </>
      }
    >
      <div className="pick-tools">
        <label className="search">
          <Icon name="search" size={16} />
          <input autoFocus placeholder={t("Rechercher une leçon", "Search lessons")} value={q} onChange={(e) => setQ(e.target.value)} aria-label={t("Rechercher une leçon", "Search lessons")} />
        </label>
        <Segmented
          id="pick-filter"
          label={t("Filtrer", "Filter")}
          value={filter}
          onChange={(v) => setFilter(v as typeof filter)}
          options={[
            { value: "all", label: t("Toutes", "All") },
            { value: "media", label: t("Audio et vidéo", "Audio and video") },
            { value: "todo", label: t("À lire", "To read") },
          ]}
        />
      </div>
      <div className="pick-list">
        {list.map((l) => {
          const on = inList.has(l.id);
          return (
            <button key={l.id} className={`pick-row ${on ? "on" : ""}`} onClick={() => toggle(l.id)} aria-pressed={on}>
              <span className="pick-thumb">
                <Cover lesson={l} bare />
              </span>
              <span className="pick-body">
                <strong>{l.title}</strong>
                <span>{subtitleOf(l)}</span>
              </span>
              <span className="pick-check" aria-hidden="true">
                <Icon name={on ? "check" : "plus"} size={15} stroke={2.2} />
              </span>
            </button>
          );
        })}
        {list.length === 0 && <p className="muted pick-none">{all.length ? t("Aucune leçon ne correspond.", "No lesson matches.") : t("Votre bibliothèque est encore vide : importez d'abord des leçons.", "Your library is still empty: import some lessons first.")}</p>}
      </div>
    </Sheet>
  );
}
