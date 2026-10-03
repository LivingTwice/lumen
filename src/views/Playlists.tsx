import { AnimatePresence, Reorder, motion, useDragControls } from "motion/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Cover, KIND, PlaylistCover } from "../components/Cover";
import { Icon } from "../components/Icon";
import { Menu, Orb, Segmented, Sheet, useGlow } from "../components/ui";
import { api, errorText } from "../lib/api";
import { confirmAsk } from "../lib/dialogs";
import { langInfo } from "../lib/langs";
import { formatLength, lessonsOf, listenSecs, playPlaylist, plural } from "../lib/playlists";
import { formatDuration, formatNumber, useApp } from "../lib/store";
import type { LessonSummary, Playlist, PlaylistPatch } from "../lib/types";
import { progressOf } from "./Library";

const SPRING = { type: "spring", stiffness: 260, damping: 28 } as const;

/** Durée (audio, vidéo) ou longueur (texte) d'une leçon, en quelques caractères. */
function lengthOf(l: LessonSummary): string {
  return l.has_media && l.duration > 0 ? formatDuration(l.duration) : `${formatNumber(l.word_count)} mots`;
}

function subtitleOf(l: LessonSummary): string {
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
    setLists((ls) => ls?.map((p) => (p.id === id ? { ...p, ...(patch.name !== undefined && { name: patch.name.trim() || p.name }), ...(patch.lessons && { lessons: patch.lessons }) } : p)) ?? ls);
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
    if (!(await confirmAsk(`Supprimer la playlist « ${p.name} » ? Ses leçons restent dans votre bibliothèque.`, "Supprimer la playlist", "Supprimer"))) return;
    try {
      await api().playlistDelete(p.id);
      if (useApp.getState().queue === p.id) useApp.setState({ queue: null });
      openPlaylist(null);
      setLists((ls) => ls?.filter((x) => x.id !== p.id) ?? ls);
      toast("Playlist supprimée");
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
        title="Nouvelle playlist"
        width={520}
        footer={
          <>
            <button className="btn ghost" onClick={() => setNaming(false)}>
              Annuler
            </button>
            <button className="btn primary" onClick={create}>
              Créer
            </button>
          </>
        }
      >
        <input
          className="input"
          autoFocus
          placeholder="Par exemple : Podcasts du matin"
          value={name}
          maxLength={120}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && create()}
          aria-label="Nom de la playlist"
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

// ---------- toutes les playlists ----------

function PlaylistsHome({ lists, all, onCreate, onDelete }: { lists: Playlist[] | null; all: LessonSummary[] | null; onCreate(): void; onDelete(p: Playlist): void }) {
  const lang = useApp((s) => s.lang)();
  const li = langInfo(lang);
  return (
    <>
      <div className="titlebar drag" data-tauri-drag-region />
      <div className="view">
        <div className="view-inner">
          <header className="page-head">
            <div>
              <h1>Playlists</h1>
              <p>{lists && lists.length ? `${plural(lists.length, "playlist")} en ${li.name.toLowerCase()}` : "Vos leçons, dans l'ordre qui vous plaît"}</p>
            </div>
            {lists && lists.length > 0 && (
              <button className="btn soft" onClick={onCreate}>
                <Icon name="plus" size={16} /> Nouvelle playlist
              </button>
            )}
          </header>

          {lists && all && lists.length === 0 && (
            <div className="empty">
              <Orb size={48} />
              <h3>Des leçons qui s'enchaînent</h3>
              <p>Rassemblez des leçons dans une playlist : à l'écoute, chacune laisse place à la suivante, comme une émission. Idéal pour une série de podcasts, un livre audio ou vos révisions du matin.</p>
              <button className="btn primary lg glow" onClick={onCreate} style={{ marginTop: 8 }}>
                <Icon name="plus" size={16} stroke={2} /> Créer une playlist
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
                Nouvelle playlist
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
  const secs = listenSecs(ls);
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
      <button className="lesson-hit" onClick={() => openPlaylist(p.id)} aria-label={`Ouvrir la playlist ${p.name}`} />
      <div className="pl-cover-wrap">
        <PlaylistCover lessons={ls} seed={p.id} />
        {ls.length > 0 && (
          <button className="pl-play" onClick={() => playPlaylist(p)} aria-label={`Écouter la playlist ${p.name}`} title={p.current ? "Reprendre l'écoute" : "Écouter la playlist"}>
            <Icon name="play" size={18} />
          </button>
        )}
      </div>
      <div className="lesson-body">
        <span className={`lesson-collection ${playing ? "pl-live" : ""}`}>{playing ? "En cours d'écoute" : "Playlist"}</span>
        <h3 className="lesson-title">{p.name}</h3>
        <div className="lesson-meta">
          <span className="num">{plural(ls.length, "leçon")}</span>
          {secs > 0 && (
            <>
              <span className="sep" />
              <span className="num">{formatLength(secs)}</span>
            </>
          )}
        </div>
        {ls.length > 0 && (
          <div className="lesson-known" title={`${done} leçon${done > 1 ? "s" : ""} terminée${done > 1 ? "s" : ""} sur ${ls.length}`}>
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
            <button className="icon-btn" onClick={() => setMenu((m) => !m)} aria-label="Options de la playlist">
              <Icon name="more" size={18} stroke={2.6} />
            </button>
          }
        >
          <button className="menu-item" onClick={() => (setMenu(false), openPlaylist(p.id))}>
            <Icon name="playlist" size={16} /> Ouvrir
          </button>
          <div className="menu-sep" />
          <button className="menu-item danger" onClick={() => (setMenu(false), onDelete())}>
            <Icon name="trash" size={16} /> Supprimer la playlist
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
  const secs = listenSecs(ls);
  const words = ls.reduce((s, l) => s + l.word_count, 0);
  const resume = p.current ? byId.get(p.current) : undefined;

  const saveTitle = () => {
    setEditing(false);
    const t = title.trim();
    if (t && t !== p.name) onUpdate({ name: t });
    else setTitle(p.name);
  };

  const removeLesson = (l: LessonSummary) => {
    onUpdate({ lessons: p.lessons.filter((x) => x !== l.id) });
    toast(`« ${l.title} » a quitté la playlist`);
  };

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
          <PlaylistCover lessons={ls} seed={p.id} />
        </div>
        <div className="view-inner">
          <header className="pl-hero">
            <PlaylistCover lessons={ls} seed={p.id} big />
            <div className="pl-hero-body">
              <span className="eyebrow">Playlist · {li.name}</span>
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
                  aria-label="Nom de la playlist"
                />
              ) : (
                <h1 className="display pl-title" onDoubleClick={() => (setTitle(p.name), setEditing(true))} title="Double-cliquez pour renommer">
                  {p.name}
                </h1>
              )}
              <p className="pl-meta num">
                {plural(ls.length, "leçon")}
                {secs > 0 && ` · ${formatLength(secs)} d'écoute`}
                {words > 0 && ` · ${formatNumber(words)} mots`}
                {done > 0 && ` · ${done} terminée${done > 1 ? "s" : ""}`}
              </p>
              <div className="pl-actions">
                <button className="btn primary lg glow" onClick={() => playPlaylist(p)} disabled={!ls.length}>
                  <Icon name="play" size={15} /> {resume ? "Reprendre" : "Écouter"}
                </button>
                <button className="btn outline lg" onClick={onAdd}>
                  <Icon name="plus" size={16} /> Ajouter des leçons
                </button>
                <Menu
                  open={menu}
                  onClose={() => setMenu(false)}
                  anchor={
                    <button className="icon-btn pl-more" onClick={() => setMenu((m) => !m)} aria-label="Options de la playlist">
                      <Icon name="more" size={20} stroke={2.6} />
                    </button>
                  }
                >
                  <button className="menu-item" onClick={() => (setMenu(false), setTitle(p.name), setEditing(true))}>
                    <Icon name="edit" size={16} /> Renommer
                  </button>
                  <div className="menu-sep" />
                  <button className="menu-item danger" onClick={() => (setMenu(false), onDelete())}>
                    <Icon name="trash" size={16} /> Supprimer la playlist
                  </button>
                </Menu>
              </div>
              {resume && <p className="pl-resume muted">Reprise à « {resume.title} »</p>}
            </div>
          </header>

          {ls.length === 0 ? (
            <div className="pl-empty">
              <p>Cette playlist attend ses premières leçons.</p>
              <button className="btn soft" onClick={onAdd}>
                <Icon name="plus" size={15} /> Ajouter des leçons
              </button>
            </div>
          ) : (
            <Reorder.Group as="ol" axis="y" values={order} onReorder={setOrder} className="pl-list">
              <AnimatePresence initial={false}>
                {order.map((id, i) => {
                  const l = byId.get(id);
                  return l ? (
                    <Row
                      key={id}
                      l={l}
                      index={i}
                      current={id === p.current}
                      onOpen={() => openLesson(id, { playlist: p.id })}
                      onPlay={() => playPlaylist(p, id)}
                      onRemove={() => removeLesson(l)}
                      onDrop={saveOrder}
                    />
                  ) : null;
                })}
              </AnimatePresence>
            </Reorder.Group>
          )}
        </div>
      </motion.div>
    </>
  );
}

function Row({ l, index, current, onOpen, onPlay, onRemove, onDrop }: { l: LessonSummary; index: number; current: boolean; onOpen(): void; onPlay(): void; onRemove(): void; onDrop(): void }) {
  const controls = useDragControls();
  const [dragging, setDragging] = useState(false);
  const progress = progressOf(l);
  return (
    <Reorder.Item
      as="li"
      value={l.id}
      dragListener={false}
      dragControls={controls}
      className={`pl-row ${current ? "current" : ""} ${dragging ? "dragging" : ""}`}
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0, transition: { ...SPRING, delay: Math.min(index, 14) * 0.025 } }}
      exit={{ opacity: 0, x: -24, transition: { duration: 0.2 } }}
      onDragStart={() => setDragging(true)}
      onDragEnd={() => {
        setDragging(false);
        onDrop();
      }}
    >
      <button
        className="pl-grip"
        onPointerDown={(e) => {
          e.preventDefault();
          controls.start(e);
        }}
        aria-label="Déplacer la leçon"
        title="Glisser pour changer l'ordre"
      >
        <Icon name="grip" size={16} stroke={2.8} />
      </button>
      <span className="pl-index num">{current ? <Orb size={10} /> : index + 1}</span>
      <button className="pl-row-hit" onClick={onOpen} aria-label={`Ouvrir ${l.title}`} />
      <div className="pl-thumb">
        <Cover lesson={l} bare />
        <button className="pl-thumb-play" onClick={onPlay} aria-label={`Écouter à partir de ${l.title}`} title="Écouter la playlist à partir d'ici">
          <Icon name="play" size={14} />
        </button>
      </div>
      <div className="pl-row-body">
        <strong>{l.title}</strong>
        <span>{current ? `En cours · ${subtitleOf(l)}` : subtitleOf(l)}</span>
      </div>
      <div className="pl-row-end">
        {l.completed ? (
          <span className="done-pill">
            <Icon name="check" size={12} stroke={2.4} /> Lu
          </span>
        ) : (
          progress > 0.005 && (
            <span className="pl-row-progress" title={`${Math.round(progress * 100)} %`}>
              <i style={{ width: `${progress * 100}%` }} />
            </span>
          )
        )}
        <button className="icon-btn pl-remove" onClick={onRemove} aria-label="Retirer de la playlist" title="Retirer de la playlist (la leçon reste dans la bibliothèque)">
          <Icon name="close" size={15} />
        </button>
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
      title="Ajouter des leçons"
      width={660}
      footer={
        <>
          <span className="muted pick-count num">
            {added > 0 ? `${plural(added, "leçon")} ajoutée${added > 1 ? "s" : ""} à « ${shown.name} »` : `Dans « ${shown.name} » : ${plural(shown.lessons.length, "leçon")}`}
          </span>
          <button className="btn primary" onClick={onClose}>
            Terminé
          </button>
        </>
      }
    >
      <div className="pick-tools">
        <label className="search">
          <Icon name="search" size={16} />
          <input autoFocus placeholder="Rechercher une leçon" value={q} onChange={(e) => setQ(e.target.value)} aria-label="Rechercher une leçon" />
        </label>
        <Segmented
          id="pick-filter"
          label="Filtrer"
          value={filter}
          onChange={(v) => setFilter(v as typeof filter)}
          options={[
            { value: "all", label: "Toutes" },
            { value: "media", label: "Audio et vidéo" },
            { value: "todo", label: "À lire" },
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
        {list.length === 0 && <p className="muted pick-none">{all.length ? "Aucune leçon ne correspond." : "Votre bibliothèque est encore vide : importez d'abord des leçons."}</p>}
      </div>
    </Sheet>
  );
}
