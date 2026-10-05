import { AnimatePresence, motion } from "motion/react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Icon } from "../../components/Icon";
import { Markdown, frenchSpaces, plainText } from "../../components/Markdown";
import { Orb } from "../../components/ui";
import { api } from "../../lib/api";
import { attachedLesson, efforts, langWithArticle, suggestions, useChat, type LessonRef, type Pending, type Suggestion } from "../../lib/chat";
import { count, t } from "../../lib/i18n";
import { inLang } from "../../lib/langs";
import { useOnline } from "../../lib/online";
import { useApp } from "../../lib/store";
import type { ChatEffort, ChatMessage, LangCode, LessonSummary } from "../../lib/types";
import { useUserName } from "../../lib/user";

/**
 * Conversation avec l'IA (sur ce Mac ou en ligne) : messages, réponse qui s'écrit, réflexion,
 * champ de saisie. Sert dans la vue Chat et, en plus étroit (`compact`), à
 * côté du texte dans le lecteur. `lessonNow` : la leçon ouverte, que `/leçon`
 * joint d'un coup.
 */
export function ChatThread({ compact = false, lessonNow = null }: { compact?: boolean; lessonNow?: LessonRef | null }) {
  const thread = useChat((s) => s.thread);
  const pending = useChat((s) => s.pending);
  const error = useChat((s) => s.error);
  const draftLesson = useChat((s) => s.draftLesson);
  const appLang = useApp((s) => s.lang)();
  const lang = useChat((s) => s.lang) ?? appLang;
  const openSettings = useApp((s) => s.openSettings);
  const onlineChat = useOnline().chat;
  const messages = thread?.messages ?? [];
  const mine = pending && pending.chatId === thread?.chat.id ? pending : null;
  const lesson = attachedLesson({ thread, draftLesson });

  // on suit la réponse qui s'écrit tant que l'on est en bas de la conversation
  const scrollRef = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const onScroll = () => {
    const el = scrollRef.current;
    if (el) stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 90;
  };
  useLayoutEffect(() => {
    stick.current = true;
  }, [thread?.chat.id]);
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [thread?.chat.id, messages.length, mine?.answer, mine?.thought, mine?.phase, error]);

  const empty = !messages.length && !mine;
  return (
    <div className={`chat ${compact ? "compact" : ""}`}>
      <div className="chat-scroll" ref={scrollRef} onScroll={onScroll}>
        <div className="chat-inner">
          {empty ? (
            <Welcome key={thread?.chat.id ?? "new"} lang={lang} lesson={lesson} compact={compact} />
          ) : (
            messages.map((m) => <Message key={m.id} m={m} />)
          )}
          {mine && <Live p={mine} withLesson={!!lesson} />}
          {error && (
            <motion.div className="chat-error" initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }}>
              <span>{error.text}</span>
              {(error.noModel || onlineChat) && (
                <button className="btn sm soft" onClick={() => openSettings("ai")}>
                  {error.noModel ? t("Installer un modèle", "Install a model") : t("Réglages de l'IA", "AI settings")}
                </button>
              )}
            </motion.div>
          )}
        </div>
      </div>
      <Composer compact={compact} lang={lang} lesson={lesson} lessonNow={lessonNow} />
    </div>
  );
}

// ---------- messages ----------

