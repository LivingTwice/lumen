import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { Fragment, useEffect, useMemo, useState, type ReactNode } from "react";
import { count, t } from "../lib/i18n";
import { useApp } from "../lib/store";
import type { LangCode, ModelRow } from "../lib/types";
import { Icon, type IconName } from "./Icon";

/* Le petit guide : quatre cartes illustrées pour comprendre Lumen en une minute
   (le principe, toucher puis tourner la page, la lanterne, les modèles d'IA). */

const EASE = [0.16, 1, 0.3, 1] as const;

/** Phrase des scènes : la lumière du matin entre dans la chambre, dans la langue étudiée si possible. */
interface Sample {
  text: string;
  /** statut de chaque mot : 0 nouveau, 1 à 3 en apprentissage, 4 connu */
  st: number[];
  /** mot nouveau touché dans la deuxième scène, et son sens en contexte */
  tap: number;
  fr: string;
  en: string;
}

const SAMPLES: Partial<Record<LangCode, Sample>> = {
  it: { text: "La luce del mattino entra piano nella stanza.", st: [4, 2, 4, 0, 4, 0, 4, 0], tap: 5, fr: "doucement", en: "softly" },
  es: { text: "La luz de la mañana entra despacio en la habitación.", st: [4, 2, 4, 4, 0, 4, 0, 4, 4, 0], tap: 6, fr: "lentement", en: "slowly" },
  de: { text: "Das Morgenlicht fällt leise ins Zimmer.", st: [4, 0, 2, 0, 4, 0], tap: 3, fr: "doucement", en: "softly" },
  pt: { text: "A luz da manhã entra devagar no quarto.", st: [4, 2, 4, 0, 4, 0, 4, 0], tap: 5, fr: "lentement", en: "slowly" },
  ru: { text: "Утренний свет тихо входит в комнату.", st: [0, 2, 0, 4, 4, 0], tap: 2, fr: "doucement", en: "quietly" },
  fr: { text: "La lumière du matin entre doucement dans la chambre.", st: [4, 2, 4, 0, 4, 0, 4, 4, 0], tap: 5, fr: "sans bruit", en: "gently" },
  en: { text: "The morning light drifts softly into the room.", st: [4, 0, 2, 0, 4, 4, 4, 0], tap: 3, fr: "glisse", en: "floats gently" },
};

const WORD = /([\p{L}\p{M}\p{N}]+)/u;

/** La phrase d'exemple, mot par mot ; `word` dessine chaque mot selon la scène. */
function Line({ sample, lang, word }: { sample: Sample; lang: LangCode; word(i: number, text: string): ReactNode }) {
  const parts = useMemo(() => sample.text.split(WORD), [sample]);
  let n = -1;
  return (
    <p className="g-line" lang={lang}>
      {parts.map((p, k) => (k % 2 ? <Fragment key={k}>{word(++n, p)}</Fragment> : p))}
    </p>
  );
}

/** 1. Les couleurs : la page s'éclaire à mesure que les mots deviennent familiers. */
function SceneLight({ sample, lang, still }: { sample: Sample; lang: LangCode; still: boolean }) {
  const [lit, setLit] = useState(still ? sample.st.length : 0);
  useEffect(() => {
    if (still) return;
    let k = 0;
    let tick = 0;
    const start = window.setTimeout(() => {
      tick = window.setInterval(() => {
        setLit(++k);
        if (k >= sample.st.length) window.clearInterval(tick);
      }, 230);
    }, 650);
    return () => {
      window.clearTimeout(start);
      window.clearInterval(tick);
    };
  }, [sample, still]);

  return (
    <div className="g-scene">
      <Line
        sample={sample}
        lang={lang}
        word={(i, w) => {
          const s = i < lit ? sample.st[i] : 0;
          return <span className={`g-w s${s} ${s === 4 && !still ? "flash" : ""}`}>{w}</span>;
        }}
      />
      <div className="g-legend">
        <span>
          <i className="sw s0" /> {t("Nouveau", "New")}
        </span>
        <span>
          <i className="sw s1" /> {t("En apprentissage", "Learning")}
        </span>
        <span>
          <i className="sw s4" /> {t("Connu", "Known")}
        </span>
      </div>
    </div>
  );
}

