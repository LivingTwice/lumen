import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useEffect, useMemo, useRef, useState } from "react";
import { api, errorText } from "../lib/api";
import { formatWhen } from "../lib/backup";
import { ago, levelRange, openSource, useDiscover } from "../lib/discover";
import { count, formatNumber, t } from "../lib/i18n";
import { extractArticle, firstWords, wordCount } from "../lib/importers";
import { stageText, useImports, useJob } from "../lib/imports";
import { LANGS, inLang } from "../lib/langs";
import { jobOf, usePreview, type Previewable } from "../lib/preview";
import { formatBytes, formatDuration, useApp } from "../lib/store";
import type { LangCode, Lyrics, MediaStream, TextStats } from "../lib/types";
import { CoverArt } from "./Cover";
import { Icon, type IconName } from "./Icon";
import { Orb } from "./ui";

// ---------- lecture directe : adresses gardées le temps de la session ----------

/** Les adresses de YouTube expirent au bout de quelques heures : renouvelées après 40 minutes. */
const STREAM_TTL = 40 * 60 * 1000;
const streams = new Map<string, { at: number; p: Promise<MediaStream> }>();

/** Adresses de lecture d'une vidéo ou d'un son (préparées dès le survol d'une carte). */
export function prefetchStream(url: string, audio: boolean): Promise<MediaStream> {
  const key = `${audio}:${url}`;
  const known = streams.get(key);
  if (known && Date.now() - known.at < STREAM_TTL) return known.p;
  const p = api().mediaStream(url, audio);
  streams.set(key, { at: Date.now(), p });
  // un échec se retente à la prochaine ouverture
  p.catch(() => streams.delete(key));
  return p;
}

const songs = new Map<string, Promise<string>>();
function findSong(artist: string, title: string): Promise<string> {
  const key = `${artist}|${title}`.toLowerCase();
  let p = songs.get(key);
  if (!p) {
    p = api().songFind(artist, title);
    songs.set(key, p);
    p.catch(() => songs.delete(key));
  }
  return p;
}

type Load<T> = { state: "loading" } | { state: "ready"; value: T } | { state: "error"; error: string };

/** Ce qu'il faut lire : image et son, ou son seul. */
function useSource(item: Previewable): Load<MediaStream> {
  const [load, setLoad] = useState<Load<MediaStream>>({ state: "loading" });
  useEffect(() => {
    let alive = true;
    const done = (value: MediaStream) => alive && setLoad({ state: "ready", value });
    const fail = (e: unknown) => alive && setLoad({ state: "error", error: errorText(e) });
    const plain = (video: string, audio: string, duration = 0): MediaStream => ({ video, audio, width: 0, height: 0, language: "", title: "", description: "", author: "", duration, published: 0, count: 0 });
    if (item.kind === "text") return;
    if (item.direct) done(item.kind === "video" ? plain(item.url, "") : plain("", item.url));
    else if (item.song && !item.song.video)
      findSong(item.song.artist, item.song.title)
        .then((u) => prefetchStream(u, true))
        .then(done)
        // à défaut, l'extrait de 30 secondes
        .catch((e) => (item.song?.sample ? done(plain("", item.song.sample, 30)) : fail(e)));
    else prefetchStream(item.url, item.kind === "audio").then(done).catch(fail);
    return () => {
      alive = false;
    };
  }, [item]);
  return load;
}

// ---------- paroles minutées ----------

interface Line {
  t: number;
  text: string;
}

/** Lignes d'un texte LRC (miroir de `lyrics::parse_lrc`, sans décalage). */
function parseLrc(s: string): Line[] {
  const out: Line[] = [];
  for (const raw of s.split("\n")) {
    let rest = raw.trim();
    const times: number[] = [];
    let m: RegExpMatchArray | null;
    while ((m = rest.match(/^\[(\d+):(\d+(?:[.,]\d+)?)\]\s*/))) {
      times.push(Number(m[1]) * 60 + Number(m[2].replace(",", ".")));
      rest = rest.slice(m[0].length);
    }
    for (const time of times) out.push({ t: time, text: rest.trim() });
  }
  return out.sort((a, b) => a.t - b.t);
}

function lyricsText(l: Lyrics): string {
  return l.plain.trim() || parseLrc(l.synced).map((x) => x.text).join("\n");
}