function Welcome({ lang, lesson, compact }: { lang: LangCode; lesson: LessonRef | null; compact: boolean }) {
  const online = useOnline();
  const send = useChat((s) => s.send);
  const busy = useChat((s) => !!s.pending);
  const items = suggestions(lang, !!lesson);
  const use = (q: Suggestion) => {
    if (!q.draft) return void send(q.text);
    // à compléter : le curseur attend le sujet au bout de la phrase
    useChat.getState().setDraft(q.text);
    useChat.getState().focus();
  };
  const name = useUserName();
  return (
    <motion.div className="chat-welcome" initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.4, ease: [0.2, 0.8, 0.2, 1] }}>
      <div className="chat-dawn" aria-hidden="true">
        <Orb size={compact ? 30 : 42} />
      </div>
      <h2>
        {frenchSpaces(
          lesson
            ? t("Parlons de cette leçon", "Let's talk about this lesson")
            : name
              ? t(`Que voulez-vous comprendre, ${name} ?`, `What would you like to understand, ${name}?`)
              : t("Que voulez-vous comprendre ?", "What would you like to understand?"),
        )}
      </h2>
      <p>
        {frenchSpaces(
          lesson
            ? t(
                `Lumen a lu « ${lesson.title} ». Demandez le sens d'un mot, une règle de grammaire, un résumé…`,
                `Lumen has read “${lesson.title}”. Ask for the meaning of a word, a grammar rule, a summary…`,
              )
            : t(
                `Posez vos questions sur ${langWithArticle(lang)}, ou écrivez ${inLang(lang)} pour vous entraîner : Lumen vous répond et vous corrige.`,
                `Ask your questions about ${langWithArticle(lang)}, or write ${inLang(lang)} to practise: Lumen answers and corrects you.`,
              ),
        )}
      </p>
      <div className="chat-suggest">
        {items.map((q, i) => (
          <motion.button
            key={q.text}
            className={q.draft ? "draft" : undefined}
            disabled={busy}
            onClick={() => use(q)}
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ delay: 0.12 + i * 0.06, duration: 0.35, ease: [0.2, 0.8, 0.2, 1] }}
          >
            {frenchSpaces(q.label ?? q.text)}
          </motion.button>
        ))}
      </div>
      <p className="chat-private">
        {online.chat ? (
          <>
            <span className="dot online" />{" "}
            {t(`${online.provider.name} répond, en ligne : vos questions et la leçon jointe lui sont envoyées`, `${online.provider.name} answers, online: your questions and the attached lesson are sent to it`)}
          </>
        ) : (
          <>
            <span className="dot ok" /> {t("Calculé sur votre Mac, rien ne quitte votre ordinateur", "Computed on your Mac, nothing leaves your computer")}
          </>
        )}
      </p>
    </motion.div>
  );
}

function Message({ m }: { m: ChatMessage }) {
  if (m.role === "user") {
    return (
      <div className="msg user">
        <div className="bubble" dir="auto">
          {frenchSpaces(m.content)}
        </div>
      </div>
    );
  }
  return (
    <div className="msg ai">
      {m.thought && <Thought text={m.thought} secs={m.thought_secs} />}
      <Markdown text={m.content} />
      <div className="msg-actions">
        <CopyButton text={m.content} />
      </div>
    </div>
  );
}

/** Question qui vient d'être posée, et la réponse qui s'écrit. */
function Live({ p, withLesson }: { p: Pending; withLesson: boolean }) {
  return (
    <>
      <motion.div className="msg user" initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }} transition={{ type: "spring", stiffness: 420, damping: 34 }}>
        <div className="bubble" dir="auto">
          {frenchSpaces(p.question)}
        </div>
      </motion.div>
      <div className="msg ai live">
        {p.phase === "read" && (
          <motion.div className="chat-reading" initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: 0.15 }}>
            <span className="ember" />
            <span className="shimmer">{withLesson ? t("Lumen relit la leçon…", "Lumen is rereading the lesson…") : t("Lumen prépare sa réponse…", "Lumen is preparing an answer…")}</span>
          </motion.div>
        )}
        {p.phase !== "read" && p.think && <Thought text={p.thought} secs={p.thoughtSecs} live={p.phase === "think"} since={p.since} />}
        {p.answer && <Markdown text={p.answer} />}
      </div>
    </>
  );
}

function Thought({ text, secs, live = false, since = 0 }: { text: string; secs: number; live?: boolean; since?: number }) {
  const [open, setOpen] = useState(false);
  const boxRef = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (live && boxRef.current) boxRef.current.scrollTop = boxRef.current.scrollHeight;
  }, [text, live]);
  if (live) {
    return (
      <div className="thought live">
        <div className="thought-head">
          <span className="ember" />
          <span className="shimmer">{t("Réflexion", "Thinking")}</span>
          <Elapsed since={since} />
        </div>
        <div className="thought-box" ref={boxRef}>
          {text.trim()}
        </div>
      </div>
    );
  }
  return (
    <div className={`thought ${open ? "open" : ""}`}>
      <button className="thought-head" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <Icon name="bulb" size={14} />
        <span>{t(`A réfléchi ${thinkLength(secs)}`, `Thought ${thinkLength(secs)}`)}</span>
        <Icon name="chevron" size={14} className="chev" />
      </button>
      <AnimatePresence initial={false}>
        {open && text && (
          <motion.div
            className="thought-full"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.28, ease: [0.2, 0.8, 0.2, 1] }}
          >
            <div>{text.trim()}</div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

function thinkLength(secs: number): string {
  if (secs < 1) return t("un instant", "for a moment");
  const s = Math.round(secs);
  if (s < 60) return t(`pendant ${s} s`, `for ${s} s`);
  return t(`pendant ${Math.floor(s / 60)} min ${String(s % 60).padStart(2, "0")} s`, `for ${Math.floor(s / 60)} min ${String(s % 60).padStart(2, "0")} s`);
}

/** Secondes écoulées depuis `since`, mises à jour en direct. */
function Elapsed({ since }: { since: number }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 500);
    return () => window.clearInterval(t);
  }, []);
  return <span className="thought-time num">{Math.max(0, Math.floor((now - since) / 1000))} s</span>;
}

