import { forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Icon } from "../../components/Icon";
import { Segmented } from "../../components/ui";
import { api, errorText, isNoModel, isTauri } from "../../lib/api";
import { confirmAsk } from "../../lib/dialogs";
import { t } from "../../lib/i18n";
import { stopPronunciation } from "../../lib/pronounce";
import { playbackRates } from "../../lib/reading";
import { formatDuration, useApp } from "../../lib/store";
import type { PageRange } from "../../lib/tokenize";
import { loadVoices, speak, ttsAvailable, voicesFor, type SpeakHandle } from "../../lib/tts";
import type { Lesson, Token, VoicedLesson } from "../../lib/types";

export interface PlayerHandle {
  toggle(): void;
  playFrom(tokenIndex: number): void;
  pause(): void;
  stop(): void;
  isPlaying(): boolean;
  /** avance ou recule dans l'audio ou la vidéo (secondes) */
  skip(secs: number): void;
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
  /** audio créé par la voix naturelle pour une leçon de texte */
  onVoiced?(v: VoicedLesson): void;
  /** la lecture démarre d'elle-même à l'ouverture (playlist qui s'enchaîne) */
  autoplay?: boolean;
  /** la leçon a été écoutée jusqu'au bout */
  onFinished?(): void;
}

/** Identifiant du modèle de voix naturelle (Réglages › Voix). */
const VOICE_MODEL = "supertonic-3";

/**
 * « Créer l'audio » : la voix naturelle lit toute la leçon, la lanterne suit.
 * Sans voix installée, le premier clic lance son téléchargement.
 */