/** 2. On touche un mot inconnu, puis on termine la page : les mots bleus restants deviennent connus. */
function SceneTap({ sample, lang, still }: { sample: Sample; lang: LangCode; still: boolean }) {
  // 0 : la page ; 1 : un mot touché, son sens s'affiche ; 2 : « Terminer la page »
  const [phase, setPhase] = useState(still ? 1 : 0);
  const [loop, setLoop] = useState(0);
  useEffect(() => {
    if (still) return;
    const steps = [
      window.setTimeout(() => setPhase(1), 900),
      window.setTimeout(() => setPhase(2), 3900),
      window.setTimeout(() => {
        setPhase(0);
        setLoop((l) => l + 1);
      }, 7600),
    ];
    return () => steps.forEach((s) => window.clearTimeout(s));
  }, [loop, still]);

  const rest = sample.st.filter((s, i) => s === 0 && i !== sample.tap).length;
  let order = 0;

  return (
    <motion.div key={loop} className="g-scene" initial={{ opacity: loop ? 0 : 1 }} animate={{ opacity: 1 }} transition={{ duration: 0.5 }}>
      <Line
        sample={sample}
        lang={lang}
        word={(i, w) => {
          if (i === sample.tap) {
            return (
              <span className={`g-w s${phase >= 1 ? 1 : 0} ${phase === 1 ? "sel tap" : ""}`}>
                {w}
                <AnimatePresence>
                  {phase === 1 && (
                    <motion.span
                      className="g-bubble"
                      style={{ x: "-50%" }}
                      initial={{ opacity: 0, y: 6, scale: 0.92 }}
                      animate={{ opacity: 1, y: 0, scale: 1 }}
                      exit={{ opacity: 0, y: 4 }}
                      transition={{ type: "spring", stiffness: 420, damping: 28, delay: still ? 0 : 0.25 }}
                    >
                      <Icon name="sparkle" size={12} /> {t(sample.fr, sample.en)}
                    </motion.span>
                  )}
                </AnimatePresence>
              </span>
            );
          }
          const fresh = sample.st[i] === 0;
          const d = fresh ? order++ * 0.18 : 0;
          return (
            <span className={`g-w s${fresh && phase === 2 ? 4 : sample.st[i]} ${fresh && phase === 2 ? "flash" : ""}`} style={{ "--d": `${d}s` } as React.CSSProperties}>
              {w}
            </span>
          );
        }}
      />
      <div className="g-finish">
        <span className={`g-finish-btn ${phase === 2 ? "press" : ""}`}>
          <Icon name="check" size={13} stroke={2.2} /> {t("Terminer la page", "Finish page")}
        </span>
        <AnimatePresence>
          {phase === 2 && (
            <motion.span
              className="g-gain"
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0 }}
              transition={{ delay: 0.5, duration: 0.5, ease: EASE }}
            >
              +{count(rest, "mot connu", "mots connus", "known word", "known words")}
            </motion.span>
          )}
        </AnimatePresence>
      </div>
    </motion.div>
  );
}

