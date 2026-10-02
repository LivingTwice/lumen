import { AnimatePresence, motion } from "motion/react";
import { useEffect, useMemo, useState } from "react";
import { Icon, type IconName } from "../components/Icon";
import { Menu, Orb, Segmented, Sheet, useGlow } from "../components/ui";
import { api, errorText } from "../lib/api";
import { confirmAsk } from "../lib/dialogs";
import { STARTERS, langInfo } from "../lib/langs";
import { formatNumber, useApp } from "../lib/store";
import type { LessonSummary } from "../lib/types";

type Filter = "all" | "text" | "audio" | "book" | "done";

const KIND: Record<string, { label: string; icon: IconName }> = {
  text: { label: "Texte", icon: "text" },
  web: { label: "Article", icon: "globe" },
  book: { label: "Livre", icon: "book" },
  pdf: { label: "PDF", icon: "file" },
  subtitles: { label: "Sous-titres", icon: "text" },
  audio: { label: "Audio", icon: "wave" },
  video: { label: "Vidéo", icon: "video" },
  simplified: { label: "Simplifié", icon: "sparkle" },
};

function greeting() {
  const h = new Date().getHours();
  if (h < 5) return "Bonne nuit";
  if (h < 18) return "Bonjour";
  return "Bonsoir";
}

export function pagesOf(words: number) {
  return Math.max(1, Math.round(words / 230));
}

export function Cover({ hue, kind, big = false }: { hue: number; kind: string; big?: boolean }) {
  const k = KIND[kind] ?? KIND.text;
  // position de la « source de lumière » dérivée de la teinte
  const x = 25 + ((hue * 7) % 50);
  const y = 20 + ((hue * 13) % 45);
  return (
    <div
      className={`cover ${big ? "big" : ""}`}
      style={{
        background: `radial-gradient(120% 90% at ${x}% ${y}%, hsl(${(hue + 30) % 360} 85% 78% / .95), transparent 55%),
          linear-gradient(150deg, hsl(${hue} 55% 42%), hsl(${(hue + 40) % 360} 60% 22%))`,
      }}
    >
      <span className="cover-sun" style={{ left: `${x}%`, top: `${y}%` }} />
      <span className="cover-kind">
        <Icon name={k.icon} size={14} />
        {k.label}
      </span>
    </div>
  );
}