function CopyButton({ text }: { text: string }) {
  const toast = useApp((s) => s.toast);
  const [done, setDone] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(plainText(text));
      setDone(true);
      window.setTimeout(() => setDone(false), 1400);
    } catch {
      toast(t("La copie n'a pas pu se faire.", "Couldn't copy."), "error");
    }
  };
  return (
    <button className="msg-action" onClick={copy} aria-label={t("Copier la réponse", "Copy the answer")} title={t("Copier la réponse", "Copy the answer")}>
      <Icon name={done ? "check" : "copy"} size={14} />
      {done ? t("Copié", "Copied") : t("Copier", "Copy")}
    </button>
  );
}

// ---------- saisie ----------

interface Command {
  name: string;
  hint: string;
  run(): void;
}

/** Sans accents ni majuscules : « /lecon » trouve « /leçon ». */
const fold = (s: string) => s.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();

function Composer({ compact, lang, lesson, lessonNow }: { compact: boolean; lang: LangCode; lesson: LessonRef | null; lessonNow: LessonRef | null }) {
  const draft = useChat((s) => s.draft);
  const setDraft = useChat((s) => s.setDraft);
  const send = useChat((s) => s.send);
  const stop = useChat((s) => s.stop);
  const busy = useChat((s) => !!s.pending);
  const focusTick = useChat((s) => s.focusTick);
  const attach = useChat((s) => s.attach);
  const fresh = useChat((s) => s.fresh);
  const think = useApp((s) => s.settings.chat_think === "1");
  const effort = (useApp((s) => s.settings.chat_effort) || "medium") as ChatEffort;
  const setSetting = useApp((s) => s.setSetting);
  const ref = useRef<HTMLTextAreaElement>(null);
  const [pick, setPick] = useState(-1);
  const [picker, setPicker] = useState(false);
  const [effortMenu, setEffortMenu] = useState(false);

  // le champ grandit avec le texte, jusqu'à une limite
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, compact ? 150 : 200)}px`;
  }, [draft, compact]);
  useEffect(() => {
    const el = ref.current;
    if (!focusTick || !el) return;
    el.focus();
    // le curseur au bout du texte déjà écrit (question à compléter)
    el.setSelectionRange(el.value.length, el.value.length);
  }, [focusTick]);

  const toggleThink = () => void setSetting("chat_think", think ? "0" : "1");
  const commands: Command[] = [
    {
      name: t("/leçon", "/lesson"),
      hint: lessonNow ? t(`Joindre « ${lessonNow.title} »`, `Attach “${lessonNow.title}”`) : t("Joindre une leçon de votre bibliothèque", "Attach a lesson from your library"),
      run: () => (lessonNow ? void attach(lessonNow) : setPicker(true)),
    },
    { name: t("/nouveau", "/new"), hint: t("Commencer une nouvelle conversation", "Start a new conversation"), run: () => fresh(lessonNow && compact ? lessonNow : null) },
    {
      name: t("/réflexion", "/think"),
      hint: think ? t("Couper la réflexion", "Turn thinking off") : t("Laisser l'IA réfléchir avant de répondre", "Let the AI think before answering"),
      run: toggleThink,
    },
  ];
  if (lesson) commands.splice(1, 0, { name: t("/sans-leçon", "/no-lesson"), hint: t("Retirer la leçon de la conversation", "Remove the lesson from the conversation"), run: () => void attach(null) });
  const typed = draft.trimStart();
  const query = typed.startsWith("/") && !/\s/.test(typed) ? fold(typed) : null;
  const matches = query !== null ? commands.filter((c) => fold(c.name).startsWith(query)) : [];
  const sel = matches.length ? Math.min(Math.max(pick, 0), matches.length - 1) : -1;

  const runCommand = (c: Command) => {
    setDraft("");
    setPick(-1);
    c.run();
    ref.current?.focus();
  };
  const submit = () => {
    if (sel >= 0) return runCommand(matches[sel]);
    if (busy || !draft.trim()) return;
    void send(draft);
  };

  const placeholder = lesson
    ? t("Un mot, une phrase, la grammaire de la leçon…", "A word, a sentence, the grammar of the lesson…")
    : compact
      ? t("Posez votre question…", "Ask your question…")
      : t(`Posez une question, ou écrivez ${inLang(lang)}…`, `Ask a question, or write ${inLang(lang)}…`);

  return (
    <div className="composer-wrap">
      <AnimatePresence>
        {matches.length > 0 && (
          <motion.div className="chat-pop slash" role="listbox" initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: 4 }} transition={{ duration: 0.16 }}>
            {matches.map((c, i) => (
              <button
                key={c.name}
                role="option"
                aria-selected={i === sel}
                className={i === sel ? "on" : ""}
                onMouseEnter={() => setPick(i)}
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => runCommand(c)}
              >
                <code>{c.name}</code>
                <span>{c.hint}</span>
              </button>
            ))}
          </motion.div>
        )}
      </AnimatePresence>
      <div className={`composer ${busy ? "busy" : ""}`}>
        <div className="composer-context">
          {lesson ? (
            <span className="lesson-chip" title={lesson.title}>
              <Icon name="book" size={13} />
              <span>{lesson.title}</span>
              <button onClick={() => void attach(null)} aria-label={t("Retirer la leçon", "Remove the lesson")} title={t("Retirer la leçon", "Remove the lesson")}>
                <Icon name="close" size={11} stroke={2} />
              </button>
            </span>
          ) : (
            <Popover
              open={picker}
              onClose={() => setPicker(false)}
              anchor={
                <button className="ctx-btn" onClick={() => setPicker((o) => !o)} aria-expanded={picker}>
                  <Icon name="attach" size={14} /> {t("Joindre une leçon", "Attach a lesson")}
                </button>
              }
            >
              <LessonPicker
                lang={lang}
                now={lessonNow}
                onPick={(l) => {
                  setPicker(false);
                  void attach(l);
                  ref.current?.focus();
                }}
              />
            </Popover>
          )}
        </div>
        <textarea
          ref={ref}
          className="composer-input"
          rows={1}
          value={draft}
          dir="auto"
          placeholder={placeholder}
          aria-label={t("Votre message", "Your message")}
          onChange={(e) => {
            setDraft(e.target.value);
            setPick(-1);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              submit();
            } else if (matches.length && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
              e.preventDefault();
              setPick((sel + (e.key === "ArrowDown" ? 1 : matches.length - 1)) % matches.length);
            } else if (matches.length && e.key === "Tab") {
              e.preventDefault();
              setDraft(matches[sel].name);
            } else if (e.key === "Escape") {
              if (matches.length) setDraft("");
              else (e.target as HTMLTextAreaElement).blur();
              e.stopPropagation();
            }
          }}
        />
        <div className="composer-tools">
          <button
            className={`think-toggle ${think ? "on" : ""}`}
            onClick={toggleThink}
            aria-pressed={think}
            title={t(
              "Avec la réflexion, l'IA raisonne avant de répondre : plus juste pour les questions difficiles, mais plus lent.",
              "With thinking, the AI reasons before answering: more accurate for difficult questions, but slower.",
            )}
          >
            <Icon name="bulb" size={14} />
            {t("Réflexion", "Thinking")}
          </button>
          <AnimatePresence initial={false}>
            {think && (
              <motion.div className="effort-slot" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: 0.2 }}>
                <Popover
                  open={effortMenu}
                  onClose={() => setEffortMenu(false)}
                  anchor={
                    <button className="effort-btn" onClick={() => setEffortMenu((o) => !o)} aria-expanded={effortMenu} title={t("Effort : la longueur de la réflexion", "Effort: how long the AI thinks")}>
                      {efforts().find((x) => x.value === effort)?.label ?? t("Équilibré", "Balanced")}
                      <Icon name="chevron" size={13} />
                    </button>
                  }
                >
                  <div className="effort-menu" role="menu">
                    <span className="eyebrow">{t("Effort de réflexion", "Thinking effort")}</span>
                    {efforts().map((x) => (
                      <button
                        key={x.value}
                        role="menuitemradio"
                        aria-checked={effort === x.value}
                        className={effort === x.value ? "on" : ""}
                        onClick={() => {
                          void setSetting("chat_effort", x.value);
                          setEffortMenu(false);
                        }}
                      >
                        <span className="effort-level" data-level={x.value} aria-hidden="true">
                          <i />
                          <i />
                          <i />
                        </span>
                        <span>
                          <strong>{x.label}</strong>
                          <small>{x.hint}</small>
                        </span>
                        {effort === x.value && <Icon name="check" size={14} />}
                      </button>
                    ))}
                  </div>
                </Popover>
              </motion.div>
            )}
          </AnimatePresence>
          <span style={{ flex: 1 }} />
          {busy ? (
            <button className="send stop" onClick={() => void stop()} aria-label={t("Arrêter la réponse", "Stop the answer")} title={t("Arrêter la réponse (ce qui est écrit est gardé)", "Stop the answer (what is written is kept)")}>
              <Icon name="stop" size={13} />
            </button>
          ) : (
            <button className="send" onClick={submit} disabled={!draft.trim()} aria-label={t("Envoyer", "Send")} title={t("Envoyer (Entrée)", "Send (Return)")}>
              <Icon name="send" size={16} stroke={2.2} />
            </button>
          )}
        </div>
      </div>
      {!compact && <p className="composer-hint">{t("Entrée pour envoyer, Maj + Entrée pour aller à la ligne, / pour les commandes", "Return to send, Shift + Return for a new line, / for commands")}</p>}
    </div>
  );
}

/** Petite fenêtre qui s'ouvre au-dessus de son bouton (le champ est en bas de l'écran). */
function Popover({ open, onClose, anchor, children }: { open: boolean; onClose(): void; anchor: ReactNode; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [open, onClose]);
  return (
    <div ref={ref} className="pop-anchor">
      {anchor}
      <AnimatePresence>
        {open && (
          <motion.div className="chat-pop" initial={{ opacity: 0, y: 6, scale: 0.98 }} animate={{ opacity: 1, y: 0, scale: 1 }} exit={{ opacity: 0, y: 4, scale: 0.98 }} transition={{ duration: 0.16 }}>
            {children}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/** Leçons à joindre : celle qu'on lit d'abord, puis les plus récemment ouvertes. */
function LessonPicker({ lang, now, onPick }: { lang: LangCode; now: LessonRef | null; onPick(l: LessonRef): void }) {
  const [lessons, setLessons] = useState<LessonSummary[] | null>(null);
  useEffect(() => {
    api()
      .lessonsList(lang)
      .then(setLessons)
      .catch(() => setLessons([]));
  }, [lang]);
  const shown = useMemo(() => {
    const all = [...(lessons ?? [])].sort((a, b) => (b.opened_at ?? b.created_at) - (a.opened_at ?? a.created_at));
    if (now) {
      const i = all.findIndex((l) => l.id === now.id);
      if (i > 0) all.unshift(...all.splice(i, 1));
    }
    return all.slice(0, 9);
  }, [lessons, now]);
  return (
    <div className="lesson-pick" role="menu">
      <span className="eyebrow">{t("Joindre une leçon", "Attach a lesson")}</span>
      {lessons === null ? (
        <div className="skeleton" style={{ height: 30, margin: "6px 8px" }} />
      ) : shown.length === 0 ? (
        <p className="muted">{t("Aucune leçon dans cette langue pour l'instant.", "No lessons in this language yet.")}</p>
      ) : (
        shown.map((l) => (
          <button key={l.id} role="menuitem" onClick={() => onPick({ id: l.id, title: l.title })}>
            <Icon name={l.id === now?.id ? "book" : "text"} size={14} />
            <span>
              <strong>{l.title}</strong>
              <small>{l.id === now?.id ? t("Lecture en cours", "Now reading") : l.collection || count(l.word_count, "mot", "mots", "word", "words")}</small>
            </span>
          </button>
        ))
      )}
    </div>
  );
}
