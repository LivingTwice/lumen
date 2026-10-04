import { AnimatePresence, motion } from "motion/react";
import { useEffect, useMemo, useState } from "react";
import { AddToPlaylist } from "../components/AddToPlaylist";
import { Cover } from "../components/Cover";
import { Icon } from "../components/Icon";
import { Menu, Orb, Segmented, Sheet, useGlow } from "../components/ui";
import { api, errorText } from "../lib/api";
import { confirmAsk } from "../lib/dialogs";
import { count, isEn, t } from "../lib/i18n";
import { STARTERS, inLang, starterCollection } from "../lib/langs";
import { formatDuration, formatNumber, useApp } from "../lib/store";
import type { LessonSummary } from "../lib/types";
import { Discover, DiscoverTab } from "./Discover";

type Filter = "all" | "text" | "audio" | "book" | "done" | "discover";

function greeting() {
  const h = new Date().getHours();
  if (isEn()) return h >= 5 && h < 12 ? "Good morning" : h >= 12 && h < 18 ? "Good afternoon" : "Good evening";
  if (h < 5) return "Bonne nuit";
  if (h < 18) return "Bonjour";
  return "Bonsoir";
}

export function pagesOf(words: number) {
  return Math.max(1, Math.round(words / 230));
}

/** Avancement (0 à 1) : à la seconde près pour l'audio et la vidéo, à la page près sinon. */
export function progressOf(l: LessonSummary): number {
  if (l.completed) return 1;
  if (l.has_media && l.duration > 0) return Math.min(1, l.position / l.duration);
  return l.page / pagesOf(l.word_count);
}

// Les cartes glissent vers leur nouvelle place (recherche, filtre, nouvelle leçon)
// sans attendre leur tour : seule l'apparition est décalée de l'une à l'autre.
const SLIDE = { type: "spring", stiffness: 420, damping: 40, mass: 0.8 } as const;

function LessonCard({
  l,
  index,
  intro,
  onDelete,
  onRename,
  onPlaylist,
  ref,
}: {
  l: LessonSummary;
  index: number;
  /** première apparition de la bibliothèque : les cartes se lèvent l'une après l'autre */
  intro: boolean;
  onDelete(): void;
  onRename(): void;
  onPlaylist(): void;
  ref?: React.Ref<HTMLDivElement>;
}) {
  const openLesson = useApp((s) => s.openLesson);
  const glow = useGlow<HTMLDivElement>();
  const [menu, setMenu] = useState(false);
  return (
    // l'enveloppe porte les mouvements de motion, la carte garde son survol en CSS :
    // deux animations sur la même propriété transform se contrarieraient
    <motion.div
      ref={ref}
      layout="position"
      className="lesson-cell"
      initial={intro ? { opacity: 0, y: 14 } : { opacity: 0, scale: 0.97 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, scale: 0.97, transition: { duration: 0.16, ease: "easeOut" } }}
      transition={{
        default: intro ? { delay: Math.min(index, 12) * 0.035, type: "spring", stiffness: 260, damping: 28 } : { duration: 0.22, ease: [0.2, 0.8, 0.2, 1] },
        layout: SLIDE,
      }}
    >
      <div className="lesson-card glow-surface" ref={glow.ref} onMouseMove={glow.onMouseMove}>
        <button className="lesson-hit" onClick={() => openLesson(l.id)} aria-label={t(`Ouvrir ${l.title}`, `Open ${l.title}`)} />
        <Cover lesson={l} progress={progressOf(l)} editable />
        <div className="lesson-body">
          {l.collection && <span className="lesson-collection">{l.collection}</span>}
          <h3 className="lesson-title">{l.title}</h3>
          <p className="lesson-excerpt">{l.excerpt}</p>
          <div className="lesson-meta">
            <span className="num">{count(l.word_count, "mot", "mots", "word", "words")}</span>
            <span className="sep" />
            <span className="num new-dot">{formatNumber(l.new_words)} {t("nouveaux", "new")}</span>
            {l.completed && (
              <span className="done-pill">
                <Icon name="check" size={12} stroke={2.4} /> {t("Lu", "Read")}
              </span>
            )}
          </div>
          <div className="lesson-known" title={t(`${l.known_pct} % des mots déjà rencontrés`, `${l.known_pct}% of the words already met`)}>
            <div className="bar">
              <i style={{ width: `${l.known_pct}%` }} />
            </div>
            <span className="num">{t(`${l.known_pct} %`, `${l.known_pct}%`)}</span>
          </div>
        </div>
        <div className="lesson-menu">
          <Menu
            open={menu}
            onClose={() => setMenu(false)}
            align="right"
            anchor={
              <button className="icon-btn" onClick={() => setMenu((m) => !m)} aria-label={t("Options de la leçon", "Lesson options")}>
                <Icon name="more" size={18} stroke={2.6} />
              </button>
            }
          >
            <button className="menu-item" onClick={() => (setMenu(false), onPlaylist())}>
              <Icon name="playlist" size={16} /> {t("Ajouter à une playlist…", "Add to a playlist…")}
            </button>
            <button className="menu-item" onClick={() => (setMenu(false), onRename())}>
              <Icon name="edit" size={16} /> {t("Renommer", "Rename")}
            </button>
            <div className="menu-sep" />
            <button className="menu-item danger" onClick={() => (setMenu(false), onDelete())}>
              <Icon name="trash" size={16} /> {t("Supprimer", "Delete")}
            </button>
          </Menu>
        </div>
      </div>
    </motion.div>
  );
}

