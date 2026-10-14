import { motion } from "motion/react";
import { useEffect, useRef, useState } from "react";
import { Cover } from "../../components/Cover";
import { Icon } from "../../components/Icon";
import { api, errorText } from "../../lib/api";
import { useApp } from "../../lib/store";
import type { Lesson, LessonSummary, Playlist } from "../../lib/types";
import { t } from "../../lib/i18n";
import { isBook, nextIn } from "../../lib/playlists";

/**
 * Playlist suivie par la leçon ouverte, relue à chaque changement. Une
 * playlist disparue (ou dont la leçon est sortie) n'est plus suivie. Un
 * chapitre ouvert seul (bibliothèque, dernière leçon) suit toujours son livre.
 */
export function usePlaylist(lesson: Lesson | undefined): Playlist | null {
  const queue = useApp((s) => s.queue);
  const version = useApp((s) => s.libraryVersion);
  const [pl, setPl] = useState<Playlist | null>(null);
  const marked = useRef(0);
  const id = lesson?.id;
  const lang = lesson?.lang;
  useEffect(() => {
    if (!id || !lang) {
      setPl(null);
      return;
    }
    let alive = true;
    api()
      .playlistsList(lang)
      .then((ps) => {
        if (!alive) return;
        const p = (queue ? ps.find((x) => x.id === queue && x.lessons.includes(id)) : undefined) ?? ps.find((x) => isBook(x) && x.lessons.includes(id)) ?? null;
        setPl(p);
        if (!p) {
          if (queue) {
            useApp.setState({ queue: null });
            useApp.getState().settleLesson(lang);
          }
          return;
        }
        if (p.id !== queue) {
          useApp.setState({ queue: p.id });
          useApp.getState().settleLesson(lang);
        }
        if (marked.current !== id) {
          // la playlist reprendra ici
          marked.current = id;
          if (p.current !== id) void api().playlistUpdate(p.id, { current: id });
        }
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [id, lang, queue, version]);
  return pl;
}

/** Met la playlist en boucle, ou l'en sort ; le lecteur la relit. */
export async function toggleLoop(pl: Playlist) {
  const app = useApp.getState();
  try {
    await api().playlistUpdate(pl.id, { looping: !pl.looping });
    app.bumpLibrary();
    app.toast(pl.looping ? t("La playlist ne se lit plus en boucle", "The playlist no longer loops") : t("La playlist se lit en boucle", "The playlist now loops"), "light");
  } catch (e) {
    app.toast(errorText(e), "error");
  }
}

/** Au centre de la barre du haut : la playlist, la place de la leçon, précédente et suivante. */
export function PlaylistStrip({ pl, lessonId, playing }: { pl: Playlist; lessonId: number; playing(): boolean }) {
  const openLesson = useApp((s) => s.openLesson);
  const openPlaylist = useApp((s) => s.openPlaylist);
  const i = pl.lessons.indexOf(lessonId);
  const prev = pl.lessons[i - 1];
  // en boucle, après la dernière vient la première
  const after = nextIn(pl, lessonId);
  const next = after !== lessonId ? after : undefined;
  // on passe d'une leçon à l'autre sans couper l'écoute en cours
  const go = (id: number) => openLesson(id, { playlist: pl.id, autoplay: playing() });
  const book = isBook(pl);
  return (
    <motion.div className="pl-strip no-drag" initial={{ opacity: 0, y: -4 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.25 }}>
      <button
        className="icon-btn"
        onClick={() => prev && go(prev)}
        disabled={!prev}
        aria-label={book ? t("Chapitre précédent du livre", "Previous chapter of the book") : t("Leçon précédente de la playlist", "Previous lesson in the playlist")}
        title={book ? t("Chapitre précédent", "Previous chapter") : t("Leçon précédente", "Previous lesson")}
      >
        <Icon name="left" size={16} />
      </button>
      <button className="pl-strip-name" onClick={() => openPlaylist(pl.id)} title={book ? t("Voir les chapitres du livre", "See the book's chapters") : t("Ouvrir la playlist", "Open the playlist")}>
        <Icon name={book ? "book" : "playlist"} size={14} />
        <span className="name">{pl.name}</span>
        <span className="num pos">
          {i + 1} / {pl.lessons.length}
        </span>
      </button>
      <button
        className="icon-btn"
        onClick={() => next && go(next)}
        disabled={!next}
        aria-label={book ? t("Chapitre suivant du livre", "Next chapter of the book") : t("Leçon suivante de la playlist", "Next lesson in the playlist")}
        title={book ? t("Chapitre suivant", "Next chapter") : t("Leçon suivante", "Next lesson")}
      >
        <Icon name="right" size={16} />
      </button>
      {!book && (
        <button
          className={`icon-btn pl-strip-loop ${pl.looping ? "on" : ""}`}
          onClick={() => void toggleLoop(pl)}
          aria-pressed={pl.looping}
          aria-label={t("Lire la playlist en boucle", "Loop the playlist")}
          title={pl.looping ? t("En boucle : après la dernière leçon, la première", "Looping: after the last lesson, the first one") : t("Lire la playlist en boucle", "Loop the playlist")}
        >
          <Icon name="repeat" size={15} />
        </button>
      )}
    </motion.div>
  );
}

/** Secondes avant que la leçon suivante s'enchaîne. */
const UP_NEXT_SECS = 8;

/**
 * Fin d'une leçon dans une playlist : la suivante s'annonce, un anneau de
 * lumière se remplit, puis elle s'ouvre et sa lecture démarre.
 */
export function UpNext({ next, onGo, onCancel }: { next: LessonSummary; onGo(): void; onCancel(): void }) {
  const [left, setLeft] = useState(UP_NEXT_SECS);
  const goRef = useRef(onGo);
  goRef.current = onGo;
  useEffect(() => {
    const t0 = performance.now();
    const id = window.setInterval(() => {
      const l = UP_NEXT_SECS - (performance.now() - t0) / 1000;
      if (l <= 0) {
        window.clearInterval(id);
        goRef.current();
      } else setLeft(Math.ceil(l));
    }, 200);
    return () => window.clearInterval(id);
  }, []);
  const r = 21;
  return (
    <motion.div
      className="up-next"
      role="status"
      initial={{ opacity: 0, y: 18, scale: 0.97 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: 10, scale: 0.98 }}
      transition={{ type: "spring", stiffness: 320, damping: 28 }}
    >
      <span className="up-thumb">
        <Cover lesson={next} bare />
      </span>
      <span className="up-body">
        <span className="eyebrow num">{t(`Ensuite, dans ${left} s`, `Up next, in ${left} s`)}</span>
        <strong>{next.title}</strong>
      </span>
      <button className="up-go" onClick={onGo} aria-label={t(`Lire maintenant : ${next.title}`, `Play now: ${next.title}`)} title={t("Lire maintenant", "Play now")}>
        <svg className="up-ring" viewBox="0 0 48 48" aria-hidden="true">
          <circle cx="24" cy="24" r={r} className="ring-bg" />
          <motion.circle
            cx="24"
            cy="24"
            r={r}
            className="ring"
            initial={{ pathLength: 0 }}
            animate={{ pathLength: 1 }}
            transition={{ duration: UP_NEXT_SECS, ease: "linear" }}
          />
        </svg>
        <Icon name="play" size={16} />
      </button>
      <button className="icon-btn" onClick={onCancel} aria-label={t("Rester sur cette leçon", "Stay on this lesson")} title={t("Rester sur cette leçon", "Stay on this lesson")}>
        <Icon name="close" size={16} />
      </button>
    </motion.div>
  );
}
