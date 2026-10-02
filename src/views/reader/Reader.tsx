import { AnimatePresence, motion, useAnimationControls } from "motion/react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Icon } from "../../components/Icon";
import { Orb, Sheet } from "../../components/ui";
import { api, errorText, isNoModel } from "../../lib/api";
import { LEVELS, langInfo } from "../../lib/langs";
import { formatNumber, useApp } from "../../lib/store";
import { paginate, sentenceBounds } from "../../lib/tokenize";
import type { OpenedLesson, Term } from "../../lib/types";
import { Player, type PlayerHandle, type PlaybackState } from "./Player";
import { VideoStage } from "./VideoStage";
import { WordPanel, type Selection } from "./WordPanel";

interface Range {
  a: number;
  b: number;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
/** Hauteur (part de la zone visible) de la ligne où l'œil lit : le point de reprise y revient. */
const READ_LINE = 0.3;

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
  const bump = useApp((s) => s.bumpLibrary);
  const refreshKnown = useApp((s) => s.refreshKnown);
  const openLesson = useApp((s) => s.openLesson);

  const [data, setData] = useState<OpenedLesson | null>(null);
  const [terms, setTerms] = useState<Record<string, Term>>({});
  const [page, setPage] = useState(0);
  const [range, setRange] = useState<Range | null>(null);
  const [cursor, setCursor] = useState(-1);
  const [illum, setIllum] = useState(false);
  const [complete, setComplete] = useState<null | { read: number; known: number; lingqs: number; next: number | null }>(null);
  const [scrolled, setScrolled] = useState(false);
  const [simplify, setSimplify] = useState(false);
  const [glowKeys, setGlowKeys] = useState<Set<string>>(new Set());