export function Library() {
  const lang = useApp((s) => s.lang)();
  const langSetting = useApp((s) => s.settings.lang);
  const version = useApp((s) => s.libraryVersion);
  const bump = useApp((s) => s.bumpLibrary);
  const openImport = useApp((s) => s.openImport);
  const openLesson = useApp((s) => s.openLesson);
  const toast = useApp((s) => s.toast);
  const known = useApp((s) => s.knownCount);
  const [lessons, setLessons] = useState<LessonSummary[] | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const discovering = filter === "discover";
  const [query, setQuery] = useState("");
  const hero = useGlow<HTMLDivElement>();
  // l'entrée en cascade ne vaut que pour la première apparition des leçons ;
  // ensuite (recherche, filtre), les cartes apparaissent et glissent sans attendre
  const [intro, setIntro] = useState(true);
  useEffect(() => {
    if (!lessons || !intro) return;
    const timer = window.setTimeout(() => setIntro(false), 900);
    return () => window.clearTimeout(timer);
  }, [lessons, intro]);

  useEffect(() => {
    api()
      .lessonsList(lang)
      .then(setLessons)
      .catch((e) => toast(errorText(e), "error"));
  }, [lang, langSetting, version, toast]);

  const resume = useMemo(() => lessons?.find((l) => l.opened_at && !l.completed), [lessons]);

  const shown = useMemo(() => {
    if (!lessons) return [];
    const q = query.trim().toLowerCase();
    return lessons.filter((l) => {
      if (q && !(l.title.toLowerCase().includes(q) || l.collection.toLowerCase().includes(q))) return false;
      if (filter === "text") return ["text", "web", "pdf", "subtitles", "simplified"].includes(l.kind);
      if (filter === "audio") return l.has_media;
      if (filter === "book") return l.kind === "book";
      if (filter === "done") return l.completed;
      return true;
    });
  }, [lessons, filter, query]);

  const addStarter = async () => {
    const s = STARTERS[lang];
    const id = await api().lessonCreate({ lang, title: s.title, text: s.text, collection: starterCollection(), kind: "text" });
    bump();
    openLesson(id);
  };

  const remove = async (l: LessonSummary) => {
    const ok = await confirmAsk(
      t(`Supprimer « ${l.title} » ? Les mots appris sont conservés.`, `Delete “${l.title}”? The words you learned are kept.`),
      t("Supprimer la leçon", "Delete the lesson"),
      t("Supprimer", "Delete"),
    );
    if (!ok) return;
    await api().lessonDelete(l.id);
    useApp.getState().forgetLesson(l.id);
    toast(t("Leçon supprimée", "Lesson deleted"));
    bump();
  };

  const [adding, setAdding] = useState<LessonSummary | null>(null);
  const [renaming, setRenaming] = useState<LessonSummary | null>(null);
  const [newTitle, setNewTitle] = useState("");
  const rename = (l: LessonSummary) => {
    setNewTitle(l.title);
    setRenaming(l);
  };
  const saveRename = async () => {
    if (!renaming || !newTitle.trim()) return;
    await api().lessonUpdate(renaming.id, { title: newTitle.trim() });
    setRenaming(null);
    bump();
  };


  return (
    <>
      <div className="titlebar drag" data-tauri-drag-region>
        <div style={{ flex: 1 }} data-tauri-drag-region />
        <label className="search no-drag">
          <Icon name="search" size={16} />
          <input
            placeholder={discovering ? t("Rechercher dans Découvrir", "Search Discover") : t("Rechercher une leçon", "Search lessons")}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            aria-label={discovering ? t("Rechercher dans Découvrir", "Search Discover") : t("Rechercher une leçon", "Search lessons")}
          />
        </label>
      </div>
      <div className="view">
        <div className="view-inner">
          <header className="page-head">
            <div>
              <h1>
                {greeting()}.
              </h1>
              <p>
                {lessons ? `${count(lessons.length, "leçon", "leçons", "lesson", "lessons")} ${inLang(lang)}` : "…"} · {count(known, "mot connu", "mots connus", "known word", "known words")}
              </p>
            </div>
            <button className="btn soft" onClick={() => openImport()}>
              <Icon name="import" size={16} /> {t("Importer du contenu", "Import content")}
            </button>
          </header>

          {resume && !discovering && (
            <motion.div
              className="hero glow-surface"
              ref={hero.ref}
              onMouseMove={hero.onMouseMove}
              initial={{ opacity: 0, y: 12 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ type: "spring", stiffness: 220, damping: 26 }}
            >
              <Cover lesson={resume} big progress={progressOf(resume)} />
              <div className="hero-body">
                <span className="eyebrow">{t("Reprendre la lecture", "Continue reading")}</span>
                <h2 className="display">{resume.title}</h2>
                <p className="hero-excerpt">{resume.excerpt}…</p>
                <div className="hero-foot">
                  <button className="btn primary lg glow" onClick={() => openLesson(resume.id)}>
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
                        )}{" "}
                    · {count(resume.new_words, "mot nouveau", "mots nouveaux", "new word", "new words")}
                  </span>
                </div>
              </div>
            </motion.div>
          )}

          {lessons && (lessons.length > 0 || discovering) && (
            <div className="lib-toolbar">
              <Segmented
                id="lib-filter"
                label={t("Filtrer", "Filter")}
                value={filter}
                onChange={(v) => setFilter(v as Filter)}
                options={[
                  { value: "all", label: t("Tout", "All") },
                  { value: "text", label: t("Textes", "Texts") },
                  { value: "audio", label: t("Audio et vidéo", "Audio and video") },
                  { value: "book", label: t("Livres", "Books") },
                  { value: "done", label: t("Terminées", "Finished") },
                  { value: "discover", label: <DiscoverTab lang={lang} /> },
                ]}
              />
            </div>
          )}

          {discovering && <Discover lang={lang} query={query} />}

          {lessons && lessons.length === 0 && !discovering && (
            <div className="empty">
              <Orb size={48} />
              <h3>{t("Votre bibliothèque attend sa première lumière", "Your library is waiting for its first light")}</h3>
              <p>
                {t(
                  "Importez un article, un livre, un podcast ou une vidéo, ou commencez par une courte histoire écrite pour vous.",
                  "Import an article, a book, a podcast or a video, or start with a short story written for you.",
                )}
              </p>
              <div style={{ display: "flex", gap: 10, marginTop: 8 }}>
                <button className="btn primary lg glow" onClick={addStarter}>
                  {t(`Lire « ${STARTERS[lang].title} »`, `Read “${STARTERS[lang].title}”`)}
                </button>
                <button className="btn outline lg" onClick={() => openImport()}>
                  <Icon name="import" size={16} /> {t("Importer", "Import")}
                </button>
              </div>
              <button className="btn ghost disc-invite" onClick={() => setFilter("discover")}>
                <Icon name="sparkle" size={15} /> {t("Ou découvrez des vidéos, des podcasts et des articles à votre niveau", "Or discover videos, podcasts and articles at your level")}
              </button>
            </div>
          )}

          {/* « popLayout » : une carte qui s'en va quitte aussitôt la grille, les autres prennent sa place en glissant */}
          {!discovering && (
            <div className="lesson-grid">
              <AnimatePresence mode="popLayout">
                {shown.map((l, i) => (
                  <LessonCard key={l.id} l={l} index={i} intro={intro} onDelete={() => remove(l)} onRename={() => rename(l)} onPlaylist={() => setAdding(l)} />
                ))}
              </AnimatePresence>
            </div>
          )}
          {!discovering && lessons && lessons.length > 0 && shown.length === 0 && <p className="muted" style={{ padding: "40px 0", textAlign: "center" }}>{t("Aucune leçon ne correspond.", "No lesson matches.")}</p>}
        </div>
      </div>
      <Sheet
        open={!!renaming}
        onClose={() => setRenaming(null)}
        title={t("Renommer la leçon", "Rename the lesson")}
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