function MakeAudio({ lesson, again, onVoiced }: { lesson: Lesson; again?: boolean; onVoiced?(v: VoicedLesson): void }) {
  const ready = useApp((s) => s.models.some((m) => m.kind === "tts" && m.installed));
  const dl = useApp((s) => s.downloads[VOICE_MODEL]);
  const [busy, setBusy] = useState<{ stage: string; pct: number } | null>(null);
  const toast = useApp((s) => s.toast);

  const start = async () => {
    if (!isTauri) {
      toast(t("La voix naturelle fonctionne dans l'application Mac.", "The natural voice works in the Mac app."), "error");
      return;
    }
    if (!ready) {
      toast(t("La voix naturelle se télécharge (149 Mo). Vous pourrez ensuite créer l'audio d'un clic.", "The natural voice is downloading (149 MB). Then you can create the audio in one click."), "light");
      void useApp.getState().download(VOICE_MODEL);
      return;
    }
    if (
      again &&
      !(await confirmAsk(
        t("Recréer l'audio de cette leçon avec la voix choisie dans les Réglages ?", "Recreate the audio for this lesson with the voice chosen in Settings?"),
        t("Recréer l'audio", "Recreate the audio"),
        t("Recréer", "Recreate"),
      ))
    )
      return;
    setBusy({ stage: "voice", pct: 0 });
    try {
      const v = await api().lessonVoice(lesson.id, (e) =>
        setBusy((b) => (e.type === "stage" ? { stage: e.stage, pct: b?.pct ?? 0 } : { stage: b?.stage ?? "voice", pct: e.value })),
      );
      onVoiced?.(v);
      useApp.getState().bumpLibrary();
      toast(t("L'audio est prêt : la lanterne suit la voix", "The audio is ready: the lantern follows the voice"), "light");
    } catch (e) {
      const msg = errorText(e);
      if (msg !== "annulé") toast(msg, "error");
    } finally {
      setBusy(null);
    }
  };

  if (busy) {
    return (
      <span className="player-voice busy num" role="status">
        <span className="dot busy" /> {busy.stage === "align" ? t("Calage de la lanterne", "Aligning the lantern") : t("Création de l'audio", "Creating the audio")}{" "}
        {t(`${Math.round(busy.pct)} %`, `${Math.round(busy.pct)}%`)}
        <button
          className="icon-btn"
          onClick={() => void api().lessonVoiceCancel(lesson.id)}
          aria-label={t("Annuler la création de l'audio", "Cancel creating the audio")}
          title={t("Annuler", "Cancel")}
        >
          <Icon name="close" size={12} />
        </button>
      </span>
    );
  }
  if (dl && !dl.error) {
    return (
      <span className="player-voice busy num" role="status">
        <span className="dot busy" /> {t("Voix naturelle", "Natural voice")} {t(`${Math.round((dl.received / Math.max(1, dl.total)) * 100)} %`, `${Math.round((dl.received / Math.max(1, dl.total)) * 100)}%`)}
      </span>
    );
  }
  return again ? (
    <button
      className="icon-btn player-voice"
      onClick={start}
      aria-label={t("Recréer l'audio", "Recreate the audio")}
      title={t("Recréer l'audio avec la voix choisie dans les Réglages", "Recreate the audio with the voice chosen in Settings")}
    >
      <Icon name="wave" size={16} />
    </button>
  ) : (
    <button
      className="btn sm soft player-voice"
      onClick={start}
      title={t("La voix naturelle lit toute la leçon, calculée sur votre Mac ; la lanterne suit chaque mot", "The natural voice reads the whole lesson, computed on your Mac; the lantern follows every word")}
    >
      <Icon name="wave" size={14} /> <span className="voice-label">{t("Créer l'audio", "Create audio")}</span>
    </button>
  );
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

export const Player = forwardRef<PlayerHandle, Props>(function Player({ lesson, tokens, pages, page, onPage, onCursor, videoHost, onState, onResynced, onVoiced, autoplay, onFinished }, ref) {
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
  // pages du moment : elles se recomposent (fenêtre, police) pendant que la voix lit
  const pagesRef = useRef(pages);
  pagesRef.current = pages;
  const listenSecs = useRef(0);
  // lu une seule fois, à l'ouverture de la leçon
  const autoplayRef = useRef(!!autoplay);
  const finishedRef = useRef(onFinished);
  finishedRef.current = onFinished;

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
      const timings = await api().lessonResync(lesson.id, (e) => e.type === "progress" && setResync(e.value));
      onResynced?.(timings);
      useApp.getState().toast(t("La lanterne suit maintenant la voix, mot à mot", "The lantern now follows the voice, word by word"), "light");
    } catch (e) {
      useApp.getState().toast(errorText(e), "error");
      if (isNoModel(e)) useApp.getState().openSettings("ai");
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

  const pageOf = useCallback((i: number) => {
    const p = pagesRef.current.findIndex((r) => i >= r.start && i < r.end);
    return p === -1 ? pageRef.current : p;
  }, []);

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
      // le mot touché se tait : la leçon reprend la parole
      stopPronunciation();
      const p = pageOf(start);
      const range = pagesRef.current[p];
      let a = start;
      while (a < range.end && !tokens[a].w) a++;
      if (a >= range.end) return false;
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
          // la suite : le premier mot après la page lue, dans les pages du moment
          const next = range.end;
          if (completed && next < tokens.length) {
            window.setTimeout(() => {
              onPage(pageOf(next));
              if (speakFromRef.current(next)) return;
              setPlaying(false);
              setCursor(-1);
              finishedRef.current?.();
            }, 650);
          } else {
            setPlaying(false);
            if (completed) {
              setCursor(-1);
              finishedRef.current?.();
            }
          }
        },
      });
      return true;
    },
    [lesson, tokens, rate, settings, onPage, pageOf, setCursor],
  );
  const speakFromRef = useRef(speakFrom);
  speakFromRef.current = speakFrom;

  // ---------- commandes ----------
  const toggleRef = useRef<() => void>(() => {});
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
    // refus possible (lecture sans geste de l'utilisateur) : le lecteur reste simplement en pause
    void m.play().catch(() => {});
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
      useApp
        .getState()
        .toast(
          t(
            "Aucune voix n'est installée pour cette langue. Ajoutez-en une dans Réglages Système › Accessibilité › Contenu énoncé.",
            "No voice is installed for this language. Add one in System Settings › Accessibility › Spoken Content.",
          ),
          "error",
        );
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

  toggleRef.current = toggle;

  // leçon de texte dans une playlist : la voix du système démarre d'elle-même
  useEffect(() => {
    if (!autoplayRef.current || hasMedia || !ttsAvailable()) return;
    let alive = true;
    void loadVoices().then(() => {
      if (!alive || !autoplayRef.current) return;
      autoplayRef.current = false;
      toggleRef.current();
    });
    return () => {
      alive = false;
    };
  }, [hasMedia]);

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
      skip: (secs: number) => skip(secs),
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
  const rates = playbackRates(hasMedia);

  const masterEvents = {
    onPlay: () => {
      // le mot touché se tait : la leçon reprend la parole (bouton, Espace ou touches du Mac)
      stopPronunciation();
      setPlaying(true);
    },
    onPause: (e: React.SyntheticEvent<HTMLMediaElement>) => {
      setPlaying(false);
      if (dual) videoRef.current?.pause();
      if (!e.currentTarget.ended) remember(e.currentTarget.currentTime, true);
    },
    // écoutée jusqu'au bout : la prochaine fois, on repart du début
    onEnded: () => {
      setPlaying(false);
      remember(0, true);
      finishedRef.current?.();
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
      if (autoplayRef.current) {
        autoplayRef.current = false;
        playMedia();
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
      <div className={`player ${canResync ? "has-resync" : ""}`} data-tour="player">
        <button className={`play-btn ${playing ? "playing" : ""}`} onClick={toggle} aria-label={playing ? t("Pause", "Pause") : t("Lecture", "Play")} disabled={!hasMedia && !ttsAvailable()}>
          <Icon name={playing ? "pause" : "play"} size={18} />
        </button>
        {hasMedia && (
          <>
            <button className="icon-btn player-skip" onClick={() => skip(-5)} aria-label={t("Reculer de 5 secondes", "Back 5 seconds")}>
              <Icon name="back5" size={18} />
            </button>
            <button className="icon-btn player-skip" onClick={() => skip(5)} aria-label={t("Avancer de 5 secondes", "Forward 5 seconds")}>
              <Icon name="fwd5" size={18} />
            </button>
            <span className="time">{formatDuration(time)}</span>
          </>
        )}
        <div className="timeline" onClick={onTimeline} role="slider" aria-label={t("Position", "Position")} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(progress * 100)}>
          <div className="track">
            <div className="fill" style={{ width: `${progress * 100}%` }} />
            <div className="knob" style={{ left: `${progress * 100}%` }} />
          </div>
        </div>
        {hasMedia && <span className="time player-total">{formatDuration(duration)}</span>}
        <div className="rate-full">
          <Segmented
            id={`rate-${hasMedia ? "m" : "t"}`}
            label={t("Vitesse", "Speed")}
            value={rates.includes(String(rate)) ? String(rate) : rates[2]}
            onChange={(v) => setSetting(rateKey, v)}
            options={rates.map((r) => ({ value: r, label: `${t(r.replace(".", ","), r)}×` }))}
          />
        </div>
        {/* fenêtre étroite : un seul bouton qui passe à la vitesse suivante */}
        <button
          className="btn sm soft rate-compact num"
          onClick={() => setSetting(rateKey, rates[(rates.indexOf(String(rate)) + 1) % rates.length])}
          aria-label={t("Vitesse de lecture", "Playback speed")}
          title={t("Vitesse de lecture", "Playback speed")}
        >
          {t(String(rate).replace(".", ","), String(rate))}×
        </button>
        {canResync &&
          (resync === null ? (
            <button className="btn sm soft player-resync" onClick={recaler} aria-label={t("Recaler la lanterne", "Realign the lantern")}
              title={t("Réécoute l'audio pour caler la lanterne sur chaque mot. Le texte ne change pas.", "Listens to the audio again to align the lantern on every word. The text doesn't change.")}
            >
              <Icon name="sparkle" size={14} /> <span className="resync-label">{t("Recaler la lanterne", "Realign the lantern")}</span>
            </button>
          ) : (
            <span className="player-resync busy num" role="status">
              <span className="dot busy" /> {t(`Calage ${Math.round(resync)} %`, `Aligning ${Math.round(resync)}%`)}
            </span>
          ))}
        {(!hasMedia || lesson.media_path?.includes(".voice.")) && <MakeAudio lesson={lesson} again={hasMedia} onVoiced={onVoiced} />}
        {!hasMedia && <span className="player-label">{voiceName ? t(`Voix : ${voiceName}`, `Voice: ${voiceName}`) : ttsAvailable() ? t("Voix du système", "System voice") : t("Synthèse vocale indisponible", "Speech synthesis unavailable")}</span>}
      </div>
    </>
  );
});