function LyricsView({ lyrics, time, live }: { lyrics: Lyrics; time: number; live: boolean }) {
  const lines = useMemo(() => (lyrics.synced ? parseLrc(lyrics.synced) : lyrics.plain.split("\n").map((text) => ({ t: -1, text: text.trim() }))), [lyrics]);
  const box = useRef<HTMLDivElement>(null);
  const still = useReducedMotion();
  let at = -1;
  if (live && lyrics.synced) for (let i = 0; i < lines.length; i++) if (lines[i].t <= time + 0.2) at = i;
  useEffect(() => {
    const el = box.current?.querySelector<HTMLElement>(`[data-l="${at}"]`);
    const c = box.current;
    if (!el || !c) return;
    c.scrollTo({ top: el.offsetTop - c.clientHeight * 0.38, behavior: still ? "auto" : "smooth" });
  }, [at, still]);
  return (
    <div className={`pv-lyrics ${live && lyrics.synced ? "live" : ""}`} ref={box} dir="auto">
      {lines.map((l, i) =>
        l.text ? (
          <p key={i} data-l={i} className={i === at ? "on" : i < at ? "past" : ""}>
            {l.text}
          </p>
        ) : (
          <span key={i} className="pv-lyrics-gap" />
        ),
      )}
    </div>
  );
}

// ---------- lecteur ----------

const SPEEDS = [1, 0.85, 0.7];

