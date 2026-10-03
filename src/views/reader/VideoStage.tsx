import { motion } from "motion/react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
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

/** Mots au plus dans un sous-titre : quelques mots à la fois, pas la phrase entière. */
const CHUNK_WORDS = 5;

/**
 * Découpe la phrase [a, b) en groupes de quelques mots de tailles voisines,
 * coupés de préférence après une virgule ou un point-virgule.
 */
function chunksOf(tokens: Token[], a: number, b: number): [number, number][] {
  const words: number[] = [];
  for (let i = a; i < b; i++) if (tokens[i].w) words.push(i);
  const n = Math.ceil(words.length / CHUNK_WORDS);
  if (n <= 1) return [[a, b]];
  // une ponctuation entre le mot j-1 et le mot j
  const pause = (j: number) => {
    for (let i = words[j - 1] + 1; i < words[j]; i++) if (/[,;:…]/.test(tokens[i].t)) return true;
    return false;
  };
  const starts = [0];
  for (let c = 1; c < n; c++) {
    const even = Math.round((c * words.length) / n);
    const prev = starts[c - 1];
    const k = [even, even - 1, even + 1].find((j) => j > prev + 1 && j < words.length - 1 && j - prev <= CHUNK_WORDS && pause(j));
    starts.push(k ?? even);
  }
  return starts.map((w, c) => [c === 0 ? a : words[w], c === n - 1 ? b : words[starts[c + 1]]]);
}

/** Ressort de la lanterne, le même que dans le texte. */
const LANTERN_SPRING = { type: "spring", stiffness: 760, damping: 50, mass: 0.5 } as const;

/** Fenêtre native de Lumen, chargée à la demande. */
const nativeWindow = () => import("@tauri-apps/api/window").then((m) => m.getCurrentWindow());

/**
 * Scène vidéo : l'image seule au-dessus du texte ; en plein écran, l'image
 * occupe l'écran et les sous-titres interactifs s'affichent dans le noir, dessous.
 */