  const scrollRef = useRef<HTMLDivElement>(null);
  const pageRef = useRef<HTMLDivElement>(null);
  const playerRef = useRef<PlayerHandle>(null);
  const drag = useRef<{ start: number; moved: boolean } | null>(null);
  const session = useRef({ read: 0, known: 0, lingqs: 0 });
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
    session.current = { read: 0, known: 0, lingqs: 0 };
    api()
      .lessonOpen(lessonId)
      .then((d) => {
        if (!alive) return;
        setData(d);
        setTerms(d.terms);
        const p = paginate(d.tokens);
        setPage(Math.min(d.lesson.page, p.length - 1));
      })
      .catch(() => {
        if (!alive) return;
        // leçon supprimée entre-temps : on l'oublie
        useApp.getState().forgetLesson(lessonId);
        toast("Cette leçon n'existe plus.", "error");
        go("library");
      });
    void api().aiWarmup().catch(() => {});
    return () => {
      alive = false;
    };
  }, [lessonId, toast, go]);

  const lesson = data?.lesson;
  const tokens = useMemo(() => data?.tokens ?? [], [data]);
  const pages = useMemo(() => paginate(tokens), [tokens]);
  const pr = pages[page] ?? { start: 0, end: 0, words: 0 };
  const lang = lesson?.lang ?? "en";

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
    if (cursor >= 0) anchorRef.current.want = cursor;
  }, [cursor]);

  // à l'ouverture : retour au mot où l'on s'était arrêté, signalé par un bref halo
  useLayoutEffect(() => {
    if (!data || restoredFor.current === data.lesson.id) return;
    restoredFor.current = data.lesson.id;
    const a = data.lesson.anchor;
    const range = pages[page];
    if (!range || a <= range.start || a >= range.end) return;
    const sc = scrollRef.current;
    const el = pageRef.current?.querySelector(`[data-i="${a}"]`);
    if (!sc || !el) return;
    const top = el.getBoundingClientRect().top - sc.getBoundingClientRect().top + sc.scrollTop;
    // ce défilement automatique ne doit pas déplacer le point de reprise
    ignoreScrollUntil.current = performance.now() + 600;
    sc.scrollTop = Math.max(0, top - sc.clientHeight * READ_LINE);
    // avec un média, la lanterne montre déjà l'endroit
    if (!(data.lesson.media_path && data.lesson.position > 0.5)) setResumeAt(a);
  }, [data, pages, page]);
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
    return {
      surface: clean(lesson.text.slice(selStart, selEnd)),
      key: words.join(" "),
      sentence: clean(lesson.text.slice(sentStart, sentEnd)).trim(),
      before: clean(lesson.text.slice(sentStart, selStart)).trimStart(),
      after: clean(lesson.text.slice(selEnd, sentEnd)).trimEnd(),
      isPhrase,
      tokenIndex: a,
      phrase: ph ? { key: ph.key, a: ph.a, b: ph.b } : null,
    };
  }, [range, tokens, lesson, phraseCover]);

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

  const select = useCallback(
    (a: number, b: number) => {
      if (a > b) [a, b] = [b, a];
      // retire la ponctuation aux extrémités
      while (a < b && !tokens[a].w) a++;
      while (b > a && !tokens[b].w) b--;
      if (!tokens[a]?.w) return;
      setRange({ a, b });
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
    [tokens, statusOf, setStatus, lesson],
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
  const rangeRef = useRef<Range | null>(null);
  rangeRef.current = range;
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
  const goPage = useCallback(
    (p: number) => {
      if (!lesson || p < 0 || p >= pages.length) return;
      setPage(p);
      setRange(null);
      scrollRef.current?.scrollTo({ top: 0, behavior: "smooth" });
      anchorRef.current.want = pages[p].start;
      void api().lessonUpdate(lesson.id, { page: p });
    },
    [lesson, pages],
  );

  const onPlayerPage = useCallback(
    (p: number) => {
      setPage(p);
      setRange(null);
      if (lesson) void api().lessonUpdate(lesson.id, { page: p });
    },
    [lesson],
  );

  const findNext = async () => {
    if (!lesson) return null;
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
    try {
      if (marks && keys.length) {
        setIllum(true);
        await sleep(1050);
        added = await api().termsMarkKnown(lesson.lang, keys, pr.words);
        setTerms((prev) => {
          const next = { ...prev };
          for (const k of keys) if (!next[k]) next[k] = { term: k, status: 4, translation: "", note: "", lemma: "", context: "", updated_at: Date.now() / 1000 };
          return next;
        });
        setIllum(false);
      } else {
        await api().activityAdd(lesson.lang, pr.words, 0);
      }
    } catch (e) {
      setIllum(false);
      toast(errorText(e), "error");
      return;
    }
    session.current.read += pr.words;
    session.current.known += added;
    void refreshKnown();
    if (page < pages.length - 1) {
      goPage(page + 1);
      if (added) toast(`${added} mot${added > 1 ? "s" : ""} rejoigne${added > 1 ? "nt" : ""} vos mots connus`, "light");
    } else {
      playerRef.current?.stop();
      // leçon terminée : la prochaine lecture repart du début
      playerRef.current?.forget();
      anchorRef.current = { id: lesson.id, want: 0, saved: 0 };
      await api().lessonUpdate(lesson.id, { completed: true, page: 0, position: 0, anchor: 0 });
      bump();
      const next = await findNext();
      setComplete({ ...session.current, next });
    }
  };

  // ---------- clavier ----------
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t.closest("input, textarea, select, [contenteditable]") || e.metaKey || e.ctrlKey) return;
      if (useApp.getState().importOpen || simplify || complete) return;
      const words: number[] = [];
      for (let i = pr.start; i < pr.end; i++) if (tokens[i].w) words.push(i);
      const cur = range ? words.indexOf(range.a) : -1;
      const key = e.key.toLowerCase();
      if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
        e.preventDefault();
        const n = e.key === "ArrowRight" ? (cur < 0 ? 0 : Math.min(words.length - 1, cur + 1)) : cur < 0 ? words.length - 1 : Math.max(0, cur - 1);
        if (words[n] !== undefined) select(words[n], words[n]);
      } else if (["1", "2", "3"].includes(key) && selection) {
        setStatus(selection.key, Number(key));
      } else if ((key === "k" || key === "4") && selection) {
        setStatus(selection.key, 4);
      } else if (key === "x" && selection) {
        setStatus(selection.key, 5);
      } else if (key === "0" && selection) {
        setStatus(selection.key, 0);
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

  // ---------- lanterne (mot prononcé) ----------
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
  }, [cursor, page, pr.start, pr.end]);

  if (!lessonId) {
    return (
      <div className="empty" style={{ flex: 1, justifyContent: "center" }}>
        <Orb size={44} />
        <h3>Aucune lecture en cours</h3>
        <p>Choisissez une leçon dans la bibliothèque.</p>
        <button className="btn primary" onClick={() => go("library")}>
          Bibliothèque
        </button>
      </div>
    );
  }

  if (!data || !lesson) {
    return (
      <div className="reader">
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
  const fontSize = Number(settings.font_size) || 23;
  const lineHeight = Number(settings.line_height) || 1.75;
  const markClass = settings.word_style === "line" ? "mark-line" : "mark-tint";
  const paragraphs: React.ReactNode[] = [];
  let current: React.ReactNode[] = [];
  const total = Math.max(1, pr.end - pr.start);
  for (let i = pr.start; i < pr.end; i++) {
    const t = tokens[i];
    const ph = phraseCover.get(i);
    const inPhrase = ph && i > ph.a && i <= ph.b;
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
          inPhrase ? (
            <span key={i} className={`ph p${ph!.status}`}>
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
    if (selected) cls += range!.a === range!.b ? " sel" : " sel-range";
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

  return (
    <div className={`reader ${cinema ? "cinema" : ""}`}>
      <div className="reader-col">
        <div className="reader-top drag" data-tauri-drag-region>
          <button className="icon-btn no-drag" onClick={() => go("library")} aria-label="Retour à la bibliothèque">
            <Icon name="back" size={18} />
          </button>
          <span className={`title-mini ${scrolled ? "show" : ""}`} data-tauri-drag-region>
            {lesson.title}
          </span>
          <button className="btn sm soft no-drag" onClick={() => setSimplify(true)} title="Réécrire ce texte à un niveau plus simple">
            <Icon name="sparkle" size={14} /> Simplifier
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
              playerRef.current?.pause();
              select(i, i);
            }}
            onVideoReady={(path) => setData((d) => (d ? { ...d, lesson: { ...d.lesson, video_path: path } } : d))}
            cinema={cinema}
            setCinema={setCinema}
          />
        )}

        <div className="reader-scroll" ref={scrollRef} onScroll={onReaderScroll}>
          <div className="reader-inner">
            <header className="reader-head">
              <div className="reader-crumb">
                <button onClick={() => go("library")}>Bibliothèque</button>
                {lesson.collection && (
                  <>
                    <span>›</span>
                    <span>{lesson.collection}</span>
                  </>
                )}
              </div>
              <h1>{lesson.title}</h1>
              <div className="reader-chips">
                <span className="chip">{li.name}</span>
                <span className="chip num">
                  Page {page + 1} sur {pages.length}
                </span>
                <span className="chip new num">
                  {newOnPage} nouveau{newOnPage > 1 ? "x" : ""}
                </span>
                <span className="chip num">{formatNumber(pr.words)} mots</span>
              </div>
            </header>

            <AnimatePresence mode="wait" initial={false}>
              <motion.div
                key={page}
                ref={pageRef}
                className={`page ${markClass} ${illum ? "illuminate" : ""}`}
                style={{ ["--read-size" as string]: `${fontSize}px`, ["--read-lh" as string]: lineHeight }}
                onPointerDown={onPointerDown}
                onPointerOver={onPointerOver}
                lang={lang}
                initial={{ opacity: 0, x: 24, filter: "blur(4px)" }}
                animate={{ opacity: 1, x: 0, filter: "blur(0px)" }}
                exit={{ opacity: 0, x: -24, filter: "blur(4px)" }}
                transition={{ duration: 0.32, ease: [0.2, 0.8, 0.2, 1] }}
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
              </motion.div>
            </AnimatePresence>

            <div className="page-end">
              {pages.length > 1 && (
                <div className="page-nav">
                  <button className="icon-btn" onClick={() => goPage(page - 1)} disabled={page === 0} aria-label="Page précédente">
                    <Icon name="left" />
                  </button>
                  <span className="num">
                    {page + 1} / {pages.length}
                  </span>
                  <button className="icon-btn" onClick={() => goPage(page + 1)} disabled={isLast} aria-label="Page suivante sans marquer">
                    <Icon name="right" />
                  </button>
                </div>
              )}
              <p>
                {settings.finish_marks_known !== "0" && newOnPage > 0
                  ? `Les ${newOnPage} mots bleus que vous n'avez pas consultés rejoindront vos mots connus.`
                  : isLast
                    ? "Dernière page de la leçon."
                    : "Tous les mots de cette page sont déjà rencontrés."}
              </p>
              <button className="btn primary lg glow" onClick={finishPage} disabled={illum}>
                {isLast ? "Terminer la leçon" : "Terminer la page"}
                <Icon name="forward" size={16} stroke={2} />
              </button>
            </div>
            {pages.length > 1 && (
              <div className="page-dots" aria-hidden="true">
                {pages.map((_, i) => (
                  <button key={i} className={i === page ? "on" : i < page ? "done" : ""} onClick={() => goPage(i)} tabIndex={-1} />
                ))}
              </div>
            )}
          </div>
        </div>

        <Player
          key={`player-${lesson.id}`}
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
        />

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
                <h2>Leçon terminée</h2>
                <p className="muted">{lesson.title}</p>
                <div className="complete-stats">
                  <div>
                    <strong>{formatNumber(complete.read)}</strong>
                    <span>mots lus</span>
                  </div>
                  <div>
                    <strong>{formatNumber(complete.known)}</strong>
                    <span>nouveaux connus</span>
                  </div>
                  <div>
                    <strong>{formatNumber(complete.lingqs)}</strong>
                    <span>mots étudiés</span>
                  </div>
                </div>
                <div style={{ display: "flex", gap: 10 }}>
                  <button className="btn outline lg" onClick={() => go("library")}>
                    Bibliothèque
                  </button>
                  {complete.next ? (
                    <button className="btn primary lg glow" onClick={() => openLesson(complete.next!)}>
                      Leçon suivante <Icon name="forward" size={16} />
                    </button>
                  ) : (
                    <button
                      className="btn primary lg glow"
                      onClick={() => {
                        setComplete(null);
                        goPage(0);
                      }}
                    >
                      Relire
                    </button>
                  )}
                </div>
              </motion.div>
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      <WordPanel
        lang={lang}
        sel={selection}
        term={selection ? terms[selection.key] : undefined}
        onStatus={(k, s) => setStatus(k, s)}
        onTranslation={onTranslation}
        onPlayFrom={(i) => playerRef.current?.playFrom(i)}
        onSelectPhrase={(a, b) => setRange({ a, b })}
        onClose={() => setRange(null)}
      />

      <SimplifySheet open={simplify} onClose={() => setSimplify(false)} lessonTitle={lesson.title} text={lesson.text} lang={lang} />
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
  const go = useApp((s) => s.go);

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
        go("settings");
      }
      toast(errorText(e), "error");
    }
  };

  const save = async () => {
    const id = await api().lessonCreate({ lang, title: `${lessonTitle} (${level})`, text: out, kind: "simplified", collection: "Versions simplifiées" });
    bump();
    onClose();
    openLesson(id);
  };

  return (
    <Sheet
      open={open}
      onClose={() => state !== "run" && onClose()}
      title="Simplifier le texte"
      footer={
        state === "done" ? (
          <>
            <button className="btn ghost" onClick={run}>
              Recommencer
            </button>
            <button className="btn primary" onClick={save}>
              Créer la leçon simplifiée
            </button>
          </>
        ) : (
          <button className="btn primary glow" onClick={run} disabled={state === "run"}>
            <Icon name="sparkle" size={15} /> {state === "run" ? "Réécriture…" : "Réécrire"}
          </button>
        )
      }
    >
      <div className="import-pane">
        <p className="muted">L'IA locale réécrit la leçon avec des phrases plus courtes et un vocabulaire plus courant, en gardant le sens. Une nouvelle leçon est créée : l'originale reste intacte.</p>
        <div className="field">
          <span className="label">Niveau visé</span>
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