/** 3. La lanterne glisse sur chaque mot prononcé. */
function SceneListen({ sample, lang, still }: { sample: Sample; lang: LangCode; still: boolean }) {
  const n = sample.st.length;
  // k : mot atteint (au-delà de n : trois temps de silence) ; pass : passage en cours
  const [{ k, pass }, setPos] = useState({ k: still ? 2 : -2, pass: 0 });
  useEffect(() => {
    if (still) return;
    const tick = window.setInterval(() => setPos((p) => (p.k + 1 >= n + 3 ? { k: -1, pass: p.pass + 1 } : { k: p.k + 1, pass: p.pass })), 440);
    return () => window.clearInterval(tick);
  }, [n, still]);
  const at = k >= 0 && k < n ? k : -1;

  return (
    <div className="g-scene">
      <Line
        sample={sample}
        lang={lang}
        word={(i, w) => (
          // la page de la scène précédente, une fois terminée
          <span className={`g-w s${i === sample.tap ? 1 : sample.st[i] || 4}`}>
            {at === i && (
              <motion.span
                // un passage = une lanterne neuve : elle ne revient pas en glissant de la fin au début
                layoutId={`g-lantern-${pass}`}
                className="g-lantern"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                transition={{ type: "spring", stiffness: 520, damping: 42, mass: 0.6 }}
              />
            )}
            {w}
          </span>
        )}
      />
      <div className="g-player" aria-hidden="true">
        <span className="g-play">
          <Icon name={at >= 0 ? "pause" : "play"} size={13} />
        </span>
        <span className="g-track">
          <i style={{ width: `${((at >= 0 ? at + 1 : k >= n ? n : 0) / n) * 100}%` }} />
        </span>
        <span className={`g-wave ${at >= 0 ? "on" : ""}`}>
          <i />
          <i />
          <i />
          <i />
        </span>
      </div>
    </div>
  );
}

interface Role {
  kind: ModelRow["kind"];
  icon: IconName;
  name: string;
  model: string;
  what: string;
  ready: string;
  optional: boolean;
}

const roles = (): Role[] => [
  {
    kind: "llm",
    icon: "sparkle",
    name: t("Le traducteur", "The translator"),
    model: "Qwen3.5",
    what: t("Le sens de chaque mot dans sa phrase, et le chat.", "What each word means in its sentence, and the chat."),
    ready: t("Prêt", "Ready"),
    optional: false,
  },
  {
    kind: "asr",
    icon: "wave",
    name: t("L'oreille", "The ear"),
    model: "Whisper",
    what: t("Écoute l'audio et situe chaque mot dans le temps.", "Listens to audio and places each word in time."),
    ready: t("Prête", "Ready"),
    optional: false,
  },
  {
    kind: "asrtext",
    icon: "edit",
    name: t("La plume", "The pen"),
    model: "Qwen3-ASR",
    what: t("Écrit plus fidèlement ce que l'oreille entend.", "Writes down what the ear hears, more faithfully."),
    ready: t("Prête", "Ready"),
    optional: true,
  },
  {
    kind: "tts",
    icon: "speaker",
    name: t("La voix", "The voice"),
    model: "Supertonic",
    what: t("Prononce les mots et lit vos leçons à voix haute.", "Pronounces words and reads your lessons aloud."),
    ready: t("Prête", "Ready"),
    optional: true,
  },
];

