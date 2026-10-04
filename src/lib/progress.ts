/**
 * Progrès : temps actif passé dans les leçons, objectif du jour, mise en forme
 * des durées.
 *
 * Le temps d'apprentissage ne compte que le temps réellement passé dans une
 * leçon : fenêtre au premier plan et apprenant présent (un geste, souris,
 * clavier, défilement, depuis moins de 3 min), ou leçon qui joue (audio,
 * vidéo, voix). Sans geste depuis 3 min, on garde la première minute de
 * l'attente (lecture silencieuse probable) et on cesse de compter.
 */
import { useCallback, useEffect, useRef } from "react";
import { api } from "./api";
import { count, formatNumber, t } from "./i18n";
import { useApp } from "./store";
import type { LangCode } from "./types";

/** Objectifs du jour proposés, en minutes. */
export const GOALS = [5, 10, 15, 20, 30, 45, 60];

/** Sans geste depuis ce délai (et sans lecture en cours), l'apprenant est considéré absent. */
const IDLE_SECS = 180;
/** De l'attente sans geste, on garde cette part : le temps de lire une page en silence. */
const GRACE_SECS = 60;
/** Le temps compté part vers la base toutes les 30 s (et à la sortie). */
const FLUSH_SECS = 30;

const GESTURES = ["pointerdown", "pointermove", "keydown", "wheel", "touchstart"] as const;

export interface StudyClock {
  /** secondes actives depuis l'ouverture de la leçon */
  session(): number;
  /** envoie tout de suite le temps compté (fin de leçon) */
  flush(): Promise<void>;
}

/**
 * Compte le temps actif dans le lecteur. `lessonId` remet le compteur de la
 * séance à zéro d'une leçon à l'autre.
 */
export function useStudyClock(lang: LangCode | undefined, playing: boolean, lessonId: number | undefined): StudyClock {
  const s = useRef({ committed: 0, pending: 0, last: performance.now(), session: 0, lang: lang });
  const playingRef = useRef(playing);
  playingRef.current = playing;

  const send = useCallback(async () => {
    const st = s.current;
    const n = st.committed;
    if (n <= 0 || !st.lang) return;
    st.committed = 0;
    try {
      const r = await api().activityAdd(st.lang, 0, 0, n);
      if (r) {
        useApp
          .getState()
          .toast(
            r.streak > 1
              ? t(`Objectif du jour atteint · ${count(r.streak, "jour", "jours", "", "")} de suite`, `Daily goal reached · ${formatNumber(r.streak)}-day streak`)
              : t(`Objectif du jour atteint : ${r.goal_min} min`, `Daily goal reached: ${r.goal_min} min`),
            "light",
          );
      }
      // la série de la barre latérale suit
      void useApp.getState().refreshKnown();
    } catch {
      // base occupée : le temps sera renvoyé avec le prochain envoi
      st.committed += n;
    }
  }, []);

  // la séance repart de zéro à chaque leçon
  useEffect(() => {
    s.current.session = 0;
  }, [lessonId]);

  useEffect(() => {
    if (!lang) return;
    const st = s.current;
    st.lang = lang;
    st.last = performance.now();
    st.pending = 0;
    const commit = (n: number) => {
      st.committed += n;
      st.session += n;
    };
    // un geste confirme le temps passé depuis le précédent
    const touch = () => {
      if (st.pending) commit(st.pending);
      st.pending = 0;
      st.last = performance.now();
    };
    // absent : seule la première minute d'attente est gardée
    const leave = () => {
      if (st.pending) commit(Math.min(st.pending, GRACE_SECS));
      st.pending = 0;
    };
    const tick = () => {
      const now = performance.now();
      if (playingRef.current) {
        // écouter la leçon, c'est apprendre, même fenêtre en arrière-plan
        commit(1 + st.pending);
        st.pending = 0;
        st.last = now;
      } else if (document.visibilityState === "visible" && document.hasFocus()) {
        if ((now - st.last) / 1000 <= IDLE_SECS) st.pending += 1;
        else leave();
      } else leave();
      if (st.committed >= FLUSH_SECS) void send();
    };
    const onHide = () => {
      if (document.visibilityState === "hidden") {
        leave();
        void send();
      }
    };
    const onExit = () => {
      leave();
      void send();
    };
    for (const g of GESTURES) window.addEventListener(g, touch, { capture: true, passive: true });
    // le défilement ne remonte pas jusqu'à la fenêtre : écouté en capture sur le document
    document.addEventListener("scroll", touch, { capture: true, passive: true });
    document.addEventListener("visibilitychange", onHide);
    window.addEventListener("pagehide", onExit);
    const id = window.setInterval(tick, 1000);
    return () => {
      window.clearInterval(id);
      for (const g of GESTURES) window.removeEventListener(g, touch, { capture: true });
      document.removeEventListener("scroll", touch, { capture: true });
      document.removeEventListener("visibilitychange", onHide);
      window.removeEventListener("pagehide", onExit);
      // sortie de la leçon (un clic) : le temps en attente est confirmé
      touch();
      void send();
    };
  }, [lang, send]);

  return {
    session: () => s.current.session + s.current.pending,
    flush: async () => {
      const st = s.current;
      if (st.pending) {
        st.committed += st.pending;
        st.session += st.pending;
        st.pending = 0;
      }
      st.last = performance.now();
      await send();
    },
  };
}

/** Durée en morceaux pour l'affichage : 1 h 24 min → [["1", "h"], ["24", "min"]]. */
export function timeParts(secs: number): [string, string][] {
  const m = Math.round(Math.max(0, secs) / 60);
  if (m < 60) return [[formatNumber(m), "min"]];
  const h = Math.floor(m / 60);
  const r = m % 60;
  // au-delà de 100 heures, les minutes n'apprennent plus rien
  if (h >= 100 || r === 0) return [[formatNumber(h), "h"]];
  return [
    [String(h), "h"],
    [String(r).padStart(2, "0"), "min"],
  ];
}

/** Durée en une ligne : « 1 h 24 min », « 12 min », « 142 h ». */
export function studyTime(secs: number): string {
  return timeParts(secs)
    .map(([v, u]) => `${v} ${u}`)
    .join(" ");
}