function LessonCard({ l, index, onDelete, onRename }: { l: LessonSummary; index: number; onDelete(): void; onRename(): void }) {
  const openLesson = useApp((s) => s.openLesson);
  const glow = useGlow<HTMLDivElement>();
  const [menu, setMenu] = useState(false);
  const pages = pagesOf(l.word_count);
  const progress = l.completed ? 100 : Math.round((l.page / pages) * 100);
  return (
    <motion.div
      layout
      initial={{ opacity: 0, y: 14 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, scale: 0.96 }}
      transition={{ delay: Math.min(index, 12) * 0.035, type: "spring", stiffness: 260, damping: 28 }}
      className="lesson-card glow-surface"
      ref={glow.ref}
      onMouseMove={glow.onMouseMove}
    >
      <button className="lesson-hit" onClick={() => openLesson(l.id)} aria-label={`Ouvrir ${l.title}`} />
      <Cover hue={l.hue} kind={l.kind} />
      <div className="lesson-body">
        {l.collection && <span className="lesson-collection">{l.collection}</span>}
        <h3 className="lesson-title">{l.title}</h3>
        <p className="lesson-excerpt">{l.excerpt}</p>
        <div className="lesson-meta">
          <span className="num">{formatNumber(l.word_count)} mots</span>
          <span className="sep" />
          <span className="num new-dot">{formatNumber(l.new_words)} nouveaux</span>
          {l.completed && (
            <span className="done-pill">
              <Icon name="check" size={12} stroke={2.4} /> Lu
            </span>
          )}
        </div>
        <div className="lesson-known" title={`${l.known_pct} % des mots déjà rencontrés`}>
          <div className="bar">
            <i style={{ width: `${l.known_pct}%` }} />
          </div>
          <span className="num">{l.known_pct} %</span>
        </div>
        {progress > 0 && progress < 100 && <div className="lesson-progress" style={{ width: `${progress}%` }} />}
      </div>
      <div className="lesson-menu">
        <Menu
          open={menu}
          onClose={() => setMenu(false)}
          align="right"
          anchor={
            <button className="icon-btn" onClick={() => setMenu((m) => !m)} aria-label="Options de la leçon">
              <Icon name="more" size={18} stroke={2.6} />
            </button>
          }
        >
          <button className="menu-item" onClick={() => (setMenu(false), onRename())}>
            <Icon name="edit" size={16} /> Renommer
          </button>
          <div className="menu-sep" />
          <button className="menu-item danger" onClick={() => (setMenu(false), onDelete())}>
            <Icon name="trash" size={16} /> Supprimer
          </button>
        </Menu>
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
  const [query, setQuery] = useState("");
  const hero = useGlow<HTMLDivElement>();

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
    const id = await api().lessonCreate({ lang, title: s.title, text: s.text, collection: "Pour commencer", kind: "text" });
    bump();
    openLesson(id);
  };

  const remove = async (l: LessonSummary) => {
    if (!(await confirmAsk(`Supprimer « ${l.title} » ? Les mots appris sont conservés.`, "Supprimer la leçon", "Supprimer"))) return;
    await api().lessonDelete(l.id);
    toast("Leçon supprimée");
    bump();
  };

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

  const li = langInfo(lang);

  return (
    <>
      <div className="titlebar drag" data-tauri-drag-region>
        <div style={{ flex: 1 }} data-tauri-drag-region />
        <label className="search no-drag">
          <Icon name="search" size={16} />
          <input placeholder="Rechercher une leçon" value={query} onChange={(e) => setQuery(e.target.value)} aria-label="Rechercher une leçon" />
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
                {lessons ? `${lessons.length} leçon${lessons.length > 1 ? "s" : ""} en ${li.name.toLowerCase()}` : "…"} · {formatNumber(known)} mots connus
              </p>
            </div>
            <button className="btn soft" onClick={() => openImport()}>
              <Icon name="import" size={16} /> Importer du contenu
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
              <Cover hue={resume.hue} kind={resume.kind} big />
              <div className="hero-body">
                <span className="eyebrow">Reprendre la lecture</span>
                <h2 className="display">{resume.title}</h2>
                <p className="hero-excerpt">{resume.excerpt}…</p>
                <div className="hero-foot">
                  <button className="btn primary lg glow" onClick={() => openLesson(resume.id)}>
                    <Icon name="book" size={17} /> Continuer
                  </button>
                  <span className="muted num">
                    Page {Math.min(resume.page + 1, pagesOf(resume.word_count))} sur {pagesOf(resume.word_count)} · {resume.new_words} mots nouveaux
                  </span>
                </div>
              </div>
            </motion.div>
          )}

          {lessons && lessons.length > 0 && (
            <div className="lib-toolbar">
              <Segmented
                id="lib-filter"
                label="Filtrer"
                value={filter}
                onChange={(v) => setFilter(v as Filter)}
                options={[
                  { value: "all", label: "Tout" },
                  { value: "text", label: "Textes" },
                  { value: "audio", label: "Audio et vidéo" },
                  { value: "book", label: "Livres" },
                  { value: "done", label: "Terminées" },
                ]}
              />
            </div>
          )}

          {lessons && lessons.length === 0 && (
            <div className="empty">
              <Orb size={48} />
              <h3>Votre bibliothèque attend sa première lumière</h3>
              <p>Importez un article, un livre, un podcast ou une vidéo, ou commencez par une courte histoire écrite pour vous.</p>
              <div style={{ display: "flex", gap: 10, marginTop: 8 }}>
                <button className="btn primary lg glow" onClick={addStarter}>
                  Lire « {STARTERS[lang].title} »
                </button>
                <button className="btn outline lg" onClick={() => openImport()}>
                  <Icon name="import" size={16} /> Importer
                </button>
              </div>
            </div>
          )}

          <motion.div className="lesson-grid" layout>
            <AnimatePresence>
              {shown.map((l, i) => (
                <LessonCard key={l.id} l={l} index={i} onDelete={() => remove(l)} onRename={() => rename(l)} />
              ))}
            </AnimatePresence>
          </motion.div>
          {lessons && lessons.length > 0 && shown.length === 0 && <p className="muted" style={{ padding: "40px 0", textAlign: "center" }}>Aucune leçon ne correspond.</p>}
        </div>
      </div>
      <Sheet
        open={!!renaming}
        onClose={() => setRenaming(null)}
        title="Renommer la leçon"
        width={520}
        footer={
          <>
            <button className="btn ghost" onClick={() => setRenaming(null)}>Annuler</button>
            <button className="btn primary" onClick={saveRename} disabled={!newTitle.trim()}>Enregistrer</button>
          </>
        }
      >
        <input className="input" autoFocus value={newTitle} onChange={(e) => setNewTitle(e.target.value)} onKeyDown={(e) => e.key === "Enter" && saveRename()} aria-label="Titre" />
      </Sheet>
    </>
  );
}