/** 4. Quatre modèles, quatre rôles, tous sur ce Mac. */
function SceneModels({ still }: { still: boolean }) {
  const models = useApp((s) => s.models);
  return (
    <div className="g-scene models">
      <div className="g-roles">
        {roles().map((r, i) => {
          const installed = models.some((m) => m.kind === r.kind && m.installed);
          return (
            <motion.div
              key={r.kind}
              className="g-role"
              initial={still ? false : { opacity: 0, y: 10, scale: 0.98 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              transition={{ delay: 0.15 + i * 0.09, duration: 0.55, ease: EASE }}
            >
              <span className="ic">
                <Icon name={r.icon} size={16} />
              </span>
              <span className="body">
                <span className="head">
                  <strong>{r.name}</strong>
                  <span className="model">{r.model}</span>
                </span>
                <span className="what">{r.what}</span>
                <span className={`state ${installed ? "ok" : ""}`}>
                  <i />
                  {installed ? r.ready : r.optional ? t("Facultatif", "Optional") : t("À installer", "To install")}
                </span>
              </span>
            </motion.div>
          );
        })}
      </div>
    </div>
  );
}

interface Card {
  title: string;
  body: string;
  note?: string;
}

const cards = (): Card[] => [
  {
    title: t("Lisez ce qui vous plaît", "Read what you love"),
    body: t(
      "Chaque mot porte une couleur : bleu s'il est nouveau, ambré pendant que vous l'apprenez, sans couleur une fois connu. Plus vous lisez, plus la page s'éclaire.",
      "Every word wears a color: blue when it's new, amber while you're learning it, plain once you know it. The more you read, the brighter the page.",
    ),
  },
  {
    title: t("Touchez, puis tournez la page", "Tap, then turn the page"),
    body: t(
      "Touchez un mot inconnu : le dictionnaire et l'IA vous donnent son sens dans cette phrase précise. Terminez la page : les mots bleus que vous n'avez pas touchés, vous les avez compris, ils deviennent connus.",
      "Tap a word you don't know: the dictionary and the AI give you its meaning in this very sentence. Finish the page: the blue words you didn't tap, you understood, so they become known.",
    ),
  },
  {
    title: t("Écoutez, la lanterne suit", "Listen, the lantern follows"),
    body: t(
      "Un halo glisse sur chaque mot prononcé : vous lisez et vous entendez en même temps. Importez un livre, un article, un podcast ou une vidéo YouTube, Lumen en fait une leçon.",
      "A halo glides over each spoken word, so you read and hear at the same time. Import a book, an article, a podcast or a YouTube video, and Lumen turns it into a lesson.",
    ),
  },
  {
    title: t("Une IA qui vit sur votre Mac", "An AI that lives on your Mac"),
    body: t(
      "Un modèle, c'est un gros fichier qui a lu des milliards de phrases. Téléchargé une fois, il travaille ici, même sans Internet : ce que vous lisez ne quitte jamais votre Mac.",
      "A model is a big file that has read billions of sentences. Downloaded once, it works right here, even offline: what you read never leaves your Mac.",
    ),
    note: t(
      "Léger, Équilibré ou Maximum : c'est la taille du traducteur. Plus il est grand, plus il est fin, mais plus il pèse et demande de mémoire.",
      "Light, Balanced or Maximum is the size of the translator. The bigger it is, the finer it gets, but the more space and memory it needs.",
    ),
  },
];

export function Guide() {
  const open = useApp((s) => s.guide);
  const close = useApp((s) => s.closeGuide);
  const go = useApp((s) => s.go);
  const lang = useApp((s) => s.lang)();
  const still = !!useReducedMotion();
  const [card, setCard] = useState(0);
  const [dir, setDir] = useState(1);
  const list = cards();
  const last = card === list.length - 1;
  // la langue étudiée si elle a sa phrase d'exemple, sinon l'italien
  const sampleLang: LangCode = SAMPLES[lang] ? lang : "it";
  const sample = SAMPLES[sampleLang]!;

  useEffect(() => {
    if (open !== null) {
      setDir(1);
      setCard(open);
    }
  }, [open]);

  const goTo = (i: number) => {
    if (i < 0 || i >= list.length || i === card) return;
    setDir(i > card ? 1 : -1);
    setCard(i);
  };
  const next = () => (last ? close() : goTo(card + 1));

  // le guide est modal : le lecteur (flèches, Entrée, Espace) n'entend rien pendant ce temps
  useEffect(() => {
    if (open === null) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey) return;
      e.stopPropagation();
      // Entrée ou Espace sur un bouton : le bouton s'en charge
      const onButton = !!(e.target as HTMLElement).closest?.("button");
      if (e.key === "Escape") close();
      else if (e.key === "ArrowRight" || (e.key === "Enter" && !onButton)) {
        e.preventDefault();
        next();
      } else if (e.key === "ArrowLeft") {
        e.preventDefault();
        goTo(card - 1);
      } else if (e.key === " " && !onButton) e.preventDefault();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  });

  const seeModels = () => {
    close();
    go("settings");
    window.setTimeout(() => document.getElementById("set-ai")?.scrollIntoView({ behavior: "smooth", block: "start" }), 350);
  };

  const c = list[card];
  const slide = {
    initial: (d: number) => (still ? { opacity: 0 } : { opacity: 0, x: d * 28, filter: "blur(6px)" }),
    animate: { opacity: 1, x: 0, filter: "blur(0px)" },
    exit: (d: number) => (still ? { opacity: 0 } : { opacity: 0, x: d * -20, filter: "blur(6px)" }),
  };

  return (
    <AnimatePresence>
      {open !== null && (
        <motion.div className="scrim guide-scrim" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} onMouseDown={(e) => e.target === e.currentTarget && close()}>
          <motion.div
            className="guide"
            role="dialog"
            aria-modal="true"
            aria-label={t("Le petit guide de Lumen", "Lumen's short guide")}
            initial={{ opacity: 0, y: 24, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 12, scale: 0.98 }}
            transition={{ type: "spring", stiffness: 340, damping: 32 }}
          >
            <div className="guide-head">
              <span className="eyebrow">{t("Le petit guide", "The short guide")}</span>
              <button className="icon-btn" onClick={close} aria-label={t("Fermer le guide", "Close the guide")}>
                <Icon name="close" size={16} />
              </button>
            </div>

            <div className="guide-stage">
              <AnimatePresence mode="wait" custom={dir} initial={false}>
                <motion.div key={card} className="guide-scene-wrap" custom={dir} variants={slide} initial="initial" animate="animate" exit="exit" transition={{ duration: 0.4, ease: EASE }}>
                  {card === 0 && <SceneLight sample={sample} lang={sampleLang} still={still} />}
                  {card === 1 && <SceneTap sample={sample} lang={sampleLang} still={still} />}
                  {card === 2 && <SceneListen sample={sample} lang={sampleLang} still={still} />}
                  {card === 3 && <SceneModels still={still} />}
                </motion.div>
              </AnimatePresence>
            </div>

            <div className="guide-text">
              <AnimatePresence mode="wait" custom={dir} initial={false}>
                <motion.div key={card} custom={dir} variants={slide} initial="initial" animate="animate" exit="exit" transition={{ duration: 0.4, ease: EASE, delay: 0.04 }}>
                  <h2 className="display">{c.title}</h2>
                  <p>{c.body}</p>
                  {c.note && (
                    <p className="note">
                      {c.note}{" "}
                      <button className="guide-more" onClick={seeModels}>
                        {t("Voir les modèles", "See the models")} <Icon name="forward" size={12} stroke={2} />
                      </button>
                    </p>
                  )}
                </motion.div>
              </AnimatePresence>
            </div>

            <div className="guide-foot">
              <div className="guide-dots" role="tablist" aria-label={t("Cartes du guide", "Guide cards")}>
                {list.map((x, i) => (
                  <button key={i} role="tab" aria-selected={i === card} aria-label={x.title} className={i === card ? "on" : ""} onClick={() => goTo(i)}>
                    {i === card && <motion.i layoutId="guide-dot" transition={{ type: "spring", stiffness: 480, damping: 36 }} />}
                  </button>
                ))}
              </div>
              {card > 0 ? (
                <button className="btn sm ghost" onClick={() => goTo(card - 1)}>
                  {t("Retour", "Back")}
                </button>
              ) : (
                <button className="btn sm ghost" onClick={close}>
                  {t("Passer", "Skip")}
                </button>
              )}
              <button className="btn sm primary glow" onClick={next} autoFocus>
                {last ? (
                  <>
                    {t("C'est parti", "Let's go")} <Icon name="check" size={14} stroke={2.2} />
                  </>
                ) : (
                  <>
                    {t("Suivant", "Next")} <Icon name="forward" size={14} stroke={2} />
                  </>
                )}
              </button>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
