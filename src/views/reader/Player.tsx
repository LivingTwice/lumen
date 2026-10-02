import { forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Icon } from "../../components/Icon";
import { Segmented } from "../../components/ui";
import { api, errorText, isNoModel } from "../../lib/api";
import { formatDuration, useApp } from "../../lib/store";
import type { PageRange } from "../../lib/tokenize";
import { loadVoices, speak, ttsAvailable, voicesFor, type SpeakHandle } from "../../lib/tts";
import type { Lesson, Token } from "../../lib/types";

export interface PlayerHandle {
  toggle(): void;
  playFrom(tokenIndex: number): void;
  pause(): void;
  stop(): void;
  isPlaying(): boolean;
  /** oublie la position mémorisée (leçon terminée : la prochaine lecture repart du début) */
  forget(): void;
}

export interface PlaybackState {
  playing: boolean;
  time: number;
  duration: number;
}

interface Props {
  lesson: Lesson;
  tokens: Token[];
  pages: PageRange[];
  page: number;
  onPage(p: number): void;
  onCursor(i: number): void;
  /** emplacement (dans la scène vidéo) où afficher l'image */
  videoHost?: HTMLElement | null;
  onState?(s: PlaybackState): void;
  /** nouveaux horodatages après un recalage de la lanterne */
  onResynced?(timings: string): void;
}

/**
 * Avance de la lanterne sur le son : elle part un peu avant le mot pour y
 * arriver au moment où il commence (son déplacement dure environ 60 ms).
 */
const LANTERN_LEAD = 0.06;

