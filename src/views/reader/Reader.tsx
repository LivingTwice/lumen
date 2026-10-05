import { AnimatePresence, motion, useAnimationControls } from "motion/react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Icon } from "../../components/Icon";
import { Duration, Orb, Sheet } from "../../components/ui";
import { api, errorText, isNoModel } from "../../lib/api";
import { count, t } from "../../lib/i18n";
import { LEVELS, langInfo } from "../../lib/langs";
import { pronounce } from "../../lib/pronounce";
import { studyTime, useStudyClock } from "../../lib/progress";
import { formatNumber, useApp } from "../../lib/store";
import { fitPages, pageOfToken } from "../../lib/pagefit";
import { readerLook } from "../../lib/reading";
import { paginate, sentenceBounds, type PageRange } from "../../lib/tokenize";
import type { LessonSummary, OpenedLesson, Term, Token } from "../../lib/types";
import { Player, type PlayerHandle, type PlaybackState } from "./Player";
import { useChat } from "../../lib/chat";
import { chooseCover } from "../../lib/covers";
import { confirmAsk } from "../../lib/dialogs";
import { useLessonMenu, useMenu } from "../../lib/menu";
import { AddToPlaylist } from "../../components/AddToPlaylist";
import { DisplayMenu } from "./Display";
import { PlaylistStrip, UpNext, usePlaylist } from "./PlaylistBar";
import { AsideTabs, ReaderChat, type AsideTab } from "./ReaderChat";
import { VideoStage } from "./VideoStage";
import { EXPR_MAX_WORDS, WordPanel, type Selection } from "./WordPanel";

