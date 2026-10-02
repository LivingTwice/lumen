import { AnimatePresence, motion } from "motion/react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Icon } from "../../components/Icon";
import { api, errorText, isTauri } from "../../lib/api";
import { useApp } from "../../lib/store";
import { sentenceBounds } from "../../lib/tokenize";
import type { Lesson, Term, Token } from "../../lib/types";

export type StageSize = "compact" | "large";

interface Props {
  lesson: Lesson;
  tokens: Token[];
  terms: Record<string, Term>;
  cursor: number;
  playing: boolean;
  onHost(el: HTMLDivElement | null): void;
  onWord(i: number): void;
  onToggle(): void;
  onVideoReady(path: string): void;
  cinema: boolean;
  setCinema(v: boolean | ((c: boolean) => boolean)): void;
}

/** Scène vidéo : l'image, des sous-titres interactifs et le mode cinéma. */
export function VideoStage({ lesson, tokens, terms, cursor, playing, onHost, onWord, onToggle, onVideoReady, cinema, setCinema }: Props) {
  const settings = useApp((s) => s.settings);
  const setSetting = useApp((s) => s.setSetting);
  const toast = useApp((s) => s.toast);
  const size = (settings.video_size as StageSize) || "large";
  const [fetching, setFetching] = useState<number | null>(null);
  const [subTr, setSubTr] = useState("");
  const showTr = settings.video_translate === "1";
  const trCache = useRef(new Map<string, string>());
  const lastSentence = useRef(-1);
  const [shownSentence, setShownSentence] = useState<[number, number] | null>(null);

  const hasVideo = !!lesson.video_path;
  const canFetch = !hasVideo && /^https?:\/\//.test(lesson.source);

  // phrase en cours : elle reste affichée jusqu'à la suivante
  useEffect(() => {
    if (cursor < 0) return;
    const [a, b] = sentenceBounds(tokens, cursor);
    if (a !== lastSentence.current) {
      lastSentence.current = a;
      setShownSentence([a, b]);
    }
  }, [cursor, tokens]);

  const sentenceText = useMemo(() => {
    if (!shownSentence) return "";
    const [a, b] = shownSentence;
    return lesson.text.slice(tokens[a].s, tokens[b - 1].e).replace(/\s+/g, " ").trim();
  }, [shownSentence, tokens, lesson.text]);

  // traduction des sous-titres (facultative), mise en cache
  useEffect(() => {
    if (!showTr || !sentenceText) {
      setSubTr("");
      return;
    }
    const cached = trCache.current.get(sentenceText);
    if (cached) {
      setSubTr(cached);
      return;
    }
    setSubTr("");
    let alive = true;
    const t = window.setTimeout(() => {
      let acc = "";
      api()
        .aiSentence(lesson.lang, sentenceText, (p) => {
          acc += p;
          if (alive) setSubTr(acc);
        })
        .then((r) => {
          trCache.current.set(sentenceText, r);
          if (alive) setSubTr(r);
        })
        .catch(() => {});
    }, 250);
    return () => {
      alive = false;
      window.clearTimeout(t);
    };
  }, [sentenceText, showTr, lesson.lang]);

  // mode cinéma : la fenêtre passe en plein écran, la barre latérale s'efface
  useEffect(() => {
    document.documentElement.classList.toggle("cinema-mode", cinema);
    if (isTauri) import("@tauri-apps/api/window").then(({ getCurrentWindow }) => getCurrentWindow().setFullscreen(cinema)).catch(() => {});
  }, [cinema]);
  useEffect(
    () => () => {
      document.documentElement.classList.remove("cinema-mode");
      if (isTauri) import("@tauri-apps/api/window").then(({ getCurrentWindow }) => getCurrentWindow().setFullscreen(false)).catch(() => {});
    },
    [],
  );

  const fetchVideo = async () => {
    setFetching(0);
    try {
      const path = await api().lessonFetchVideo(lesson.id, (e) => {
        if (e.type === "progress") setFetching(e.value);
      });
      onVideoReady(path);
      toast("La vidéo est prête", "light");
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setFetching(null);
    }
  };

  const subtitle = shownSentence ? (
    <div className="subtitle" onClick={(e) => e.stopPropagation()}>
      <p lang={lesson.lang}>
        {tokens.slice(shownSentence[0], shownSentence[1]).map((t, j) => {
          const i = shownSentence[0] + j;
          if (!t.w) return <span key={i}>{t.t}</span>;
          const st = terms[t.k]?.status ?? 0;
          return (
            <span
              key={i}
              className={`sw s${st} ${i === cursor ? "now" : ""}`}
              onClick={() => onWord(i)}
            >
              {t.t}
            </span>
          );
        })}
      </p>
      {showTr && subTr && <p className="subtitle-tr">{subTr}</p>}
    </div>
  ) : null;

  return (
    <div className={`video-stage ${size} ${cinema ? "cinema" : ""}`}>
      <div className="video-frame" onClick={hasVideo ? onToggle : undefined}>
        <div className="video-host" ref={onHost} />
        {!hasVideo && (
          <div className="video-missing">
            <span className="orb" style={{ width: 34, height: 34, ["--s" as string]: "34px" }} />
            {canFetch ? (
              fetching === null ? (
                <>
                  <strong>L'image de cette vidéo n'a pas encore été téléchargée</strong>
                  <button className="btn sm video-get" onClick={(e) => (e.stopPropagation(), fetchVideo())}>
                    <Icon name="download" size={14} /> Télécharger la vidéo
                  </button>
                </>
              ) : (
                <>
                  <strong>Téléchargement de la vidéo…</strong>
                  <div className="bar live" style={{ width: 220 }}>
                    <i style={{ width: `${fetching}%` }} />
                  </div>
                </>
              )
            ) : (
              <strong>Leçon audio</strong>
            )}
          </div>
        )}
        {hasVideo && !playing && (
          <div className="video-play" aria-hidden="true">
            <Icon name="play" size={26} />
          </div>
        )}
        {(hasVideo || cinema) && subtitle}
        <div className="video-tools" onClick={(e) => e.stopPropagation()}>
          <button className={`icon-btn ${showTr ? "on" : ""}`} onClick={() => setSetting("video_translate", showTr ? "0" : "1")} aria-label="Sous-titres traduits" title="Sous-titres traduits en français">
            <span style={{ fontSize: 11, fontWeight: 700 }}>FR</span>
          </button>
          {!cinema && (
            <button className="icon-btn" onClick={() => setSetting("video_size", size === "large" ? "compact" : "large")} aria-label={size === "large" ? "Réduire la vidéo" : "Agrandir la vidéo"} title={size === "large" ? "Réduire" : "Agrandir"}>
              <Icon name={size === "large" ? "layers" : "video"} size={16} />
            </button>
          )}
          <button className="icon-btn" onClick={() => setCinema((c) => !c)} aria-label={cinema ? "Quitter le mode cinéma" : "Mode cinéma"} title={cinema ? "Quitter (Échap)" : "Mode cinéma"}>
            <Icon name={cinema ? "close" : "eye"} size={16} />
          </button>
        </div>
      </div>
      <AnimatePresence>
        {cinema && (
          <motion.div className="cinema-hint" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ delay: 0.4 }}>
            Touchez un mot des sous-titres : sa traduction s'affiche à droite · Espace : lecture · Échap : quitter
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