/** Dernier indice i tel que get(i) <= x (recherche dichotomique). */
export function lastLE(n: number, get: (i: number) => number, x: number): number {
  let lo = 0;
  let hi = n - 1;
  let ans = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (get(mid) <= x) {
      ans = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return ans;
}

export const Player = forwardRef<PlayerHandle, Props>(function Player({ lesson, tokens, pages, page, onPage, onCursor, videoHost, onState, onResynced }, ref) {
  const settings = useApp((s) => s.settings);
  const setSetting = useApp((s) => s.setSetting);
  const hasMedia = !!lesson.media_path;
  const videoPath = lesson.video_path;
  // une seule source (vidéo locale avec son) ou deux (image + son téléchargés séparément)
  const single = !!videoPath && videoPath === lesson.media_path;
  const dual = !!videoPath && !single;

  const audioRef = useRef<HTMLAudioElement>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const master = (): HTMLMediaElement | null => (single ? videoRef.current : audioRef.current);

  const [playing, setPlaying] = useState(false);
  const [time, setTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [voiceName, setVoiceName] = useState<string>("");
  const [ttsPos, setTtsPos] = useState(0);
  const speakRef = useRef<SpeakHandle | null>(null);
  const cursorRef = useRef(-1);
  const pageRef = useRef(page);
  pageRef.current = page;
  const listenSecs = useRef(0);

  // ---------- mémoire de la position, à la seconde près ----------
  // La seconde atteinte est écrite au plus une fois par seconde pendant
  // l'écoute, puis à chaque pause, saut ou sortie : fermer Lumen n'en
  // perd au pire qu'une seconde.
  const posRef = useRef(lesson.position || 0);
  const savedPos = useRef(lesson.position || 0);
  const savedDuration = useRef(lesson.duration || 0);
  // voix du système : mot où la lecture s'était arrêtée
  const resumeRef = useRef(hasMedia ? -1 : lesson.anchor || -1);
  const remember = useCallback(
    (t: number, force = false) => {
      posRef.current = t;
      if (!hasMedia || Math.abs(t - savedPos.current) < (force ? 0.05 : 1)) return;
      savedPos.current = t;
      void api().lessonUpdate(lesson.id, { position: Math.round(t * 10) / 10 });
    },
    [hasMedia, lesson.id],
  );
  useEffect(() => {
    const flush = () => remember(posRef.current, true);
    window.addEventListener("pagehide", flush);
    return () => {
      window.removeEventListener("pagehide", flush);
      flush();
    };
  }, [remember]);

  // ---------- recalage de la lanterne (minutage approximatif) ----------
  const [resync, setResync] = useState<number | null>(null);
  const canResync = hasMedia && !(lesson.timing_v >= 2);
  const recaler = async () => {
    setResync(0);
    try {
      const t = await api().lessonResync(lesson.id, (e) => e.type === "progress" && setResync(e.value));
      onResynced?.(t);
      useApp.getState().toast("La lanterne suit maintenant la voix, mot à mot", "light");
    } catch (e) {
      useApp.getState().toast(errorText(e), "error");
      if (isNoModel(e)) useApp.getState().go("settings");
    } finally {
      setResync(null);
    }
  };

  const rateKey = hasMedia ? "media_rate" : "tts_rate";
  const rate = parseFloat(settings[rateKey] ?? (hasMedia ? "1" : "0.95")) || 1;

  useEffect(() => {
    onState?.({ playing, time, duration });
  }, [playing, time, duration, onState]);

  // ---------- horodatages ----------
  const timing = useMemo(() => {
    if (!lesson.timings) return null;
    try {
      const raw = JSON.parse(lesson.timings) as number[][];
      const words = tokens.map((t, i) => ({ t, i })).filter((x) => x.t.w);
      const tokIdx = raw.map(([s, e]) => {
        const k = lastLE(words.length, (j) => words[j].t.s, Math.max(s, 0));
        const cand = words[k];
        if (cand && cand.t.s >= s && cand.t.s < e) return cand.i;
        const next = words[k + 1];
        if (next && next.t.s < e) return next.i;
        return cand ? cand.i : -1;
      });
      return { raw, tokIdx };
    } catch {
      return null;
    }
  }, [lesson.timings, tokens]);

  const pageOf = useCallback(
    (i: number) => {
      const p = pages.findIndex((r) => i >= r.start && i < r.end);
      return p === -1 ? pageRef.current : p;
    },
    [pages],
  );

  const setCursor = useCallback(
    (i: number) => {
      if (i === cursorRef.current) return;
      cursorRef.current = i;
      onCursor(i);
      if (i >= 0) {
        const p = pageOf(i);
        if (p !== pageRef.current) onPage(p);
      }
    },
    [onCursor, onPage, pageOf],
  );

  // ---------- boucle de synchronisation (texte et image) ----------
  useEffect(() => {
    if (!hasMedia || !playing) return;
    let raf = 0;
    const tick = () => {
      const el = single ? videoRef.current : audioRef.current;
      if (el) {
        const t = el.currentTime;
        setTime(t);
        remember(t);
        if (timing) {
          const k = lastLE(timing.raw.length, (j) => timing.raw[j][2], t + LANTERN_LEAD);
          if (k >= 0) {
            const [, , , t1] = timing.raw[k];
            const nextStart = timing.raw[k + 1]?.[2] ?? Infinity;
            if (t <= t1 + 0.5 || t < nextStart) setCursor(timing.tokIdx[k]);
          }
        }
        // l'image suit le son : petite correction de vitesse, ou saut si l'écart est grand
        const v = videoRef.current;
        if (dual && v && v.readyState >= 2) {
          const d = v.currentTime - t;
          if (Math.abs(d) > 0.35) {
            v.currentTime = t;
            v.playbackRate = rate;
          } else if (Math.abs(d) > 0.06) {
            v.playbackRate = rate * (d > 0 ? 0.96 : 1.04);
          } else if (v.playbackRate !== rate) {
            v.playbackRate = rate;
          }
          if (v.paused) void v.play().catch(() => {});
        }
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [hasMedia, playing, timing, setCursor, single, dual, rate, remember]);

  // temps d'écoute
  useEffect(() => {
    if (!playing) return;
    const id = window.setInterval(() => {
      listenSecs.current += 1;
      if (listenSecs.current >= 30) {
        void api().activityAdd(lesson.lang, 0, listenSecs.current);
        listenSecs.current = 0;
      }
    }, 1000);
    return () => window.clearInterval(id);
  }, [playing, lesson.lang]);

  useEffect(
    () => () => {
      if (listenSecs.current > 0) void api().activityAdd(lesson.lang, 0, listenSecs.current);
      speakRef.current?.stop();
    },
    [lesson.lang],
  );

  // ---------- voix ----------
  useEffect(() => {
    if (hasMedia || !ttsAvailable()) return;
    loadVoices().then(() => {
      const vs = voicesFor(lesson.lang);
      const chosen = vs.find((v) => v.voiceURI === settings[`voice_${lesson.lang}`]) ?? vs[0];
      setVoiceName(chosen ? chosen.name : "");
    });
  }, [hasMedia, lesson.lang, settings]);

  const speakFrom = useCallback(
    (start: number) => {
      speakRef.current?.stop();
      const p = pageOf(start);
      const range = pages[p];
      let a = start;
      while (a < range.end && !tokens[a].w) a++;
      if (a >= range.end) return;
      const base = tokens[a].s;
      const text = lesson.text.slice(base, tokens[range.end - 1].e);
      const words = tokens.map((t, i) => ({ t, i })).filter((x) => x.t.w && x.i >= a && x.i < range.end);
      setPlaying(true);
      setCursor(a);
      speakRef.current = speak(text, {
        lang: lesson.lang,
        voiceURI: settings[`voice_${lesson.lang}`],
        rate,
        onWord(ci) {
          const abs = base + ci;
          const k = lastLE(words.length, (j) => words[j].t.s, abs);
          if (k >= 0) {
            setCursor(words[k].i);
            setTtsPos((words[k].i - range.start) / Math.max(1, range.end - range.start));
          }
        },
        onEnd(completed) {
          speakRef.current = null;
          if (completed && p < pages.length - 1) {
            window.setTimeout(() => {
              onPage(p + 1);
              speakFrom(pages[p + 1].start);
            }, 650);
          } else {
            setPlaying(false);
            if (completed) setCursor(-1);
          }
        },
      });
    },
    [lesson, tokens, pages, rate, settings, onPage, pageOf, setCursor],
  );

  // ---------- commandes ----------
  const seekTo = useCallback(
    (t: number) => {
      const m = master();
      if (!m) return;
      m.currentTime = Math.max(0, t);
      if (dual && videoRef.current) videoRef.current.currentTime = m.currentTime;
      setTime(m.currentTime);
      remember(m.currentTime, true);
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [dual, single, remember],
  );

  const seekToToken = useCallback(
    (i: number) => {
      if (!timing) return;
      const k = timing.tokIdx.findIndex((t) => t >= i);
      const idx = k === -1 ? timing.tokIdx.length - 1 : k;
      seekTo(timing.raw[idx][2] - 0.05);
      setCursor(timing.tokIdx[idx]);
    },
    [timing, seekTo, setCursor],
  );

  const playMedia = () => {
    const m = master();
    if (!m) return;
    void m.play();
    if (dual) void videoRef.current?.play().catch(() => {});
  };
  const pauseMedia = () => {
    master()?.pause();
    if (dual) videoRef.current?.pause();
  };

  const toggle = useCallback(() => {
    if (hasMedia) {
      const m = master();
      if (!m) return;
      if (m.paused) {
        const cur = cursorRef.current;
        const r = pages[pageRef.current];
        if (timing && (cur < r.start || cur >= r.end)) seekToToken(r.start);
        playMedia();
      } else pauseMedia();
      return;
    }
    if (!ttsAvailable()) return;
    if (!playing && voicesFor(lesson.lang).length === 0) {
      useApp.getState().toast("Aucune voix n'est installée pour cette langue. Ajoutez-en une dans Réglages Système › Accessibilité › Contenu énoncé.", "error");
      return;
    }
    if (playing) {
      speakRef.current?.stop();
      setPlaying(false);
    } else {
      const cur = cursorRef.current >= 0 ? cursorRef.current : resumeRef.current;
      const r = pages[pageRef.current];
      speakFrom(cur >= r.start && cur < r.end ? cur : r.start);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasMedia, playing, pages, timing, seekToToken, speakFrom, lesson.lang, single, dual]);

  useImperativeHandle(
    ref,
    () => ({
      toggle,
      playFrom(i: number) {
        if (hasMedia) {
          seekToToken(i);
          playMedia();
        } else speakFrom(i);
      },
      pause() {
        if (hasMedia) pauseMedia();
        else if (playing) {
          speakRef.current?.stop();
          setPlaying(false);
        }
      },
      stop() {
        speakRef.current?.stop();
        pauseMedia();
        setPlaying(false);
      },
      isPlaying: () => playing,
      forget() {
        posRef.current = 0;
        savedPos.current = 0;
        resumeRef.current = -1;
      },
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [toggle, hasMedia, seekToToken, speakFrom, playing],
  );

  useEffect(() => {
    const m = master();
    if (m) {
      m.playbackRate = rate;
      (m as HTMLMediaElement & { preservesPitch?: boolean }).preservesPitch = true;
    }
    if (videoRef.current) videoRef.current.playbackRate = rate;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rate, single]);

  const skip = (d: number) => {
    const m = master();
    if (m) seekTo(Math.min(m.duration || 0, m.currentTime + d));
  };

  const onTimeline = (e: React.MouseEvent<HTMLDivElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    const f = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
    if (hasMedia && duration) {
      seekTo(f * duration);
      if (timing) {
        const k = lastLE(timing.raw.length, (j) => timing.raw[j][2], f * duration);
        if (k >= 0) setCursor(timing.tokIdx[k]);
      }
    } else if (!hasMedia) {
      const range = pages[pageRef.current];
      speakFrom(Math.floor(range.start + f * (range.end - range.start)));
    }
  };

  const progress = hasMedia ? (duration ? time / duration : 0) : ttsPos;
  const rates = hasMedia ? ["0.75", "0.9", "1", "1.25"] : ["0.75", "0.85", "0.95", "1.1"];

  const masterEvents = {
    onPlay: () => setPlaying(true),
    onPause: (e: React.SyntheticEvent<HTMLMediaElement>) => {
      setPlaying(false);
      if (dual) videoRef.current?.pause();
      if (!e.currentTarget.ended) remember(e.currentTarget.currentTime, true);
    },
    // écoutée jusqu'au bout : la prochaine fois, on repart du début
    onEnded: () => {
      setPlaying(false);
      remember(0, true);
    },
    onLoadedMetadata: (e: React.SyntheticEvent<HTMLMediaElement>) => {
      const el = e.currentTarget;
      const d = el.duration || 0;
      setDuration(d);
      el.playbackRate = rate;
      if (d && Math.abs(d - savedDuration.current) > 0.5) {
        savedDuration.current = d;
        void api().lessonUpdate(lesson.id, { duration: Math.round(d * 10) / 10 });
      }
      // reprise exacte là où l'écoute s'était arrêtée
      const p = posRef.current;
      if (p > 0.5 && (!d || p < d - 0.5)) {
        el.currentTime = p;
        if (dual && videoRef.current && videoRef.current.readyState >= 1) videoRef.current.currentTime = p;
        setTime(p);
        if (timing) {
          const k = lastLE(timing.raw.length, (j) => timing.raw[j][2], p + 0.04);
          if (k >= 0) setCursor(timing.tokIdx[k]);
        }
      }
    },
    onTimeUpdate: (e: React.SyntheticEvent<HTMLMediaElement>) => !playing && setTime(e.currentTarget.currentTime),
  };

  const video = videoPath ? (
    <video
      ref={videoRef}
      src={api().mediaUrl(videoPath)}
      preload="auto"
      playsInline
      muted={dual}
      onClick={toggle}
      {...(single ? masterEvents : {})}
      // image seule : elle se cale sur la position reprise du son
      onLoadedMetadata={single ? masterEvents.onLoadedMetadata : (e) => (e.currentTarget.currentTime = posRef.current)}
    />
  ) : null;

  return (
    <>
      {hasMedia && !single && <audio ref={audioRef} src={api().mediaUrl(lesson.media_path!)} preload="auto" {...masterEvents} />}
      {video && videoHost ? createPortal(video, videoHost) : video && <div style={{ display: "none" }}>{video}</div>}
      <div className={`player ${canResync ? "has-resync" : ""}`}>
        <button className={`play-btn ${playing ? "playing" : ""}`} onClick={toggle} aria-label={playing ? "Pause" : "Lecture"} disabled={!hasMedia && !ttsAvailable()}>
          <Icon name={playing ? "pause" : "play"} size={18} />
        </button>
        {hasMedia && (
          <>
            <button className="icon-btn player-skip" onClick={() => skip(-5)} aria-label="Reculer de 5 secondes">
              <Icon name="back5" size={18} />
            </button>
            <button className="icon-btn player-skip" onClick={() => skip(5)} aria-label="Avancer de 5 secondes">
              <Icon name="fwd5" size={18} />
            </button>
            <span className="time">{formatDuration(time)}</span>
          </>
        )}
        <div className="timeline" onClick={onTimeline} role="slider" aria-label="Position" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(progress * 100)}>
          <div className="track">
            <div className="fill" style={{ width: `${progress * 100}%` }} />
            <div className="knob" style={{ left: `${progress * 100}%` }} />
          </div>
        </div>
        {hasMedia && <span className="time player-total">{formatDuration(duration)}</span>}
        <div className="rate-full">
          <Segmented
            id={`rate-${hasMedia ? "m" : "t"}`}
            label="Vitesse"
            value={rates.includes(String(rate)) ? String(rate) : rates[2]}
            onChange={(v) => setSetting(rateKey, v)}
            options={rates.map((r) => ({ value: r, label: `${r.replace(".", ",")}×` }))}
          />
        </div>
        {/* fenêtre étroite : un seul bouton qui passe à la vitesse suivante */}
        <button
          className="btn sm soft rate-compact num"
          onClick={() => setSetting(rateKey, rates[(rates.indexOf(String(rate)) + 1) % rates.length])}
          aria-label="Vitesse de lecture"
          title="Vitesse de lecture"
        >
          {String(rate).replace(".", ",")}×
        </button>
        {canResync &&
          (resync === null ? (
            <button className="btn sm soft player-resync" onClick={recaler} aria-label="Recaler la lanterne" title="Réécoute l'audio pour caler la lanterne sur chaque mot. Le texte ne change pas.">
              <Icon name="sparkle" size={14} /> <span className="resync-label">Recaler la lanterne</span>
            </button>
          ) : (
            <span className="player-resync busy num" role="status">
              <span className="dot busy" /> Calage {Math.round(resync)} %
            </span>
          ))}
        {!hasMedia && <span className="player-label">{voiceName ? `Voix : ${voiceName}` : ttsAvailable() ? "Voix du système" : "Synthèse vocale indisponible"}</span>}
      </div>
    </>
  );
});