interface Range {
  a: number;
  b: number;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
/** Hauteur (part de la zone visible) de la ligne où l'œil lit : le point de reprise y revient. */
const READ_LINE = 0.3;
/** Au-delà, la progression des pages est une barre plutôt que des points. */
const MAX_DOTS = 16;
/** Jetons (mots, espaces, ponctuation) au-delà desquels les pages se recomposent après coup. */
const LONG_LESSON = 5000;

/** Page qui tourne : la suivante arrive de droite (de gauche en arabe), la précédente de l'autre côté. */
const TURN = {
  enter: (d: number) => ({ opacity: 0, x: 24 * d, filter: "blur(4px)" }),
  center: { opacity: 1, x: 0, filter: "blur(0px)" },
  exit: (d: number) => ({ opacity: 0, x: -24 * d, filter: "blur(4px)" }),
};

const samePages = (a: PageRange[], b: PageRange[]) => a.length === b.length && a.every((p, i) => p.start === b[i].start && p.end === b[i].end);

/**
 * Repères verticaux de la police de lecture, mesurés une fois par taille.
 * La lanterne se centre sur la hauteur des capitales, avec la même marge
 * au-dessus des capitales et sous la ligne de base (les jambages bas, légers
 * à l'œil, s'y logent), et non sur la boîte de la police, qui réserve bien
 * plus de place au-dessus des lettres qu'en dessous.
 */
const fontMarks = new Map<string, { ascent: number; capMid: number; half: number }>();
function readingMarks(el: HTMLElement) {
  const cs = getComputedStyle(el);
  const key = `${cs.fontSize}|${cs.fontFamily}|${cs.fontWeight}`;
  let m = fontMarks.get(key);
  if (!m) {
    const ctx = document.createElement("canvas").getContext("2d");
    if (!ctx) return null;
    ctx.font = `${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`;
    const size = parseFloat(cs.fontSize) || 23;
    const cap = ctx.measureText("H").actualBoundingBoxAscent;
    const desc = ctx.measureText("gpqyj").actualBoundingBoxDescent;
    m = { ascent: ctx.measureText("Hg").fontBoundingBoxAscent, capMid: cap / 2, half: cap / 2 + desc + size * 0.06 };
    // tant que la police n'est pas chargée, la mesure serait celle d'une police de secours
    if (document.fonts.status === "loaded") fontMarks.set(key, m);
  }
  return { ...m, padTop: parseFloat(cs.paddingTop) || 0 };
}

export function Reader() {
  const lessonId = useApp((s) => s.lessonId);
  const settings = useApp((s) => s.settings);
  const toast = useApp((s) => s.toast);
  const go = useApp((s) => s.go);
  const setSetting = useApp((s) => s.setSetting);
  const bump = useApp((s) => s.bumpLibrary);
  const refreshKnown = useApp((s) => s.refreshKnown);
  const openLesson = useApp((s) => s.openLesson);
  const openPlaylist = useApp((s) => s.openPlaylist);
  const autoplay = useApp((s) => s.autoplay);
  const streak = useApp((s) => s.streak);

  const [data, setData] = useState<OpenedLesson | null>(null);
  const [terms, setTerms] = useState<Record<string, Term>>({});
  const [page, setPage] = useState(0);
  const [range, setRange] = useState<Range | null>(null);
  const [cursor, setCursor] = useState(-1);
  const [illum, setIllum] = useState(false);
  const [complete, setComplete] = useState<null | { read: number; known: number; lingqs: number; secs: number; next: number | null }>(null);
  const [scrolled, setScrolled] = useState(false);
  const [simplify, setSimplify] = useState(false);
  const [glowKeys, setGlowKeys] = useState<Set<string>>(new Set());
  const [upNext, setUpNext] = useState<LessonSummary | null>(null);
  // panneau de droite : le mot touché, ou le chat sur la leçon
  const [aside, setAside] = useState<AsideTab>("word");
  // menu « Aa » : police, taille, couleur de la page, mise en page
  const [lookOpen, setLookOpen] = useState(false);
  const closeLook = useCallback(() => setLookOpen(false), []);
  // sens du dernier changement de page (1 : en avant), pour l'animation
  const [turn, setTurn] = useState(1);
  // « Ajouter à une playlist… » du menu Leçon : la leçon et celles de sa langue
  const [adding, setAdding] = useState<{ lesson: LessonSummary; all: LessonSummary[] } | null>(null);

  const look = readerLook(settings);
  const paged = look.layout === "pages";

  const scrollRef = useRef<HTMLDivElement>(null);
  const pageRef = useRef<HTMLDivElement>(null);
  // mode pages : zone du texte, bloc invisible où l'on compose les pages, copie du titre
  const viewRef = useRef<HTMLDivElement>(null);
  const measureRef = useRef<HTMLDivElement>(null);
  const ghostRef = useRef<HTMLDivElement>(null);
  const playerRef = useRef<PlayerHandle>(null);
  const drag = useRef<{ start: number; moved: boolean } | null>(null);
  const session = useRef({ read: 0, known: 0, lingqs: 0 });
  // mots lus pendant cette ouverture : chacun ne compte qu'une fois, même si les pages se recomposent
  const readToks = useRef(new Uint8Array(0));
  // jeton à garder sous les yeux quand les pages se recomposent (fenêtre, police, mise en page)
  const keepTok = useRef(0);
  const pageShownAt = useRef(performance.now());
  const pageNow = useRef(0);
  const lantern = useAnimationControls();
  const [lanternOn, setLanternOn] = useState(false);
  const [videoHost, setVideoHost] = useState<HTMLDivElement | null>(null);
  const [mediaPlaying, setMediaPlaying] = useState(false);
  const [cinema, setCinema] = useState(false);
  const onPlayback = useCallback((s: PlaybackState) => setMediaPlaying(s.playing), []);
  const lanternPage = useRef(-1);
  const lanternBox = useRef<{ y: number; height: number } | null>(null);
  // mot où reprendre la lecture : souhaité (want) et déjà écrit (saved)
  const anchorRef = useRef({ id: 0, want: 0, saved: 0 });
  const restoredFor = useRef<number | null>(null);
  const scrollTimer = useRef(0);
  const ignoreScrollUntil = useRef(0);
  const cursorNow = useRef(-1);
  cursorNow.current = cursor;
  const [resumeAt, setResumeAt] = useState(-1);

  // ---------- chargement ----------
  useEffect(() => {
    if (!lessonId) return;
    let alive = true;
    setData(null);
    setRange(null);
    setCursor(-1);
    setComplete(null);
    setUpNext(null);
    setCinema(false);
    session.current = { read: 0, known: 0, lingqs: 0 };
    api()
      .lessonOpen(lessonId)
      .then((d) => {
        if (!alive) return;
        readToks.current = new Uint8Array(d.tokens.length);
        const p = paginate(d.tokens);
        const pg = Math.min(d.lesson.page, p.length - 1);
        // reprise : le mot atteint s'il est sur la page enregistrée, sinon le début de cette page
        const a = d.lesson.anchor;
        keepTok.current = a >= p[pg].start && a < p[pg].end ? a : p[pg].start;
        setData(d);
        setTerms(d.terms);
        setPage(pg);
      })
      .catch(() => {
        if (!alive) return;
        // leçon supprimée entre-temps : on l'oublie
        useApp.getState().forgetLesson(lessonId);
        toast(t("Cette leçon n'existe plus.", "This lesson no longer exists."), "error");
        go("library");
      });
    void api().aiWarmup().catch(() => {});
    return () => {
      alive = false;
    };
  }, [lessonId, toast, go]);

  const lesson = data?.lesson;
  const tokens = useMemo(() => data?.tokens ?? [], [data]);
  // pages d'environ 230 mots : mode défilement, et page enregistrée (avancement de la bibliothèque)
  const basePages = useMemo(() => paginate(tokens), [tokens]);
  // mode pages : pages composées à la taille de l'écran
  const [fitted, setFitted] = useState<{ tokens: Token[]; pages: PageRange[] } | null>(null);
  const fittedOk = paged && fitted?.tokens === tokens;
  const pages = fittedOk ? fitted.pages : basePages;
  const pr = pages[page] ?? { start: 0, end: 0, words: 0 };
  const lang = lesson?.lang ?? "en";
  const pl = usePlaylist(lesson);
  pageNow.current = page;

  // temps actif passé dans la leçon (progrès, objectif du jour)
  const clock = useStudyClock(lesson?.lang, mediaPlaying, lesson?.id);
  useEffect(() => {
    pageShownAt.current = performance.now();
  }, [page, lessonId]);

  /**
   * Mots d'une page qu'on vient de lire, s'ils ne sont pas déjà comptés dans
   * cette ouverture. Feuilleter n'est pas lire : une page quittée en quelques
   * secondes ne compte pas, sauf « Terminer la page » (`sure`).
   */
  const creditPage = useCallback(
    (p: number, sure: boolean) => {
      const r = pages[p];
      if (!r || !r.words) return 0;
      const seen = readToks.current;
      let fresh = 0;
      for (let i = r.start; i < r.end; i++) if (tokens[i].w && !seen[i]) fresh++;
      if (!fresh) return 0;
      if (!sure && performance.now() - pageShownAt.current < Math.min(15, r.words / 4) * 1000) return 0;
      for (let i = r.start; i < r.end; i++) if (tokens[i].w) seen[i] = 1;
      return fresh;
    },
    [pages, tokens],
  );
  const logRead = useCallback(
    (words: number) => {
      if (!lesson || !words) return;
      session.current.read += words;
      void api()
        .activityAdd(lesson.lang, words, 0)
        .catch(() => {});
    },
    [lesson],
  );

  // la demande de lecture automatique ne vaut que pour cette ouverture
  useEffect(() => {
    if (data && useApp.getState().autoplay) useApp.setState({ autoplay: false });
  }, [data]);

  // fin de l'écoute dans une playlist : la leçon suivante s'annonce
  const onFinished = useCallback(() => {
    // écoutée jusqu'au bout : la dernière page est lue
    if (pageNow.current === pages.length - 1) logRead(creditPage(pageNow.current, false));
    if (!pl || !lesson) return;
    const nextId = pl.lessons[pl.lessons.indexOf(lesson.id) + 1];
    if (!nextId) {
      toast(t(`Fin de la playlist « ${pl.name} »`, `End of the playlist “${pl.name}”`), "light");
      void api().playlistUpdate(pl.id, { current: 0 });
      return;
    }
    setCinema(false);
    api()
      .lessonsList(lesson.lang)
      .then((all) => setUpNext(all.find((l) => l.id === nextId) ?? null))
      .catch(() => {});
  }, [pl, lesson, toast, pages.length, logRead, creditPage]);
  // l'écoute reprend sur cette leçon : la suivante attendra
  useEffect(() => {
    if (mediaPlaying) setUpNext(null);
  }, [mediaPlaying]);

  const statusOf = useCallback((k: string): number => terms[k]?.status ?? 0, [terms]);

  // ---------- reprise : mot atteint, écrit au plus une fois par seconde ----------
  useEffect(() => {
    if (!data) return;
    const id = data.lesson.id;
    anchorRef.current = { id, want: data.lesson.anchor, saved: data.lesson.anchor };
    const flush = () => {
      const a = anchorRef.current;
      if (a.id !== id || a.want === a.saved || a.want < 0) return;
      a.saved = a.want;
      void api().lessonUpdate(id, { anchor: a.want });
    };
    const timer = window.setInterval(flush, 1000);
    window.addEventListener("pagehide", flush);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("pagehide", flush);
      flush();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data?.lesson.id]);

  // le mot lu à voix haute (voix, audio ou vidéo) devient le point de reprise
  useEffect(() => {
    if (cursor < 0) return;
    anchorRef.current.want = cursor;
    keepTok.current = cursor;
  }, [cursor]);

  // ---------- mode pages : des pages qui tiennent dans l'écran ----------
  const [fitTick, setFitTick] = useState(0);
  const lastFit = useRef({ w: 0, h: 0 });
  useLayoutEffect(() => {
    if (!paged || !data) return;
    const view = viewRef.current;
    const box = measureRef.current;
    if (!view || !box) return;
    const cs = getComputedStyle(view);
    const height = view.clientHeight - parseFloat(cs.paddingTop) - parseFloat(cs.paddingBottom);
    const line = look.size * look.lineHeight;
    // zone cachée (plein écran vidéo) ou minuscule : les pages d'avant restent
    if (height < line * 2 || box.clientWidth < 60) return;
    lastFit.current = { w: view.clientWidth, h: view.clientHeight };
    const fit = () => {
      // la première page porte le titre de la leçon
      const head = ghostRef.current?.offsetHeight ?? 0;
      const next = fitPages(box, tokens, { height, first: Math.max(line * 2, height - head), line });
      setFitted((prev) => (prev && prev.tokens === tokens && samePages(prev.pages, next) ? prev : { tokens, pages: next }));
    };
    // très longue leçon (chapitre de livre) déjà composée : le texte change de taille tout de suite,
    // les pages suivent quand le curseur s'arrête (les recomposer prend quelques dixièmes de seconde)
    if (tokens.length > LONG_LESSON && fittedOk) {
      const timer = window.setTimeout(fit, 140);
      return () => window.clearTimeout(timer);
    }
    fit();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paged, data?.lesson.id, tokens, look.size, look.lineHeight, look.width, look.font.id, fitTick]);

  // fenêtre, vidéo ou barre latérale qui changent la place du texte ; police arrivée après coup
  useEffect(() => {
    const view = viewRef.current;
    if (!paged || !view) return;
    let timer = 0;
    const later = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => setFitTick((n) => n + 1), 110);
    };
    const ro = new ResizeObserver(() => {
      const { w, h } = lastFit.current;
      if (view.clientWidth !== w || view.clientHeight !== h) later();
    });
    ro.observe(view);
    document.fonts?.addEventListener("loadingdone", later);
    return () => {
      ro.disconnect();
      document.fonts?.removeEventListener("loadingdone", later);
      window.clearTimeout(timer);
    };
  }, [paged, data?.lesson.id]);

  // pages recomposées (ou autre mise en page) : on reste sur la page qui porte le même mot
  useLayoutEffect(() => {
    if (!data) return;
    const p = pageOfToken(pages, keepTok.current);
    setPage((cur) => (cur === p ? cur : p));
  }, [pages, data]);

  // à l'ouverture : retour au mot où l'on s'était arrêté, signalé par un bref halo
  useLayoutEffect(() => {
    if (!data || restoredFor.current === data.lesson.id) return;
    // en mode pages, on attend les vraies pages et la page qui porte le mot
    if (paged && !fittedOk) return;
    if (pageOfToken(pages, keepTok.current) !== page) return;
    restoredFor.current = data.lesson.id;
    const a = data.lesson.anchor;
    const range = pages[page];
    if (!range || a <= range.start || a >= range.end) return;
    if (!paged) {
      const sc = scrollRef.current;
      const el = pageRef.current?.querySelector(`[data-i="${a}"]`);
      if (!sc || !el) return;
      const top = el.getBoundingClientRect().top - sc.getBoundingClientRect().top + sc.scrollTop;
      // ce défilement automatique ne doit pas déplacer le point de reprise
      ignoreScrollUntil.current = performance.now() + 600;
      sc.scrollTop = Math.max(0, top - sc.clientHeight * READ_LINE);
    }
    // avec un média, la lanterne montre déjà l'endroit
    if (!(data.lesson.media_path && data.lesson.position > 0.5)) setResumeAt(a);
  }, [data, pages, page, paged, fittedOk]);
  useEffect(() => {
    if (resumeAt < 0) return;
    const t = window.setTimeout(() => setResumeAt(-1), 2600);
    return () => window.clearTimeout(t);
  }, [resumeAt]);

  /** Après un défilement : le mot prononcé s'il est visible, sinon le mot sur la ligne de lecture. */
  const onReaderScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const sc = e.currentTarget;
    setScrolled(sc.scrollTop > 90);
    if (performance.now() < ignoreScrollUntil.current) return;
    window.clearTimeout(scrollTimer.current);
    scrollTimer.current = window.setTimeout(() => {
      const range = pages[page];
      const pg = pageRef.current;
      if (!range || !pg) return;
      const box = sc.getBoundingClientRect();
      const cur = cursorNow.current;
      const curEl = cur >= range.start && cur < range.end ? pg.querySelector(`[data-i="${cur}"]`) : null;
      if (curEl) {
        const r = curEl.getBoundingClientRect();
        if (r.bottom > box.top && r.top < box.bottom) return;
      }
      if (sc.scrollTop < 40) {
        anchorRef.current.want = range.start;
        return;
      }
      const line = box.top + sc.clientHeight * READ_LINE;
      for (const el of pg.querySelectorAll<HTMLElement>("[data-i]")) {
        if (el.getBoundingClientRect().bottom > line) {
          anchorRef.current.want = Number(el.dataset.i);
          return;
        }
      }
    }, 250);
  };

  // ---------- expressions enregistrées ----------
  const phraseIndex = useMemo(() => {
    const byFirst = new Map<string, { words: string[]; status: number; key: string }[]>();
    for (const t of Object.values(terms)) {
      if (!t.term.includes(" ") || t.status === 5) continue;
      const words = t.term.split(" ");
      const list = byFirst.get(words[0]) ?? [];
      list.push({ words, status: t.status, key: t.term });
      byFirst.set(words[0], list);
    }
    return byFirst;
  }, [terms]);

  const phraseCover = useMemo(() => {
    // jeton -> { statut, bornes } pour la page courante
    const cover = new Map<number, { status: number; a: number; b: number; key: string }>();
    if (!phraseIndex.size) return cover;
    const wordIdx: number[] = [];
    for (let i = pr.start; i < pr.end; i++) if (tokens[i].w) wordIdx.push(i);
    for (let w = 0; w < wordIdx.length; w++) {
      const cands = phraseIndex.get(tokens[wordIdx[w]].k);
      if (!cands) continue;
      for (const c of cands) {
        if (w + c.words.length > wordIdx.length) continue;
        let ok = true;
        for (let j = 1; j < c.words.length; j++) {
          if (tokens[wordIdx[w + j]].k !== c.words[j]) {
            ok = false;
            break;
          }
        }
        if (ok) {
          const a = wordIdx[w];
          const b = wordIdx[w + c.words.length - 1];
          for (let i = a; i <= b; i++) cover.set(i, { status: c.status, a, b, key: c.key });
        }
      }
    }
    return cover;
  }, [phraseIndex, tokens, pr.start, pr.end]);

  // ---------- sélection ----------
  const selection: Selection | null = useMemo(() => {
    if (!range || !lesson) return null;
    const { a, b } = range;
    const [sa, sb] = sentenceBounds(tokens, a);
    const sbEnd = Math.max(sb, sentenceBounds(tokens, b)[1]);
    const sentStart = tokens[sa]?.s ?? 0;
    const sentEnd = tokens[sbEnd - 1]?.e ?? 0;
    const selStart = tokens[a].s;
    const selEnd = tokens[b].e;
    const clean = (s: string) => s.replace(/\s+/g, " ");
    const words = [];
    for (let i = a; i <= b; i++) if (tokens[i].w) words.push(tokens[i].k);
    const isPhrase = words.length > 1;
    const ph = !isPhrase ? phraseCover.get(a) : null;
    const phSurface = ph ? clean(lesson.text.slice(tokens[ph.a].s, tokens[ph.b].e)) : "";
    return {
      surface: clean(lesson.text.slice(selStart, selEnd)),
      key: words.join(" "),
      sentence: clean(lesson.text.slice(sentStart, sentEnd)).trim(),
      before: clean(lesson.text.slice(sentStart, selStart)).trimStart(),
      after: clean(lesson.text.slice(selEnd, sentEnd)).trimEnd(),
      isPhrase,
      words: words.length,
      tokenIndex: a,
      phrase: ph ? { key: ph.key, a: ph.a, b: ph.b, surface: phSurface, translation: terms[ph.key]?.translation ?? "" } : null,
    };
  }, [range, tokens, lesson, phraseCover, terms]);

  const setStatus = useCallback(
    (key: string, status: number, extra?: Partial<Term>) => {
      if (!lesson || !key) return;
      setTerms((prev) => {
        const next = { ...prev };
        if (status === 0) delete next[key];
        else
          next[key] = {
            term: key,
            status: status as Term["status"],
            translation: extra?.translation ?? prev[key]?.translation ?? "",
            note: extra?.note ?? prev[key]?.note ?? "",
            lemma: prev[key]?.lemma ?? "",
            context: prev[key]?.context ?? extra?.context ?? "",
            updated_at: Date.now() / 1000,
          };
        return next;
      });
      if (status === 4) {
        setGlowKeys(new Set([key]));
        window.setTimeout(() => setGlowKeys(new Set()), 1000);
      }
      void api()
        .termSet({ lang: lesson.lang, term: key, status, translation: extra?.translation, note: extra?.note, context: extra?.context })
        .then(() => (status === 4 || status === 0 ? refreshKnown() : undefined))
        .catch((e) => toast(errorText(e), "error"));
    },
    [lesson, refreshKnown, toast],
  );

  const rangeRef = useRef<Range | null>(null);
  rangeRef.current = range;
  const hearTimer = useRef(0);
  useEffect(() => () => window.clearTimeout(hearTimer.current), []);

  /**
   * Le mot touché ou le passage surligné se fait entendre, seulement quand la
   * leçon est en pause (jamais par-dessus l'audio). `delay` : au clavier, on
   * attend que la flèche s'arrête sur un mot plutôt que d'en dire chaque étape.
   */
  const hear = useCallback(
    (a: number, b: number, delay = 0) => {
      window.clearTimeout(hearTimer.current);
      if (!lesson || settings.auto_pronounce === "0") return;
      const say = () => {
        const r = rangeRef.current;
        // sélection fermée ou changée entre-temps, ou leçon relancée : rien à dire
        if (delay && (r?.a !== a || r?.b !== b)) return;
        if (playerRef.current?.isPlaying()) return;
        const text = lesson.text.slice(tokens[a].s, tokens[b].e).replace(/\s+/g, " ").trim();
        if (text) void pronounce(text, lesson.lang, settings[`voice_${lesson.lang}`], true);
      };
      if (delay) hearTimer.current = window.setTimeout(say, delay);
      else say();
    },
    [lesson, tokens, settings],
  );

  const select = useCallback(
    (a: number, b: number, sound: number | false = 0) => {
      if (a > b) [a, b] = [b, a];
      // retire la ponctuation aux extrémités
      while (a < b && !tokens[a].w) a++;
      while (b > a && !tokens[b].w) b--;
      if (!tokens[a]?.w) return;
      setRange({ a, b });
      setAside("word");
      if (sound !== false) hear(a, b, sound);
      if (a === b) {
        const k = tokens[a].k;
        if (statusOf(k) === 0) {
          session.current.lingqs++;
          const [sa, sb] = sentenceBounds(tokens, a);
          const ctx = lesson ? lesson.text.slice(tokens[sa].s, tokens[sb - 1].e).replace(/\s+/g, " ").trim() : "";
          setStatus(k, 1, { context: ctx });
        }
      }
    },
    [tokens, statusOf, setStatus, lesson, hear],
  );

  const onTranslation = useCallback(
    (key: string, translation: string, note: string, sentence: string) => {
      const cur = statusOf(key);
      setStatus(key, cur === 0 ? 1 : cur, { translation, note, context: sentence });
    },
    [statusOf, setStatus],
  );

  // ---------- souris : clic et glisser pour les expressions ----------
  const idxFromEvent = (e: React.PointerEvent) => {
    const el = (e.target as HTMLElement).closest("[data-i]") as HTMLElement | null;
    return el ? Number(el.dataset.i) : -1;
  };
  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    const i = idxFromEvent(e);
    if (i < 0) return;
    if (e.altKey) {
      playerRef.current?.playFrom(i);
      return;
    }
    if (e.shiftKey && range) {
      select(Math.min(range.a, i), Math.max(range.b, i));
      return;
    }
    drag.current = { start: i, moved: false };
    setRange({ a: i, b: i });
  };
  const onPointerOver = (e: React.PointerEvent) => {
    if (!drag.current) return;
    const i = idxFromEvent(e);
    if (i < 0) return;
    if (i !== drag.current.start) drag.current.moved = true;
    const a = Math.min(drag.current.start, i);
    const b = Math.max(drag.current.start, i);
    setRange({ a, b });
  };
  // toucher le vide (marges, entre deux lignes, titre) éteint la sélection, comme Échap
  const onVoidDown = (e: React.PointerEvent<HTMLElement>) => {
    if (e.button !== 0 || e.shiftKey || !rangeRef.current || lookOpen) return;
    // un mot, un bouton ou un champ gardent leur rôle
    if ((e.target as HTMLElement).closest("[data-i], button, a, input, textarea, select, [role='button']")) return;
    // la barre de défilement n'est pas du vide
    if (e.target === e.currentTarget && e.nativeEvent.offsetX >= e.currentTarget.clientWidth) return;
    setRange(null);
  };
  useEffect(() => {
    const up = () => {
      if (!drag.current) return;
      const d = drag.current;
      drag.current = null;
      const r = rangeRef.current;
      if (r) select(r.a, d.moved ? r.b : r.a);
    };
    window.addEventListener("pointerup", up);
    return () => window.removeEventListener("pointerup", up);
  }, [select]);

  // ---------- pages ----------
  // la page enregistrée reste celle des pages de 230 mots, quelle que soit la mise en page
  // (la bibliothèque en tire l'avancement) ; la reprise exacte passe par le mot atteint
  const savePage = useCallback(
    (p: number) => {
      const r = pages[p];
      if (lesson && r) void api().lessonUpdate(lesson.id, { page: pageOfToken(basePages, r.start) });
    },
    [lesson, pages, basePages],
  );

  const goPage = useCallback(
    (p: number) => {
      if (!lesson || p < 0 || p >= pages.length) return;
      // en avançant, la page qu'on quitte est lue
      if (p > pageNow.current) logRead(creditPage(pageNow.current, false));
      setTurn(p >= pageNow.current ? 1 : -1);
      setPage(p);
      setRange(null);
      scrollRef.current?.scrollTo({ top: 0, behavior: "smooth" });
      anchorRef.current.want = pages[p].start;
      keepTok.current = pages[p].start;
      savePage(p);
    },
    [lesson, pages, logRead, creditPage, savePage],
  );

  const onPlayerPage = useCallback(
    (p: number) => {
      // la lecture passe à la page suivante : celle qu'elle quitte est lue
      if (p === pageNow.current + 1) logRead(creditPage(pageNow.current, false));
      setTurn(p >= pageNow.current ? 1 : -1);
      setPage(p);
      setRange(null);
      if (pages[p]) keepTok.current = pages[p].start;
      savePage(p);
    },
    [pages, logRead, creditPage, savePage],
  );

  // mode pages : deux doigts qui glissent sur le trackpad (ou la molette) tournent la page, comme dans Livres
  const swipe = useRef({ x: 0, y: 0, lock: false, timer: 0 });
  const onSwipe = (e: React.WheelEvent) => {
    const s = swipe.current;
    // un seul tour de page par geste : on attend que l'élan du trackpad retombe
    window.clearTimeout(s.timer);
    s.timer = window.setTimeout(() => {
      s.x = 0;
      s.y = 0;
      s.lock = false;
    }, 240);
    if (s.lock) return;
    s.x += e.deltaX;
    s.y += e.deltaY;
    let forward: boolean;
    // de côté : vers la gauche pour avancer (vers la droite en arabe) ; de haut en bas : vers le bas
    if (Math.abs(s.x) >= 50 && Math.abs(s.x) > Math.abs(s.y)) forward = s.x > 0 !== !!langInfo(lang).rtl;
    else if (Math.abs(s.y) >= 70) forward = s.y > 0;
    else return;
    s.lock = true;
    goPage(pageNow.current + (forward ? 1 : -1));
  };

  const findNext = async () => {
    if (!lesson) return null;
    // dans une playlist, la suivante est la sienne ; sinon, celle de la collection
    if (pl) return pl.lessons[pl.lessons.indexOf(lesson.id) + 1] ?? null;
    const all = await api().lessonsList(lesson.lang);
    const same = all.filter((l) => l.collection && l.collection === lesson.collection).sort((a, b) => a.created_at - b.created_at || a.id - b.id);
    const idx = same.findIndex((l) => l.id === lesson.id);
    return idx >= 0 && idx < same.length - 1 ? same[idx + 1].id : null;
  };

  const finishPage = async () => {
    if (!lesson || illum) return;
    const marks = settings.finish_marks_known !== "0";
    const keys: string[] = [];
    for (let i = pr.start; i < pr.end; i++) if (tokens[i].w && statusOf(tokens[i].k) === 0) keys.push(tokens[i].k);
    let added = 0;
    // une page déjà comptée (lue par l'audio, relue) ne compte pas deux fois
    const words = creditPage(page, true);
    try {
      if (marks && keys.length) {
        setIllum(true);
        await sleep(1050);
        added = await api().termsMarkKnown(lesson.lang, keys, words);
        setTerms((prev) => {
          const next = { ...prev };
          for (const k of keys) if (!next[k]) next[k] = { term: k, status: 4, translation: "", note: "", lemma: "", context: "", updated_at: Date.now() / 1000 };
          return next;
        });
        setIllum(false);
      } else if (words) {
        await api().activityAdd(lesson.lang, words, 0);
      }
    } catch (e) {
      if (words) readToks.current.fill(0, pr.start, pr.end);
      setIllum(false);
      toast(errorText(e), "error");
      return;
    }
    session.current.read += words;
    session.current.known += added;
    void refreshKnown();
    if (page < pages.length - 1) {
      goPage(page + 1);
      if (added) toast(t(`${added} mot${added > 1 ? "s" : ""} rejoigne${added > 1 ? "nt" : ""} vos mots connus`, `${count(added, "", "", "word joins", "words join")} your known words`), "light");
    } else {
      playerRef.current?.stop();
      // leçon terminée : la prochaine lecture repart du début
      playerRef.current?.forget();
      anchorRef.current = { id: lesson.id, want: 0, saved: 0 };
      await api().lessonUpdate(lesson.id, { completed: true, page: 0, position: 0, anchor: 0 });
      bump();
      // le temps de la séance compte tout de suite (objectif du jour, série)
      await clock.flush();
      const next = await findNext();
      setComplete({ ...session.current, secs: clock.session(), next });
    }
  };

  // ---------- clavier ----------
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t.closest("input, textarea, select, [contenteditable]") || e.metaKey || e.ctrlKey) return;
      if (useApp.getState().importOpen || useMenu.getState().shortcuts || simplify || complete || lookOpen || adding) return;
      const words: number[] = [];
      for (let i = pr.start; i < pr.end; i++) if (tokens[i].w) words.push(i);
      const cur = range ? words.indexOf(range.a) : -1;
      const key = e.key.toLowerCase();
      // statut au clavier : un mot, ou une sélection assez courte pour devenir une expression
      const canStatus = !!selection && selection.words <= EXPR_MAX_WORDS;
      if ((e.key === "ArrowRight" || e.key === "ArrowLeft") && e.shiftKey && range) {
        // Maj + flèches : la sélection s'allonge ou se raccourcit d'un mot par la fin
        e.preventDefault();
        const end = words.indexOf(range.b);
        if (cur < 0 || end < 0) return;
        let next: Range | null = null;
        if (e.key === "ArrowRight") {
          if (end < words.length - 1) next = { a: range.a, b: words[end + 1] };
        } else if (end > cur) next = { a: range.a, b: words[end - 1] };
        else if (cur > 0) next = { a: words[cur - 1], b: range.b };
        if (next) {
          setRange(next);
          hear(next.a, next.b, 350);
        }
      } else if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
        e.preventDefault();
        const n = e.key === "ArrowRight" ? (cur < 0 ? 0 : Math.min(words.length - 1, cur + 1)) : cur < 0 ? words.length - 1 : Math.max(0, cur - 1);
        if (words[n] !== undefined) select(words[n], words[n], 250);
      } else if (["1", "2", "3"].includes(key) && canStatus) {
        setStatus(selection.key, Number(key));
      } else if ((key === "k" || key === "4") && canStatus) {
        setStatus(selection.key, 4);
      } else if (key === "x" && canStatus) {
        setStatus(selection.key, 5);
      } else if (key === "0" && canStatus) {
        setStatus(selection.key, 0);
      } else if (key === "c") {
        // le chat sur la leçon, prêt à écrire
        e.preventDefault();
        setAside("chat");
        useChat.getState().focus();
      } else if (e.key === " ") {
        e.preventDefault();
        playerRef.current?.toggle();
      } else if (e.key === "Enter") {
        e.preventDefault();
        void finishPage();
      } else if (e.key === "Escape") {
        if (cinema) setCinema(false);
        else setRange(null);
      } else if (e.key === "PageDown") {
        goPage(page + 1);
      } else if (e.key === "PageUp") {
        goPage(page - 1);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // ---------- menu Leçon (barre des menus du Mac) ----------
  const addToPlaylist = async () => {
    if (!lesson) return;
    try {
      const all = await api().lessonsList(lesson.lang);
      const me = all.find((l) => l.id === lesson.id);
      if (me) setAdding({ lesson: me, all });
    } catch (e) {
      toast(errorText(e), "error");
    }
  };
  const changeCover = async () => {
    if (!lesson) return;
    try {
      if (await chooseCover(lesson.id)) {
        bump();
        toast(t("Nouvelle couverture enregistrée", "New cover saved"), "light");
      }
    } catch (e) {
      toast(errorText(e), "error");
    }
  };
  const removeLesson = async () => {
    if (!lesson) return;
    const ok = await confirmAsk(
      t(`Supprimer « ${lesson.title} » ? Les mots appris sont conservés.`, `Delete “${lesson.title}”? The words you learned are kept.`),
      t("Supprimer la leçon", "Delete the lesson"),
      t("Supprimer", "Delete"),
    );
    if (!ok) return;
    playerRef.current?.stop();
    try {
      await api().lessonDelete(lesson.id);
    } catch (e) {
      toast(errorText(e), "error");
      return;
    }
    useApp.getState().forgetLesson(lesson.id);
    toast(t("Leçon supprimée", "Lesson deleted"));
    bump();
    go("library");
  };
  useLessonMenu(lesson ? { media: !!lesson.media_path, video: !!lesson.video_path } : null, {
    toggle: () => playerRef.current?.toggle(),
    skip: (secs) => playerRef.current?.skip(secs),
    prev: () => goPage(page - 1),
    next: () => goPage(page + 1),
    restart: () => goPage(0),
    finish: () => void finishPage(),
    chat: () => {
      setAside("chat");
      useChat.getState().focus();
    },
    simplify: () => setSimplify(true),
    cinema: () => setCinema(true),
    playlist: () => void addToPlaylist(),
    cover: () => void changeCover(),
    remove: () => void removeLesson(),
  });

  // le chat cadre l'extrait des longues leçons sur la page lue
  const pageStart = tokens[pr.start]?.s ?? 0;
  useEffect(() => {
    if (lesson) useChat.setState({ reading: { lesson: lesson.id, offset: pageStart } });
  }, [lesson, pageStart]);

  // ---------- lanterne (mot prononcé) ----------
  // la colonne change de largeur (barre latérale repliée, fenêtre redimensionnée) :
  // les lignes se recomposent, la lanterne se recale une fois le mouvement fini
  const [relayout, setRelayout] = useState(0);
  const loaded = !!data;
  useEffect(() => {
    const el = scrollRef.current ?? viewRef.current;
    if (!el) return;
    let w = el.clientWidth;
    let t = 0;
    const ro = new ResizeObserver(() => {
      if (el.clientWidth === w) return;
      w = el.clientWidth;
      window.clearTimeout(t);
      t = window.setTimeout(() => setRelayout((n) => n + 1), 120);
    });
    ro.observe(el);
    return () => {
      ro.disconnect();
      window.clearTimeout(t);
    };
  }, [loaded, paged]);

  useLayoutEffect(() => {
    const container = pageRef.current;
    if (!container || cursor < 0 || cursor < pr.start || cursor >= pr.end) {
      setLanternOn(false);
      return;
    }
    const el = container.querySelector(`[data-i="${cursor}"]`) as HTMLElement | null;
    if (!el) return;
    const c = container.getBoundingClientRect();
    const r = el.getBoundingClientRect();
    const fm = readingMarks(el);
    const padX = 3;
    let target = { x: r.left - c.left - padX, y: r.top - c.top - 1.5, width: r.width + padX * 2, height: r.height + 3 };
    if (fm) {
      // boîte symétrique autour des lettres : centrée sur la hauteur des capitales
      const mid = r.top - c.top + fm.padTop + fm.ascent - fm.capMid;
      target = { ...target, y: mid - fm.half, height: fm.half * 2 };
    }
    const samePage = lanternPage.current === page;
    lanternPage.current = page;
    // changement de ligne : un saut net plutôt qu'une glissade en travers du paragraphe
    const prev = lanternBox.current;
    const newLine = !prev || Math.abs(target.y - prev.y) > target.height * 0.5;
    lanternBox.current = { y: target.y, height: target.height };
    if (!lanternOn || !samePage || newLine) {
      // la glissade vers le mot précédent (fin de la ligne d'avant) tourne peut-être
      // encore : sans l'arrêter, elle écraserait le saut et ramènerait la lanterne à droite
      lantern.stop();
      lantern.set(target);
      setLanternOn(true);
    } else void lantern.start({ ...target, transition: { type: "spring", stiffness: 760, damping: 50, mass: 0.5 } });
    // garde le mot visible
    const sc = scrollRef.current;
    if (sc) {
      const sr = sc.getBoundingClientRect();
      if (r.top < sr.top + 80 || r.bottom > sr.bottom - 120) sc.scrollBy({ top: r.top - sr.top - sr.height * 0.35, behavior: "smooth" });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cursor, page, pr.start, pr.end, relayout]);

  // la visite guidée arrête la lecture à voix haute quand elle passe à la suite
  useEffect(() => {
    const pause = () => playerRef.current?.pause();
    window.addEventListener("lumen:pause", pause);
    return () => window.removeEventListener("lumen:pause", pause);
  }, []);

  if (!lessonId) {
    return (
      <div className="empty" style={{ flex: 1, justifyContent: "center" }}>
        <Orb size={44} />
        <h3>{t("Aucune lecture en cours", "Nothing being read")}</h3>
        <p>{t("Choisissez une leçon dans la bibliothèque.", "Choose a lesson in the library.")}</p>
        <button className="btn primary" onClick={() => go("library")}>
          {t("Bibliothèque", "Library")}
        </button>
      </div>
    );
  }

  if (!data || !lesson) {
    return (
      <div className={`reader ${look.className}`} style={look.style}>
        <div className="reader-col">
          <div className="reader-top drag" data-tauri-drag-region />
          <div className="reader-inner" style={{ width: "100%" }}>
            <div className="skeleton" style={{ height: 44, width: "55%", marginBottom: 30 }} />
            {Array.from({ length: 8 }).map((_, i) => (
              <div key={i} className="skeleton" style={{ height: 18, width: `${92 - (i % 3) * 9}%`, marginBottom: 16 }} />
            ))}
          </div>
        </div>
      </div>
    );
  }

  // ---------- rendu de la page ----------
  const markClass = settings.word_style === "line" ? "mark-line" : "mark-tint";
  const paragraphs: React.ReactNode[] = [];
  let current: React.ReactNode[] = [];
  const total = Math.max(1, pr.end - pr.start);
  for (let i = pr.start; i < pr.end; i++) {
    const t = tokens[i];
    const ph = phraseCover.get(i);
    const inPhrase = ph && i > ph.a && i <= ph.b;
    const inSel = range && range.a !== range.b && i > range.a && i < range.b;
    if (!t.w) {
      const parts = t.t.split(/\n+/);
      if (parts.length > 1) {
        if (parts[0]) current.push(parts[0]);
        if (current.length) paragraphs.push(<p key={`p${i}`}>{current}</p>);
        current = [];
        const tail = parts[parts.length - 1];
        if (tail.trim()) current.push(tail);
      } else {
        current.push(
          inPhrase || inSel ? (
            <span key={i} className={`${inPhrase ? `ph p${ph!.status}` : ""} ${inSel ? "sel-gap" : ""}`}>
              {t.t}
            </span>
          ) : (
            t.t
          ),
        );
      }
      continue;
    }
    const st = statusOf(t.k);
    const selected = range && i >= range.a && i <= range.b;
    let cls = `w s${st}`;
    if (selected) cls += range!.a === range!.b ? " sel" : ` sel-range${i === range!.a ? " sel-a" : ""}${i === range!.b ? " sel-b" : ""}`;
    if (ph) cls += ` ph p${ph.status}`;
    if (i === cursor) cls += " cur";
    if (i === resumeAt) cls += " resume";
    if (glowKeys.has(t.k)) cls += " known-glow";
    current.push(
      <span key={i} className={cls} data-i={i} style={illum && st === 0 ? ({ "--d": `${((i - pr.start) / total) * 0.75}s` } as React.CSSProperties) : undefined}>
        {t.t}
      </span>,
    );
  }
  if (current.length) paragraphs.push(<p key="last">{current}</p>);

  let newOnPage = 0;
  const seen = new Set<string>();
  for (let i = pr.start; i < pr.end; i++) {
    const t = tokens[i];
    if (t.w && !seen.has(t.k)) {
      seen.add(t.k);
      if (statusOf(t.k) === 0) newOnPage++;
    }
  }

  const li = langInfo(lang);
  const isLast = page === pages.length - 1;
  const isVideo = lesson.kind === "video" || !!lesson.video_path;
  const sideHidden = settings.reader_sidebar === "0";
  // en arabe, la page suivante est à gauche
  const flip = li.rtl ? -1 : 1;
  const showMini = paged ? page > 0 : scrolled;

  const hint =
    settings.finish_marks_known !== "0" && newOnPage > 0
      ? t(
          `Les ${newOnPage} mots bleus que vous n'avez pas consultés rejoindront vos mots connus.`,
          `The ${newOnPage} blue word${newOnPage > 1 ? "s" : ""} you didn't look up will join your known words.`,
        )
      : isLast
        ? t("Dernière page de la leçon.", "Last page of the lesson.")
        : t("Tous les mots de cette page sont déjà rencontrés.", "You have already met every word on this page.");
  const finishLabel = isLast ? t("Terminer la leçon", "Finish the lesson") : t("Terminer la page", "Finish the page");

  const header = (
    <header className="reader-head">
      <div className="reader-crumb">
        {pl ? (
          <>
            <button onClick={() => openPlaylist(null)}>Playlists</button>
            <span>›</span>
            <button onClick={() => openPlaylist(pl.id)}>{pl.name}</button>
          </>
        ) : (
          <button onClick={() => go("library")}>{t("Bibliothèque", "Library")}</button>
        )}
        {!pl && lesson.collection && (
          <>
            <span>›</span>
            <span>{lesson.collection}</span>
          </>
        )}
      </div>
      <h1 dir="auto">{lesson.title}</h1>
      <div className="reader-chips">
        <span className="chip">{li.name}</span>
        {paged ? (
          // pas de nombre qui dépend des pages : ce titre sert aussi à les composer
          <span className="chip num">{count(lesson.word_count, "mot", "mots", "word", "words")}</span>
        ) : (
          <>
            <span className="chip num">{t(`Page ${page + 1} sur ${pages.length}`, `Page ${page + 1} of ${pages.length}`)}</span>
            <span className="chip new num">{t(`${newOnPage} nouveau${newOnPage > 1 ? "x" : ""}`, `${newOnPage} new`)}</span>
            <span className="chip num">{count(pr.words, "mot", "mots", "word", "words")}</span>
          </>
        )}
      </div>
    </header>
  );

  const pageEl = (
    <div
      ref={pageRef}
      data-tour="page"
      className={`page ${markClass} ${illum ? "illuminate" : ""}`}
      onPointerDown={onPointerDown}
      onPointerOver={onPointerOver}
      lang={lang}
      dir={li.rtl ? "rtl" : undefined}
    >
      <motion.div className="lantern" animate={lantern} style={{ opacity: lanternOn ? 1 : 0, transition: "opacity .3s ease" }} />
      {illum && (
        <motion.div
          className="sweep"
          initial={{ y: -200, opacity: 0 }}
          animate={{ y: (pageRef.current?.offsetHeight ?? 600) + 40, opacity: [0, 1, 1, 0] }}
          transition={{ duration: 1.05, ease: [0.4, 0, 0.2, 1] }}
        />
      )}
      {paragraphs}
    </div>
  );
  const turnProps = {
    custom: turn * flip,
    variants: TURN,
    initial: "enter",
    animate: "center",
    exit: "exit",
    transition: { duration: 0.32, ease: [0.2, 0.8, 0.2, 1] },
  } as const;

  const dots = (
    <div className="page-dots" aria-hidden="true">
      {pages.map((_, i) => (
        <button key={i} className={i === page ? "on" : i < page ? "done" : ""} onClick={() => goPage(i)} tabIndex={-1} />
      ))}
    </div>
  );

  // flèches du mode pages : la précédente et la suivante, de part et d'autre du texte
  const arrow = (side: "left" | "right") => {
    const forward = (side === "right") === !li.rtl;
    const target = page + (forward ? 1 : -1);
    const off = target < 0 || target >= pages.length;
    return (
      <button
        className={`leaf-arrow ${side} ${off ? "off" : ""}`}
        onClick={() => goPage(target)}
        disabled={off}
        aria-label={forward ? t("Page suivante", "Next page") : t("Page précédente", "Previous page")}
        title={forward ? t("Page suivante (sans marquer les mots)", "Next page (without marking words)") : t("Page précédente", "Previous page")}
      >
        <span className="leaf-arrow-glow" />
        <Icon name={side} size={30} stroke={1.5} />
      </button>
    );
  };

  return (
    <div className={`reader ${cinema ? "cinema" : ""} ${look.className}`} style={look.style}>
      <div className="reader-col">
        <div className="reader-top drag" data-tauri-drag-region>
          {!cinema && (
            <button
              className="icon-btn no-drag"
              onClick={() => setSetting("reader_sidebar", sideHidden ? "1" : "0")}
              aria-label={sideHidden ? t("Afficher la barre latérale", "Show the sidebar") : t("Masquer la barre latérale", "Hide the sidebar")}
              title={sideHidden ? t("Afficher la barre latérale", "Show the sidebar") : t("Masquer la barre latérale pour lire plus au large", "Hide the sidebar to read with more room")}
            >
              <Icon name="sidebar" size={18} />
            </button>
          )}
          <button
            className="icon-btn no-drag"
            onClick={() => (pl ? openPlaylist(pl.id) : go("library"))}
            aria-label={pl ? t("Retour à la playlist", "Back to the playlist") : t("Retour à la bibliothèque", "Back to the library")}
            title={pl ? t(`Retour à « ${pl.name} »`, `Back to “${pl.name}”`) : undefined}
          >
            <Icon name="back" size={18} />
          </button>
          {pl ? (
            <div className="pl-strip-slot" data-tauri-drag-region>
              <PlaylistStrip pl={pl} lessonId={lesson.id} playing={() => !!playerRef.current?.isPlaying()} />
            </div>
          ) : (
            <span className={`title-mini ${showMini ? "show" : ""}`} data-tauri-drag-region>
              {lesson.title}
            </span>
          )}
          {!cinema && (
            <div className="look-anchor no-drag">
              <button
                className={`icon-btn look-btn ${lookOpen ? "on" : ""}`}
                data-tour="display"
                onClick={() => setLookOpen((v) => !v)}
                aria-expanded={lookOpen}
                aria-label={t("Affichage : police, taille, couleur, mise en page", "Display: font, size, color, layout")}
                title={t("Affichage : police, taille, couleur, mise en page", "Display: font, size, color, layout")}
              >
                <span className="aa">
                  A<small>a</small>
                </span>
              </button>
              <DisplayMenu open={lookOpen} onClose={closeLook} lang={lang} />
            </div>
          )}
          <button className="btn sm soft no-drag" onClick={() => setSimplify(true)} title={t("Réécrire ce texte à un niveau plus simple", "Rewrite this text at a simpler level")}>
            <Icon name="sparkle" size={14} /> {t("Simplifier", "Simplify")}
          </button>
        </div>

        {isVideo && (
          <VideoStage
            key={`stage-${lesson.id}`}
            lesson={lesson}
            tokens={tokens}
            terms={terms}
            cursor={cursor}
            playing={mediaPlaying}
            onHost={setVideoHost}
            onToggle={() => playerRef.current?.toggle()}
            onWord={(i) => {
              // la vidéo s'arrête sur le mot touché ; il ne se fait entendre que si elle était déjà en pause
              const wasPlaying = !!playerRef.current?.isPlaying();
              playerRef.current?.pause();
              select(i, i, wasPlaying ? false : 0);
            }}
            onVideoReady={(path) => setData((d) => (d ? { ...d, lesson: { ...d.lesson, video_path: path } } : d))}
            cinema={cinema}
            setCinema={setCinema}
          />
        )}

        {paged ? (
          <div className="reader-leaves" onWheel={onSwipe} onPointerDown={onVoidDown}>
            {arrow("left")}
            <div className="leaf">
              <div className="leaf-view" ref={viewRef}>
                {/* composition des pages : copie invisible du titre et de la page, mêmes styles */}
                <div className="leaf-ghost" ref={ghostRef} aria-hidden="true" inert>
                  {header}
                </div>
                <div className={`page leaf-measure ${markClass}`} ref={measureRef} lang={lang} dir={li.rtl ? "rtl" : undefined} aria-hidden="true" />
                <AnimatePresence mode="wait" initial={false} custom={turn * flip}>
                  <motion.div key={page} className="leaf-page" {...turnProps}>
                    {page === 0 && header}
                    {pageEl}
                  </motion.div>
                </AnimatePresence>
              </div>
              <footer className="leaf-foot">
                <div className="leaf-progress">
                  {pages.length > 1 &&
                    (pages.length <= MAX_DOTS ? (
                      dots
                    ) : (
                      <div className="leaf-rail" aria-hidden="true">
                        <i style={{ width: `${((page + 1) / pages.length) * 100}%` }} />
                      </div>
                    ))}
                  {pages.length > 1 && (
                    <span className="num">
                      {page + 1} / {pages.length}
                    </span>
                  )}
                </div>
                <p className="leaf-hint">{hint}</p>
                <button className="btn primary glow" onClick={finishPage} disabled={illum} data-tour="finish">
                  {finishLabel}
                  <Icon name="forward" size={15} stroke={2} />
                </button>
              </footer>
            </div>
            {arrow("right")}
          </div>
        ) : (
          <div className="reader-scroll" ref={scrollRef} onScroll={onReaderScroll} onPointerDown={onVoidDown}>
            <div className="reader-inner">
              {header}

              <AnimatePresence mode="wait" initial={false} custom={turn * flip}>
                <motion.div key={page} {...turnProps}>
                  {pageEl}
                </motion.div>
              </AnimatePresence>

              <div className="page-end">
                {pages.length > 1 && (
                  <div className="page-nav">
                    <button className="icon-btn" onClick={() => goPage(page - 1)} disabled={page === 0} aria-label={t("Page précédente", "Previous page")}>
                      <Icon name="left" />
                    </button>
                    <span className="num">
                      {page + 1} / {pages.length}
                    </span>
                    <button className="icon-btn" onClick={() => goPage(page + 1)} disabled={isLast} aria-label={t("Page suivante sans marquer", "Next page without marking")}>
                      <Icon name="right" />
                    </button>
                  </div>
                )}
                <p>{hint}</p>
                <button className="btn primary lg glow" onClick={finishPage} disabled={illum} data-tour="finish">
                  {finishLabel}
                  <Icon name="forward" size={16} stroke={2} />
                </button>
              </div>
              {pages.length > 1 && dots}
            </div>
          </div>
        )}

        <Player
          key={`player-${lesson.id}-${lesson.media_path ?? ""}`}
          ref={playerRef}
          lesson={lesson}
          tokens={tokens}
          pages={pages}
          page={page}
          onPage={onPlayerPage}
          onCursor={setCursor}
          videoHost={videoHost}
          onState={onPlayback}
          onResynced={(t) => setData((d) => (d ? { ...d, lesson: { ...d.lesson, timings: t, timing_v: 2 } } : d))}
          onVoiced={(v) => setData((d) => (d ? { ...d, lesson: { ...d.lesson, ...v, position: 0 } } : d))}
          autoplay={autoplay}
          onFinished={onFinished}
        />

        <AnimatePresence>
          {upNext && pl && (
            <UpNext
              key={upNext.id}
              next={upNext}
              onGo={() => openLesson(upNext.id, { playlist: pl.id, autoplay: true })}
              onCancel={() => setUpNext(null)}
            />
          )}
        </AnimatePresence>

        <AnimatePresence>
          {complete && (
            <motion.div className="complete" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}>
              <motion.div className="complete-card" initial={{ scale: 0.9, y: 20 }} animate={{ scale: 1, y: 0 }} transition={{ type: "spring", stiffness: 260, damping: 22 }}>
                <div className="burst">
                  {Array.from({ length: 14 }).map((_, i) => (
                    <i key={i} style={{ transform: `rotate(${i * (360 / 14)}deg)` }}>
                      <b style={{ animationDelay: `${0.12 + (i % 4) * 0.05}s` }} />
                    </i>
                  ))}
                  <Orb size={46} />
                </div>
                <h2>{t("Leçon terminée", "Lesson finished")}</h2>
                <p className="muted">{lesson.title}</p>
                <div className="complete-stats">
                  <div>
                    <strong>{formatNumber(complete.read)}</strong>
                    <span>{t("mots lus", "words read")}</span>
                  </div>
                  <div>
                    <strong>{formatNumber(complete.known)}</strong>
                    <span>{t("nouveaux connus", "newly known")}</span>
                  </div>
                  <div>
                    <strong>{formatNumber(complete.lingqs)}</strong>
                    <span>{t("mots étudiés", "words studied")}</span>
                  </div>
                  <div>
                    <strong>
                      <Duration secs={complete.secs} />
                    </strong>
                    <span>{t("temps actif", "active time")}</span>
                  </div>
                </div>
                {streak && streak.current > 0 && (
                  <p className={`complete-streak ${streak.today_done ? "lit" : ""}`}>
                    <Icon name="flame" size={15} />
                    {streak.today_done
                      ? t(
                          `Objectif du jour atteint · ${count(streak.current, "jour", "jours", "", "")} de suite`,
                          `Daily goal reached · ${formatNumber(streak.current)}-day streak`,
                        )
                      : t(
                          `Série de ${count(streak.current, "jour", "jours", "", "")} · encore ${studyTime(Math.max(60, streak.goal_min * 60 - streak.today_secs))} aujourd'hui pour la prolonger`,
                          `${formatNumber(streak.current)}-day streak · ${studyTime(Math.max(60, streak.goal_min * 60 - streak.today_secs))} more today to keep it going`,
                        )}
                  </p>
                )}
                <div style={{ display: "flex", gap: 10 }}>
                  <button className="btn outline lg" onClick={() => go("library")}>
                    {t("Bibliothèque", "Library")}
                  </button>
                  {complete.next ? (
                    <button className="btn primary lg glow" onClick={() => openLesson(complete.next!, pl ? { playlist: pl.id } : undefined)}>
                      {t("Leçon suivante", "Next lesson")} <Icon name="forward" size={16} />
                    </button>
                  ) : (
                    <button
                      className="btn primary lg glow"
                      onClick={() => {
                        // relire, c'est lire encore : les pages comptent de nouveau
                        readToks.current = new Uint8Array(tokens.length);
                        session.current = { read: 0, known: 0, lingqs: 0 };
                        setComplete(null);
                        goPage(0);
                      }}
                    >
                      {t("Relire", "Read again")}
                    </button>
                  )}
                </div>
              </motion.div>
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      <aside className="word-panel" data-tour="panel" aria-label={t("Panneau latéral", "Side panel")}>
        <div className="wp-top drag" data-tauri-drag-region>
          <AsideTabs value={aside} onChange={setAside} />
        </div>
        <AnimatePresence mode="wait" initial={false}>
          <motion.div
            key={aside}
            className="aside-body"
            initial={{ opacity: 0, x: aside === "chat" ? 12 : -12 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: aside === "chat" ? -12 : 12 }}
            transition={{ duration: 0.2, ease: [0.2, 0.8, 0.2, 1] }}
          >
            {aside === "word" ? (
              <WordPanel
                lang={lang}
                sel={selection}
                term={selection ? terms[selection.key] : undefined}
                onStatus={(k, s) => setStatus(k, s)}
                onTranslation={onTranslation}
                onSelectPhrase={(a, b) => select(a, b)}
                onAsk={(q) => {
                  setAside("chat");
                  void useChat.getState().ask({ id: lesson.id, title: lesson.title, lang }, q);
                }}
                onClose={() => setRange(null)}
              />
            ) : (
              <ReaderChat lesson={{ id: lesson.id, title: lesson.title, lang }} />
            )}
          </motion.div>
        </AnimatePresence>
      </aside>

      <SimplifySheet open={simplify} onClose={() => setSimplify(false)} lessonTitle={lesson.title} text={lesson.text} lang={lang} />
      <AddToPlaylist lesson={adding?.lesson ?? null} all={adding?.all ?? []} onClose={() => setAdding(null)} />
    </div>
  );
}

function SimplifySheet({ open, onClose, lessonTitle, text, lang }: { open: boolean; onClose(): void; lessonTitle: string; text: string; lang: OpenedLesson["lesson"]["lang"] }) {
  const [level, setLevel] = useState("A2");
  const [out, setOut] = useState("");
  const [state, setState] = useState<"idle" | "run" | "done">("idle");
  const toast = useApp((s) => s.toast);
  const bump = useApp((s) => s.bumpLibrary);
  const openLesson = useApp((s) => s.openLesson);
  const openSettings = useApp((s) => s.openSettings);

  useEffect(() => {
    if (open) {
      setOut("");
      setState("idle");
    }
  }, [open]);

  const run = async () => {
    setState("run");
    setOut("");
    let acc = "";
    try {
      const res = await api().aiSimplify(lang, text, level, (p) => {
        acc += p;
        setOut(acc);
      });
      setOut(res);
      setState("done");
    } catch (e) {
      setState("idle");
      if (isNoModel(e)) {
        onClose();
        openSettings("ai");
      }
      toast(errorText(e), "error");
    }
  };

  const save = async () => {
    const id = await api().lessonCreate({ lang, title: `${lessonTitle} (${level})`, text: out, kind: "simplified", collection: t("Versions simplifiées", "Simplified versions") });
    bump();
    onClose();
    openLesson(id);
  };

  return (
    <Sheet
      open={open}
      onClose={() => state !== "run" && onClose()}
      title={t("Simplifier le texte", "Simplify the text")}
      footer={
        state === "done" ? (
          <>
            <button className="btn ghost" onClick={run}>
              {t("Recommencer", "Start over")}
            </button>
            <button className="btn primary" onClick={save}>
              {t("Créer la leçon simplifiée", "Create the simplified lesson")}
            </button>
          </>
        ) : (
          <button className="btn primary glow" onClick={run} disabled={state === "run"}>
            <Icon name="sparkle" size={15} /> {state === "run" ? t("Réécriture…", "Rewriting…") : t("Réécrire", "Rewrite")}
          </button>
        )
      }
    >
      <div className="import-pane">
        <p className="muted">
          {t(
            "L'IA locale réécrit la leçon avec des phrases plus courtes et un vocabulaire plus courant, en gardant le sens. Une nouvelle leçon est créée : l'originale reste intacte.",
            "The local AI rewrites the lesson with shorter sentences and more common vocabulary, keeping the meaning. A new lesson is created: the original stays intact.",
          )}
        </p>
        <div className="field">
          <span className="label">{t("Niveau visé", "Target level")}</span>
          <div style={{ display: "flex", gap: 8 }}>
            {LEVELS.map((l) => (
              <button key={l.id} className={`chip ${level === l.id ? "on" : ""}`} onClick={() => setLevel(l.id)} disabled={state === "run"}>
                {l.label} · {l.hint}
              </button>
            ))}
          </div>
        </div>
        {(state !== "idle" || out) && <div className="simplify-out">{out || "…"}</div>}
      </div>
    </Sheet>
  );
}