export function VideoStage({ lesson, tokens, terms, cursor, playing, onHost, onWord, onToggle, onVideoReady, cinema, setCinema }: Props) {
  const settings = useApp((s) => s.settings);
  const setSetting = useApp((s) => s.setSetting);
  const toast = useApp((s) => s.toast);
  const size = (settings.video_size as StageSize) || "large";
  const [fetching, setFetching] = useState<number | null>(null);
  const [subTr, setSubTr] = useState("");
  const showTr = settings.video_translate === "1";
  const trCache = useRef(new Map<string, string>());
  // phrase en cours (pour la traduction) et groupe de mots affiché
  const [shown, setShown] = useState<{ sentence: [number, number]; chunk: [number, number] } | null>(null);
  const subBox = useRef<HTMLDivElement>(null);
  const lampAt = useRef<{ chunk: number; y: number } | null>(null);
  const [lamp, setLamp] = useState<{ x: number; y: number; width: number; height: number; jump: boolean } | null>(null);
  const [ratio, setRatio] = useState(16 / 9);
  const host = useRef<HTMLDivElement | null>(null);

  const hasVideo = !!lesson.video_path;
  const canFetch = !hasVideo && /^https?:\/\//.test(lesson.source);

  const setHost = useCallback(
    (el: HTMLDivElement | null) => {
      host.current = el;
      onHost(el);
    },
    [onHost],
  );

  // proportions réelles de l'image : en plein écran, les sous-titres se placent juste dessous
  useEffect(() => {
    const el = host.current;
    if (!el) return;
    const read = () => {
      const v = el.querySelector("video");
      if (v && v.videoWidth && v.videoHeight) setRatio(v.videoWidth / v.videoHeight);
    };
    read();
    // la vidéo est insérée plus tard par le lecteur : on écoute en phase de capture
    el.addEventListener("loadedmetadata", read, true);
    el.addEventListener("resize", read, true);
    return () => {
      el.removeEventListener("loadedmetadata", read, true);
      el.removeEventListener("resize", read, true);
    };
  }, []);

  // groupe de mots en cours : il reste affiché jusqu'au suivant
  useEffect(() => {
    if (cursor < 0) return;
    const [a, b] = sentenceBounds(tokens, cursor);
    const chunk = chunksOf(tokens, a, b).find(([s, e]) => cursor >= s && cursor < e) ?? [a, b];
    setShown((p) => (p && p.chunk[0] === chunk[0] && p.chunk[1] === chunk[1] ? p : { sentence: [a, b], chunk }));
  }, [cursor, tokens]);

  const sentenceText = useMemo(() => {
    if (!shown) return "";
    const [a, b] = shown.sentence;
    return lesson.text.slice(tokens[a].s, tokens[b - 1].e).replace(/\s+/g, " ").trim();
  }, [shown, tokens, lesson.text]);

  // lanterne des sous-titres : elle glisse de mot en mot, et saute d'un bond
  // vers le premier mot d'un nouveau groupe (ou d'une nouvelle ligne)
  const chunkStart = shown?.chunk[0] ?? -1;
  const placeLamp = useCallback(
    (force: boolean) => {
      const box = subBox.current;
      const el = box?.querySelector<HTMLElement>(`[data-i="${cursor}"]`);
      if (!box || !el) {
        lampAt.current = null;
        setLamp(null);
        return;
      }
      const c = box.getBoundingClientRect();
      const r = el.getBoundingClientRect();
      const y = r.top - c.top;
      const prev = lampAt.current;
      const jump = force || !prev || prev.chunk !== chunkStart || Math.abs(prev.y - y) > r.height / 2;
      lampAt.current = { chunk: chunkStart, y };
      setLamp({ x: r.left - c.left, y, width: r.width, height: r.height, jump });
    },
    [cursor, chunkStart],
  );
  useLayoutEffect(() => placeLamp(false), [placeLamp, cinema]);
  // la fenêtre change de taille (entrée en plein écran) : la lanterne se recale
  const placeRef = useRef(placeLamp);
  placeRef.current = placeLamp;
  useEffect(() => {
    const onResize = () => placeRef.current(true);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  // traduction des sous-titres (facultative, plein écran seulement), mise en cache
  useEffect(() => {
    if (!cinema || !showTr || !sentenceText) {
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
  }, [sentenceText, showTr, cinema, lesson.lang]);

  // plein écran : la fenêtre prend tout l'écran et la barre latérale s'efface.
  // Une fenêtre déjà en plein écran avant reste ainsi à la sortie.
  // Les appels sont mis en file : un aller-retour rapide garde le bon ordre.
  const ownFullscreen = useRef(false);
  const fsQueue = useRef(Promise.resolve());
  useEffect(() => {
    document.documentElement.classList.toggle("cinema-mode", cinema);
    if (!isTauri) return;
    fsQueue.current = fsQueue.current
      .then(async () => {
        const w = await nativeWindow();
        if (cinema) {
          if (await w.isFullscreen()) return;
          ownFullscreen.current = true;
          await w.setFullscreen(true);
        } else if (ownFullscreen.current) {
          ownFullscreen.current = false;
          await w.setFullscreen(false);
        }
      })
      .catch(() => {});
  }, [cinema]);
  useEffect(
    () => () => {
      document.documentElement.classList.remove("cinema-mode");
      if (!isTauri) return;
      fsQueue.current = fsQueue.current
        .then(async () => {
          if (!ownFullscreen.current) return;
          ownFullscreen.current = false;
          await (await nativeWindow()).setFullscreen(false);
        })
        .catch(() => {});
    },
    [],
  );

  // plein écran quitté par le Mac (bouton vert, ctrl ⌘ F) : la scène revient aussi
  useEffect(() => {
    if (!isTauri || !cinema) return;
    let alive = true;
    let unlisten: (() => void) | undefined;
    nativeWindow()
      .then(async (w) => {
        let was = await w.isFullscreen();
        const off = await w.onResized(async () => {
          const now = await w.isFullscreen().catch(() => was);
          if (was && !now && alive) {
            ownFullscreen.current = false;
            setCinema(false);
          }
          was = now;
        });
        if (alive) unlisten = off;
        else off();
      })
      .catch(() => {});
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [cinema, setCinema]);

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

  return (
    <div className={`video-stage ${size} ${cinema ? "cinema" : ""}`} style={{ ["--ar" as string]: ratio }}>
      <div className="video-frame" onClick={hasVideo ? onToggle : undefined}>
        <div className="video-host" ref={setHost} />
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
        <div className="video-tools" onClick={(e) => e.stopPropagation()}>
          {cinema ? (
            <button className={`icon-btn ${showTr ? "on" : ""}`} onClick={() => setSetting("video_translate", showTr ? "0" : "1")} aria-label="Sous-titres traduits" title="Sous-titres traduits en français">
              <span style={{ fontSize: 11, fontWeight: 700 }}>FR</span>
            </button>
          ) : (
            <button className="icon-btn" onClick={() => setSetting("video_size", size === "large" ? "compact" : "large")} aria-label={size === "large" ? "Réduire la vidéo" : "Agrandir la vidéo"} title={size === "large" ? "Réduire la vidéo" : "Agrandir la vidéo"}>
              <Icon name={size === "large" ? "layers" : "video"} size={16} />
            </button>
          )}
          <button className="icon-btn" onClick={() => setCinema((c) => !c)} aria-label={cinema ? "Quitter le plein écran" : "Plein écran"} title={cinema ? "Quitter le plein écran (Échap)" : "Plein écran, sous-titres sous l'image"}>
            <Icon name={cinema ? "shrink" : "expand"} size={16} />
          </button>
        </div>
      </div>
      {cinema && (
        <div className="subtitle-band" dir="auto">
          {shown ? (
            <>
              <div className="sub-box" ref={subBox}>
                {lamp && (
                  <motion.div
                    className="sub-lantern"
                    initial={false}
                    animate={{ x: lamp.x, y: lamp.y, width: lamp.width, height: lamp.height }}
                    transition={lamp.jump ? { duration: 0 } : LANTERN_SPRING}
                  />
                )}
                <p key={shown.chunk[0]} className="sub-line" lang={lesson.lang}>
                  {tokens.slice(shown.chunk[0], shown.chunk[1]).map((t, j) => {
                    const i = shown.chunk[0] + j;
                    if (!t.w) return <span key={i}>{t.t}</span>;
                    const st = terms[t.k]?.status ?? 0;
                    return (
                      <span key={i} data-i={i} className={`sw s${st}`} onClick={() => onWord(i)}>
                        {t.t}
                      </span>
                    );
                  })}
                </p>
              </div>
              {showTr && subTr && <p className="sub-tr">{subTr}</p>}
            </>
          ) : (
            <p className="sub-hint">
              Les sous-titres s'affichent ici pendant la lecture. Touchez un mot : sa traduction apparaît à droite.
              <span>Espace : lecture · Échap : quitter le plein écran</span>
            </p>
          )}
        </div>
      )}
    </div>
  );
}