/** Vidéo (muette si le son est à part, qu'elle suit) ou son seul, avec ses commandes. */
function Player({ source, poster, cover, known, onTime }: { source: MediaStream; poster: string; cover?: React.ReactNode; known: number; onTime?(t: number): void }) {
  const video = useRef<HTMLVideoElement>(null);
  const audio = useRef<HTMLAudioElement>(null);
  const dual = !!source.video && !!source.audio;
  const master = () => (source.audio ? audio.current : video.current);
  const [playing, setPlaying] = useState(false);
  const [time, setTime] = useState(0);
  const [length, setLength] = useState(0);
  const [muted, setMuted] = useState(false);
  const [speed, setSpeed] = useState(1);
  const [waiting, setWaiting] = useState(true);
  const [failed, setFailed] = useState(false);

  // le son est le maître : la vidéo muette le suit (même correction que dans les leçons)
  useEffect(() => {
    if (!dual) return;
    let raf = 0;
    const tick = () => {
      const a = audio.current;
      const v = video.current;
      if (a && v && v.readyState >= 2) {
        const d = v.currentTime - a.currentTime;
        if (Math.abs(d) > 0.35) {
          v.currentTime = a.currentTime;
          v.playbackRate = speed;
        } else if (Math.abs(d) > 0.06) v.playbackRate = speed * (d > 0 ? 0.96 : 1.04);
        else if (v.playbackRate !== speed) v.playbackRate = speed;
        if (!a.paused && v.paused) void v.play().catch(() => {});
        if (a.paused && !v.paused) v.pause();
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [dual, speed]);

  useEffect(() => {
    const m = master();
    if (m) m.playbackRate = speed;
    if (video.current) video.current.playbackRate = speed;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [speed]);

  const toggle = () => {
    const m = master();
    if (!m) return;
    if (m.paused) void m.play().catch(() => {});
    else m.pause();
  };
  const seek = (to: number) => {
    const m = master();
    if (!m) return;
    m.currentTime = Math.max(0, Math.min(to, m.duration || to));
    if (dual && video.current) video.current.currentTime = m.currentTime;
    setTime(m.currentTime);
  };

  // Espace : lecture ou pause ; flèches : cinq secondes
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLElement && e.target.closest("input, textarea, button")) return;
      if (e.key === " ") {
        e.preventDefault();
        toggle();
      } else if (e.key === "ArrowLeft") seek((master()?.currentTime ?? 0) - 5);
      else if (e.key === "ArrowRight") seek((master()?.currentTime ?? 0) + 5);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const events = {
    onPlay: () => setPlaying(true),
    onPause: () => setPlaying(false),
    onTimeUpdate: (e: React.SyntheticEvent<HTMLMediaElement>) => {
      setTime(e.currentTarget.currentTime);
      onTime?.(e.currentTarget.currentTime);
    },
    onLoadedMetadata: (e: React.SyntheticEvent<HTMLMediaElement>) => setLength(e.currentTarget.duration || 0),
    onWaiting: () => setWaiting(true),
    onPlaying: () => setWaiting(false),
    onCanPlay: () => setWaiting(false),
    onError: () => setFailed(true),
  };

  // le son de YouTube annonce parfois une durée double : celle du site fait foi
  const total = known > 0 ? known : length;
  const pct = total ? Math.min(100, (time / total) * 100) : 0;
  return (
    <div className={`pv-player ${source.video ? "has-video" : "sound"} ${playing ? "playing" : "paused"}`}>
      {/* la lumière de l'image déborde doucement dans les bandes autour de la vidéo */}
      {source.video && poster && <img className="pv-ambient" src={poster} alt="" aria-hidden="true" />}
      {source.video ? (
        <video
          ref={video}
          className="pv-video"
          src={source.video}
          poster={poster || undefined}
          playsInline
          autoPlay={!dual}
          muted={dual || muted}
          onClick={toggle}
          {...(dual ? { onError: events.onError } : events)}
        />
      ) : (
        cover
      )}
      {source.audio && <audio ref={audio} src={source.audio} autoPlay muted={muted} {...events} />}
      {failed && (
        <div className="pv-failed">
          <Icon name="ban" size={18} /> {t("La lecture n'a pas pu commencer.", "Playback couldn't start.")}
        </div>
      )}
      {!failed && waiting && !playing && (
        <div className="pv-buffer" aria-hidden="true">
          <Orb size={34} />
        </div>
      )}
      {!playing && !waiting && !failed && (
        <button className="pv-bigplay" onClick={toggle} aria-label={t("Lecture", "Play")}>
          <Icon name="play" size={30} />
        </button>
      )}
      <div className="pv-controls">
        <button className="pv-ctl" onClick={toggle} aria-label={playing ? t("Pause", "Pause") : t("Lecture", "Play")}>
          <Icon name={playing ? "pause" : "play"} size={17} />
        </button>
        <span className="pv-time num">
          {formatDuration(time)} <i>/</i> {total ? formatDuration(total) : "–:––"}
        </span>
        <label className="pv-scrub" style={{ ["--p" as string]: `${pct}%` }}>
          <input type="range" min={0} max={total || 1} step={0.1} value={Math.min(time, total || time)} onChange={(e) => seek(Number(e.target.value))} aria-label={t("Position", "Position")} />
        </label>
        <button
          className="pv-ctl pv-speed num"
          onClick={() => setSpeed((s) => SPEEDS[(SPEEDS.indexOf(s) + 1) % SPEEDS.length])}
          title={t("Vitesse de lecture", "Playback speed")}
          aria-label={t("Vitesse de lecture", "Playback speed")}
        >
          {speed === 1 ? "1×" : `${String(speed).replace(".", t(",", "."))}×`}
        </button>
        <button className="pv-ctl" onClick={() => setMuted((m) => !m)} aria-label={muted ? t("Remettre le son", "Unmute") : t("Couper le son", "Mute")}>
          <Icon name={muted ? "mute" : "speaker"} size={17} />
        </button>
      </div>
    </div>
  );
}

// ---------- ce que montre l'aperçu ----------

function platformLabel(p: Previewable): { icon: IconName; text: string } {
  switch (p.platform) {
    case "youtube":
      return { icon: "youtube", text: "YouTube" };
    case "dailymotion":
      return { icon: "video", text: "Dailymotion" };
    case "podcast":
      return { icon: "podcast", text: "Podcast" };
    case "music":
      return { icon: "music", text: t("Chanson", "Song") };
    case "wiki":
      return { icon: "globe", text: p.author };
    default:
      if (p.kind === "song") return { icon: "music", text: t("Chanson", "Song") };
      if (p.kind === "text") return { icon: "text", text: t("Article", "Article") };
      return { icon: p.kind === "audio" ? "podcast" : "video", text: p.kind === "audio" ? "Podcast" : t("Vidéo", "Video") };
  }
}

function langName(code: string): string {
  return LANGS.find((l) => l.code === code)?.name ?? code.toUpperCase();
}

/** Lire sur le site : YouTube, le site de l'épisode, l'article. */
function sourceLabel(p: Previewable): string {
  if (p.platform === "youtube" || /youtu\.?be/.test(p.page)) return t("Voir sur YouTube", "Watch on YouTube");
  if (p.platform === "dailymotion") return t("Voir sur Dailymotion", "Watch on Dailymotion");
  if (p.platform === "music") return t("Ouvrir sur Deezer", "Open on Deezer");
  return p.kind === "text" ? t("Lire sur le site", "Read on the site") : t("Ouvrir sur le site", "Open on the site");
}

/** Ce que devient l'élément une fois choisi, dit simplement. */
function whatHappens(p: Previewable, synced: boolean): string {
  if (p.kind === "text") return t("L'article devient une leçon de lecture. Trop riche ? « Simplifier » le réécrit à votre niveau.", "The article becomes a reading lesson. Too rich? “Simplify” rewrites it at your level.");
  if (p.kind === "song")
    return synced
      ? t("Les paroles deviennent le texte de la leçon, et la lanterne suit la chanson, ligne après ligne.", "The lyrics become the lesson's text, and the lantern follows the song, line after line.")
      : t("Les paroles deviennent le texte de la leçon ; la lanterne se cale sur la voix si Whisper l'entend.", "The lyrics become the lesson's text; the lantern syncs with the voice if Whisper can hear it.");
  if (p.pageText) return t("Le son et le texte publié par la source : la lanterne suit la voix, mot à mot.", "The sound and the text published by the source: the lantern follows the voice, word by word.");
  return p.kind === "video"
    ? t("Lumen télécharge la vidéo, la transcrit sur votre Mac et cale chaque mot sur la voix.", "Lumen downloads the video, transcribes it on your Mac and syncs every word with the voice.")
    : t("Lumen télécharge le son, le transcrit sur votre Mac et cale chaque mot sur la voix.", "Lumen downloads the sound, transcribes it on your Mac and syncs every word with the voice.");
}

/** Part de mots connus : une jauge qui s'éclaire. */
function KnownMeter({ stats }: { stats: TextStats }) {
  return (
    <div className="pv-known">
      <div className="pv-known-head">
        <strong className="num">{t(`${stats.known_pct} %`, `${stats.known_pct}%`)}</strong>
        <span>{t("des mots déjà connus ou en cours", "of the words already known or learning")}</span>
      </div>
      <div className="pv-known-bar">
        <i style={{ width: `${Math.max(3, stats.known_pct)}%` }} />
      </div>
      <span className="pv-known-foot num">
        {count(stats.new_words, "mot nouveau", "mots nouveaux", "new word", "new words")} · {count(stats.words, "mot", "mots", "word", "words")}
      </span>
    </div>
  );
}

/** Le modèle de transcription manque : on le propose, la leçon attendra son arrivée. */
function TranscriberPrompt() {
  const models = useApp((s) => s.models);
  const downloads = useApp((s) => s.downloads);
  const download = useApp((s) => s.download);
  const m = models.find((x) => x.kind === "asr" && x.id === "whisper-turbo") ?? models.find((x) => x.kind === "asr");
  if (!m) return null;
  const d = downloads[m.id];
  return (
    <div className="pv-note">
      <Icon name="cpu" size={17} />
      <div>
        <strong>{t("Un modèle de transcription est nécessaire", "A transcription model is needed")}</strong>
        <span>
          {t(`${m.name}, ${formatBytes(m.size)}, une seule fois. La leçon se préparera dès qu'il sera là.`, `${m.name}, ${formatBytes(m.size)}, just once. The lesson will be prepared as soon as it's here.`)}
        </span>
        {d ? (
          <div className="bar live" style={{ marginTop: 8 }}>
            <i style={{ width: `${(d.received / d.total) * 100}%` }} />
          </div>
        ) : (
          <button className="btn sm accent" onClick={() => download(m.id)}>
            <Icon name="download" size={14} /> {t("Télécharger", "Download")}
          </button>
        )}
      </div>
    </div>
  );
}

function PreviewPanel({ item, onClose }: { item: Previewable; onClose(): void }) {
  const lang = useApp((s) => s.lang)() as LangCode;
  const openLesson = useApp((s) => s.openLesson);
  const openImportItem = useApp((s) => s.openImportItem);
  const hasAsr = useApp((s) => s.models.some((m) => m.kind === "asr" && m.installed));
  const enqueue = useImports((s) => s.enqueue);
  const retry = useImports((s) => s.retry);
  const hide = useDiscover((s) => s.hide);
  const job = useJob(item.key);
  const source = useSource(item);
  const [time, setTime] = useState(0);
  const [more, setMore] = useState(false);
  const [asked, setAsked] = useState(false);

  // chanson : les paroles (déjà là, ou cherchées)
  const [lyrics, setLyrics] = useState<Lyrics | null | undefined>(item.song ? (item.song.lyrics ?? undefined) : null);
  useEffect(() => {
    if (!item.song || lyrics !== undefined) return;
    let alive = true;
    api()
      .lyricsFind(item.song.artist, item.song.title, item.song.album, item.duration)
      .then((l) => alive && setLyrics(l))
      .catch(() => alive && setLyrics(null));
    return () => {
      alive = false;
    };
  }, [item, lyrics]);

  // article : le texte de la page
  const [article, setArticle] = useState<Load<{ title: string; text: string }>>({ state: "loading" });
  const [whole, setWhole] = useState(false);
  useEffect(() => {
    if (item.kind !== "text") return;
    let alive = true;
    api()
      .linkProbe(item.url, () => {})
      .then((info) => {
        if (!info.html) throw t("Aucun article lisible n'a été trouvé sur cette page.", "No readable article was found on this page.");
        const a = extractArticle(info.html, info.url);
        if (alive) setArticle({ state: "ready", value: { title: item.title || a.title, text: a.text } });
      })
      .catch((e) => alive && setArticle({ state: "error", error: errorText(e) }));
    return () => {
      alive = false;
    };
  }, [item]);

  const articleText = article.state === "ready" ? article.value.text : "";
  const longArticle = wordCount(articleText) > 2600;
  const chosenText = longArticle && !whole ? firstWords(articleText, 1500) : articleText;

  // ce que l'apprenant connaît déjà du texte (article, paroles)
  const statsText = item.kind === "text" ? chosenText : lyrics ? lyricsText(lyrics) : "";
  const [stats, setStats] = useState<TextStats | null>(null);
  useEffect(() => {
    if (!statsText) return setStats(null);
    let alive = true;
    api()
      .textStats(lang, statsText)
      .then((s) => alive && setStats(s))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [lang, statsText]);

  const stream = source.state === "ready" ? source.value : null;
  const spoken = stream?.language && stream.language !== lang && LANGS.some((l) => l.code === stream.language) ? stream.language : "";
  const other = spoken || (item.inLang === false ? item.otherLang : "");
  const lesson = item.discover?.lesson_id ?? (job?.status === "done" ? job.lessonId : undefined);
  const synced = !!lyrics?.synced;
  const description = stream?.description || item.summary;
  const label = platformLabel(item);
  const needsAsr = (item.kind === "video" || item.kind === "audio") && !hasAsr;

  // la vidéo YouTube de la chanson, une fois trouvée (l'import ne la cherche pas une seconde fois)
  const resolvedSong = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (item.song && !item.song.video) void findSong(item.song.artist, item.song.title).then((u) => (resolvedSong.current = u)).catch(() => {});
  }, [item]);
  const make = () => {
    if (item.kind === "text" && article.state !== "ready") return;
    setAsked(true);
    enqueue(
      jobOf(item, lang, {
        article: article.state === "ready" ? { title: article.value.title, text: chosenText } : undefined,
        songUrl: resolvedSong.current,
        lyrics: lyrics ?? null,
      }),
    );
  };

  const facts = [
    item.author,
    item.duration > 0 ? formatDuration(item.duration) : "",
    item.count > 0 && (item.platform === "youtube" || item.platform === "dailymotion") ? t(`${formatNumber(item.count)} vues`, `${formatNumber(item.count)} views`) : "",
    item.published ? (Date.now() / 1000 - item.published < 35 * 86400 ? ago(item.published) : formatWhen(item.published)) : "",
    item.kind === "text" && articleText ? count(wordCount(articleText), "mot", "mots", "word", "words") : "",
  ].filter(Boolean);

  // ---------- la scène ----------
  let stage: React.ReactNode;
  if (item.kind === "text") {
    stage = (
      <div className="pv-read">
        {article.state === "loading" && (
          <div className="pv-read-ghost" aria-hidden="true">
            {[92, 100, 96, 70, 0, 98, 100, 88, 94, 60].map((w, i) => (w ? <i key={i} style={{ width: `${w}%` }} /> : <br key={i} />))}
          </div>
        )}
        {article.state === "error" && (
          <div className="pv-failed static">
            <Icon name="ban" size={18} /> {article.error}
          </div>
        )}
        {article.state === "ready" && (
          <article dir="auto">
            <h3>{article.value.title}</h3>
            {firstWords(articleText, 700)
              .split("\n\n")
              .map((p, i) => (
                <p key={i}>{p}</p>
              ))}
          </article>
        )}
      </div>
    );
  } else {
    const cover = (
      <div className="pv-cover">
        {item.image ? <img className="pv-cover-back" src={item.image} alt="" aria-hidden="true" /> : null}
        <div className="pv-cover-art">
          {item.image ? <img src={item.image} alt="" /> : <CoverArt seed={item.key.length * 7919} hue={(item.title.length * 37) % 360} />}
        </div>
        {item.song && lyrics && <LyricsView lyrics={lyrics} time={time} live={!!stream?.audio && !stream.audio.includes("dzcdn")} />}
      </div>
    );
    stage =
      source.state === "ready" ? (
        <Player source={source.value} poster={item.image} cover={cover} known={source.value.duration || item.duration} onTime={setTime} />
      ) : (
        <div className={`pv-player waiting ${item.kind === "video" ? "has-video" : "sound"}`}>
          {item.kind === "video" ? item.image && <img className="pv-poster" src={item.image} alt="" /> : cover}
          {source.state === "loading" ? (
            <div className="pv-buffer">
              <Orb size={36} />
              <span>{item.kind === "video" ? t("Préparation de la vidéo…", "Getting the video ready…") : t("Préparation du son…", "Getting the sound ready…")}</span>
            </div>
          ) : (
            <div className="pv-failed">
              <Icon name="ban" size={18} /> {source.error}
            </div>
          )}
        </div>
      );
  }

  // ---------- l'action principale ----------
  let action: React.ReactNode;
  if (lesson) {
    action = (
      <button
        className="btn primary lg glow pv-make"
        onClick={() => {
          onClose();
          openLesson(lesson);
        }}
      >
        <Icon name="book" size={17} /> {t("Ouvrir la leçon", "Open the lesson")}
      </button>
    );
  } else if (job && (job.status === "running" || job.status === "waiting" || job.status === "model")) {
    const pct = job.progress ?? null;
    action = (
      <div className="pv-making" role="status">
        <div className="pv-making-head">
          <Orb size={18} />
          <strong>{job.status === "running" ? stageText(job.stage) : job.status === "model" ? t("En attente du modèle de transcription", "Waiting for the transcription model") : t("En attente de son tour", "Waiting for its turn")}</strong>
          {pct !== null && <span className="num">{t(`${Math.round(pct)} %`, `${Math.round(pct)}%`)}</span>}
        </div>
        <div className={`bar live ${pct === null ? "indeterminate" : ""}`}>
          <i style={{ width: `${pct ?? 35}%` }} />
        </div>
        <span className="pv-making-hint">{t("Vous pouvez fermer : la leçon se prépare pendant que vous explorez.", "You can close this: the lesson gets ready while you explore.")}</span>
      </div>
    );
  } else {
    const blocked = (item.kind === "text" && article.state !== "ready") || (item.kind === "song" && lyrics === null);
    action = (
      <>
        {job?.status === "error" && (
          <div className="pv-note error">
            <Icon name="ban" size={16} />
            <div>
              <span>{job.error}</span>
            </div>
          </div>
        )}
        <button className="btn primary lg glow pv-make" disabled={blocked} onClick={() => (job?.status === "error" ? retry(item.key) : make())}>
          <Icon name={job?.status === "error" ? "refresh" : "sparkle"} size={17} />
          {job?.status === "error" ? t("Réessayer", "Try again") : t("En faire une leçon", "Make it a lesson")}
        </button>
      </>
    );
  }

  return (
    <motion.div
      className="pv-scrim"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
    >
      <motion.div
        className={`pv pv-kind-${item.kind}`}
        role="dialog"
        aria-modal="true"
        aria-label={item.title}
        initial={{ opacity: 0, y: 26, scale: 0.97 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        exit={{ opacity: 0, y: 14, scale: 0.98 }}
        transition={{ type: "spring", stiffness: 340, damping: 32 }}
      >
        <button className="icon-btn pv-close" onClick={onClose} aria-label={t("Fermer", "Close")}>
          <Icon name="close" />
        </button>
        <div className="pv-stage">{stage}</div>
        <aside className="pv-side">
          <span className="pv-platform">
            <Icon name={label.icon} size={13} /> {label.text}
            {item.lo > 0 && <span className={`lv-chip lv-${item.hi}`}>{levelRange(item)}</span>}
          </span>
          <h2 className="pv-title" dir="auto">
            {item.title}
          </h2>
          {facts.length > 0 && <p className="pv-facts num">{facts.join(" · ")}</p>}

          {other && (
            <div className="pv-note">
              <Icon name="globe" size={16} />
              <div>
                <span>
                  {t(`Semble être en ${langName(other).toLowerCase()}, pas ${inLang(lang)}.`, `Seems to be in ${langName(other)}, not ${langName(lang)}.`)}
                </span>
              </div>
            </div>
          )}

          {item.song && (
            <div className={`pv-note ${lyrics ? "glow" : ""}`}>
              <Icon name="lyrics" size={17} />
              <div>
                <strong>
                  {lyrics === undefined
                    ? t("Recherche des paroles…", "Looking for the lyrics…")
                    : lyrics
                      ? synced
                        ? t("Paroles minutées", "Timed lyrics")
                        : t("Paroles trouvées", "Lyrics found")
                      : t("Pas de paroles trouvées", "No lyrics found")}
                </strong>
                <span>
                  {lyrics === null
                    ? t("Sans paroles, pas de leçon de chanson. Essayez une autre version.", "Without lyrics, no song lesson. Try another version.")
                    : whatHappens(item, synced)}
                </span>
              </div>
            </div>
          )}

          {item.song?.video && lyrics && (
            <div className="pv-side-lyrics">
              <LyricsView lyrics={lyrics} time={time} live={false} />
            </div>
          )}

          {item.kind === "text" && longArticle && (
            <div className="pv-choice" role="radiogroup" aria-label={t("Longueur", "Length")}>
              <button role="radio" aria-checked={!whole} className={!whole ? "on" : ""} onClick={() => setWhole(false)}>
                <strong>{t("Le début", "The beginning")}</strong>
                <span className="num">{count(wordCount(firstWords(articleText, 1500)), "mot", "mots", "word", "words")}</span>
              </button>
              <button role="radio" aria-checked={whole} className={whole ? "on" : ""} onClick={() => setWhole(true)}>
                <strong>{t("Tout l'article", "The whole article")}</strong>
                <span className="num">{count(Math.ceil(wordCount(articleText) / 2600), "leçon", "leçons", "lesson", "lessons")}</span>
              </button>
            </div>
          )}

          {stats && stats.words > 0 && <KnownMeter stats={stats} />}

          {description && item.kind !== "text" && (
            <div className={`pv-desc ${more ? "open" : ""}`} dir="auto">
              <p>{description}</p>
              {description.length > 240 && (
                <button className="disc-link" onClick={() => setMore((m) => !m)}>
                  {more ? t("Moins", "Less") : t("Plus", "More")}
                </button>
              )}
            </div>
          )}

          <div className="pv-actions">
            {needsAsr && !lesson && (!job || job.status === "model") && <TranscriberPrompt />}
            {action}
            {!lesson && !item.song && !job && <p className="pv-hint">{whatHappens(item, synced)}</p>}
            {asked && job?.status === "done" && !item.discover?.lesson_id && <p className="pv-hint">{t("La leçon est prête dans votre bibliothèque.", "The lesson is ready in your library.")}</p>}
            <div className="pv-links">
              {item.page && (
                <button className="btn ghost sm" onClick={() => void openSource(item.page)}>
                  <Icon name="external" size={14} /> {sourceLabel(item)}
                </button>
              )}
              {item.discover && !item.song && !lesson && (
                <button
                  className="btn ghost sm"
                  onClick={() => {
                    onClose();
                    openImportItem(item.discover!);
                  }}
                >
                  <Icon name="more" size={14} /> {t("Plus d'options", "More options")}
                </button>
              )}
              {item.discover && !lesson && (
                <button
                  className="btn ghost sm"
                  onClick={() => {
                    onClose();
                    void hide(lang, item.discover!);
                  }}
                >
                  <Icon name="ban" size={14} /> {t("Ne plus proposer", "Don't suggest again")}
                </button>
              )}
            </div>
          </div>
        </aside>
      </motion.div>
    </motion.div>
  );
}

/** L'aperçu ouvert, par-dessus tout le reste. Échap le ferme. */
export function Preview() {
  const item = usePreview((s) => s.item);
  const close = usePreview((s) => s.close);
  useEffect(() => {
    if (!item) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [item, close]);
  return <AnimatePresence>{item && <PreviewPanel key={item.key} item={item} onClose={close} />}</AnimatePresence>;
}
