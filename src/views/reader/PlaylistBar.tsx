import { motion } from "motion/react";
import { useEffect, useRef, useState } from "react";
import { Cover } from "../../components/Cover";
import { Icon } from "../../components/Icon";
import { api } from "../../lib/api";
import { useApp } from "../../lib/store";
import type { Lesson, LessonSummary, Playlist } from "../../lib/types";

/**
 * Playlist suivie par la leçon ouverte, relue à chaque changement. Une
 * playlist disparue (ou dont la leçon est sortie) n'est plus suivie.
 */
export function usePlaylist(lesson: Lesson | undefined): Playlist | null {
  const queue = useApp((s) => s.queue);
  const version = useApp((s) => s.libraryVersion);
  const [pl, setPl] = useState<Playlist | null>(null);
  const marked = useRef(0);
  const id = lesson?.id;
  const lang = lesson?.lang;
  useEffect(() => {
    if (!id || !lang || !queue) {
      setPl(null);
      return;
    }
    let alive = true;
    api()
      .playlistsList(lang)
      .then((ps) => {
        if (!alive) return;
        const p = ps.find((x) => x.id === queue && x.lessons.includes(id)) ?? null;
        setPl(p);
        if (!p) {
          useApp.setState({ queue: null });
          void useApp.getState().setSetting("last_playlist", "");
        } else if (marked.current !== id) {
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

/** Au centre de la barre du haut : la playlist, la place de la leçon, précédente et suivante. */
export function PlaylistStrip({ pl, lessonId, playing }: { pl: Playlist; lessonId: number; playing(): boolean }) {
  const openLesson = useApp((s) => s.openLesson);
  const openPlaylist = useApp((s) => s.openPlaylist);
  const i = pl.lessons.indexOf(lessonId);
  const prev = pl.lessons[i - 1];
  const next = pl.lessons[i + 1];
  // on passe d'une leçon à l'autre sans couper l'écoute en cours
  const go = (id: number) => openLesson(id, { playlist: pl.id, autoplay: playing() });
  return (
    <motion.div className="pl-strip no-drag" initial={{ opacity: 0, y: -4 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.25 }}>
      <button className="icon-btn" onClick={() => prev && go(prev)} disabled={!prev} aria-label="Leçon précédente de la playlist" title="Leçon précédente">
        <Icon name="left" size={16} />
      </button>
      <button className="pl-strip-name" onClick={() => openPlaylist(pl.id)} title="Ouvrir la playlist">
        <Icon name="playlist" size={14} />
        <span className="name">{pl.name}</span>
        <span className="num pos">
          {i + 1} / {pl.lessons.length}
        </span>
      </button>
      <button className="icon-btn" onClick={() => next && go(next)} disabled={!next} aria-label="Leçon suivante de la playlist" title="Leçon suivante">
        <Icon name="right" size={16} />
      </button>
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
        <span className="eyebrow num">Ensuite, dans {left} s</span>
        <strong>{next.title}</strong>
      </span>
      <button className="up-go" onClick={onGo} aria-label={`Lire maintenant : ${next.title}`} title="Lire maintenant">
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
      <button className="icon-btn" onClick={onCancel} aria-label="Rester sur cette leçon" title="Rester sur cette leçon">
        <Icon name="close" size={16} />
      </button>
    </motion.div>
  );
}
