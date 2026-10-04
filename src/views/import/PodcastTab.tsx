import { AnimatePresence, motion } from "motion/react";
import { useState } from "react";
import { Icon, type IconName } from "../../components/Icon";
import { Orb, Segmented, Switch } from "../../components/ui";
import { LEVELS, openSource, useLevel } from "../../lib/discover";
import { t } from "../../lib/i18n";
import { useImports } from "../../lib/imports";
import { inLang, theLang } from "../../lib/langs";
import { useApp } from "../../lib/store";
import type { LangCode, PodcastFormat, PodcastRequest } from "../../lib/types";
import { GeminiKey } from "../PodcastSection";

/* Importer › Podcast : un sujet, une forme, un niveau, une durée. Gemini écrit
   le podcast dans la langue étudiée et le dit ; la file des leçons en préparation
   en fait une leçon pendant qu'on continue d'explorer. */

const MINUTES = [3, 5, 10, 15];

const NOTEBOOK_URL = "https://notebooklm.google.com/";

const formats = (): { id: PodcastFormat; icon: IconName; label: string; hint: string }[] => [
  { id: "talk", icon: "chat", label: t("Conversation", "Conversation"), hint: t("Deux animateurs, comme un aperçu audio", "Two hosts, like an Audio Overview") },
  { id: "story", icon: "book", label: t("Histoire", "Story"), hint: t("Un conteur, une seule voix", "One storyteller, a single voice") },
  { id: "debate", icon: "debate", label: t("Débat", "Debate"), hint: t("Deux avis qui s'opposent", "Two opposing views") },
];

/** Idées de sujets, dans la langue de l'interface : Gemini écrit de toute façon dans la langue étudiée. */
const ideas = () =>
  t(
    "La cuisine de tous les jours|Un voyage en train de nuit|Les secrets du sommeil|Une histoire de fantômes|Le premier jour dans un nouveau travail|L'histoire du café|Les animaux de la ville|Un mystère au village|Se faire des amis à l'étranger|Les fêtes de l'année|La ville dans cinquante ans|Une journée sans téléphone|Les petits plaisirs du dimanche|Un objet perdu qui revient|Apprendre à cuisiner un plat de famille|La mer en hiver",
    "Everyday cooking|A night train journey|The secrets of sleep|A ghost story|The first day at a new job|The history of coffee|City animals|A village mystery|Making friends abroad|The holidays of the year|The city in fifty years|A day without a phone|Little Sunday pleasures|A lost object comes back|Learning a family recipe|The sea in winter",
  ).split("|");

function shuffled<T>(xs: T[]): T[] {
  const a = [...xs];
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

/** Ce que demande le niveau, dit à Gemini Notebook (consignes copiées par l'apprenant). */
const notebookLevel = (level: number) =>
  [
    t(
      "Parle lentement et très clairement, en phrases très courtes, avec les mots les plus courants. Répète les mots importants et explique tout mot difficile avec des mots simples.",
      "Speak slowly and very clearly, in very short sentences, with the most common words. Repeat the key words and explain any hard word with simple words.",
    ),
    t(
      "Parle lentement et clairement, en phrases courtes, avec un vocabulaire de tous les jours. Explique brièvement les mots moins courants.",
      "Speak slowly and clearly, in short sentences, with everyday vocabulary. Briefly explain less common words.",
    ),
    t("Parle clairement, à un rythme posé, avec des expressions courantes faciles à comprendre.", "Speak clearly, at a relaxed pace, with common expressions that are easy to understand."),
    t("Parle naturellement, avec quelques expressions idiomatiques, sans excès.", "Speak naturally, with some idioms, in moderation."),
    t("Parle comme entre locuteurs natifs, avec un vocabulaire riche et précis.", "Speak as native speakers do, with rich and precise vocabulary."),
  ][Math.min(5, Math.max(1, level)) - 1];

export function usePodcastForm(lang: LangCode) {
  const settings = useApp((s) => s.settings);
  const setSetting = useApp((s) => s.setSetting);
  const learner = useLevel(lang);
  const [topic, setTopic] = useState("");
  const [details, setDetails] = useState("");
  // null : le niveau de l'apprenant (choisi ou estimé)
  const [chosenLevel, setLevel] = useState<number | null>(null);
  const [pool, setPool] = useState(() => shuffled(ideas()));
  const [offset, setOffset] = useState(0);

  const format = (["talk", "story", "debate"].includes(settings.podcast_format) ? settings.podcast_format : "talk") as PodcastFormat;
  const minutes = MINUTES.includes(Number(settings.podcast_minutes)) ? Number(settings.podcast_minutes) : 5;
  const useWords = settings.podcast_words !== "0";
  const level = chosenLevel ?? (learner.level || 2);

  const request = (): PodcastRequest => ({ topic: topic.trim(), level, minutes, format, details: details.trim(), use_words: useWords });

  return {
    lang,
    topic,
    setTopic,
    details,
    setDetails,
    level,
    /** le niveau suit celui de l'apprenant */
    ownLevel: chosenLevel === null || chosenLevel === learner.level,
    learnerLevel: learner.level,
    setLevel,
    format,
    setFormat: (f: PodcastFormat) => setSetting("podcast_format", f),
    minutes,
    setMinutes: (m: number) => setSetting("podcast_minutes", String(m)),
    useWords,
    setUseWords: (on: boolean) => setSetting("podcast_words", on ? "1" : "0"),
    hasKey: !!(settings.gemini_key ?? "").trim(),
    ready: !!(settings.gemini_key ?? "").trim() && topic.trim().length >= 2,
    ideas: pool.slice(offset, offset + 5),
    moreIdeas: () => {
      if (offset + 10 <= pool.length) setOffset(offset + 5);
      else {
        setPool(shuffled(ideas()));
        setOffset(0);
      }
    },
    request,
    /** confie le podcast à la file des leçons en préparation */
    create() {
      const r = request();
      useImports.getState().enqueue({ key: `podcast:${Date.now()}`, lang, title: r.topic, image: "", podcast: r });
      setTopic("");
      setDetails("");
    },
    reset() {
      setTopic("");
      setDetails("");
      setLevel(null);
    },
  };
}

export type PodcastForm = ReturnType<typeof usePodcastForm>;

const enter = { initial: { opacity: 0, y: 8 }, animate: { opacity: 1, y: 0 }, exit: { opacity: 0, y: -4 }, transition: { type: "spring" as const, stiffness: 320, damping: 30 } };

export function PodcastTab({ form }: { form: PodcastForm }) {
  return (
    <AnimatePresence mode="wait" initial={false}>
      {form.hasKey ? (
        <motion.div key="form" className="pod-form" {...enter}>
          <PodcastFields form={form} />
        </motion.div>
      ) : (
        <motion.div key="key" className="pod-form" {...enter}>
          <PodcastIntro lang={form.lang} />
          <GeminiKey compact />
          <NotebookRoute form={form} />
        </motion.div>
      )}
    </AnimatePresence>
  );
}

/** Avant la clé : ce que fait l'onglet, en trois temps. */
function PodcastIntro({ lang }: { lang: LangCode }) {
  return (
    <div className="pod-intro">
      <div className="pod-intro-art" aria-hidden="true">
        <Orb size={44} />
        <span className="pod-wave" />
        <span className="pod-wave late" />
      </div>
      <div className="pod-intro-text">
        <strong>{t("Des podcasts écrits pour vous", "Podcasts written for you")}</strong>
        <span>
          {t(
            `Un sujet, votre niveau, une durée : Gemini, l'IA de Google, écrit un podcast ${inLang(lang)} et le fait dire par deux voix. Il devient une leçon complète, avec sa lanterne.`,
            `A topic, your level, a length: Gemini, Google's AI, writes a podcast ${inLang(lang)} and has two voices perform it. It becomes a full lesson, with its lantern.`,
          )}
        </span>
        <ol className="pod-steps">
          <li>{t("Créez une clé gratuite sur Google AI Studio.", "Create a free key on Google AI Studio.")}</li>
          <li>{t("Collez-la ci-dessous : Lumen la vérifie et la garde sur ce Mac.", "Paste it below: Lumen checks it and keeps it on this Mac.")}</li>
          <li>{t("Choisissez un sujet. Le podcast vous attend dans la bibliothèque.", "Choose a topic. The podcast will wait for you in the library.")}</li>
        </ol>
      </div>
    </div>
  );
}

function PodcastFields({ form }: { form: PodcastForm }) {
  return (
    <>
      <div className="field">
        <label htmlFor="pod-topic">{t("De quoi parle le podcast ?", "What is the podcast about?")}</label>
        <textarea
          id="pod-topic"
          className="textarea pod-topic"
          rows={2}
          value={form.topic}
          maxLength={400}
          onChange={(e) => form.setTopic(e.target.value)}
          placeholder={t("Ex. : la cuisine de Naples, une enquête dans un train de nuit, les bienfaits de la sieste…", "E.g. Naples cooking, an investigation on a night train, the benefits of a nap…")}
          autoFocus
        />
      </div>
      <div className="pod-ideas">
        {form.ideas.map((i) => (
          <button key={i} className={`idea ${form.topic === i ? "on" : ""}`} onClick={() => form.setTopic(i)}>
            {i}
          </button>
        ))}
        <button className="idea pod-more" onClick={form.moreIdeas}>
          <Icon name="refresh" size={13} /> {t("D'autres idées", "More ideas")}
        </button>
      </div>

      <div className="pod-formats" role="radiogroup" aria-label={t("Forme du podcast", "Podcast format")}>
        {formats().map((f) => (
          <button key={f.id} type="button" role="radio" aria-checked={form.format === f.id} className={`pod-format ${form.format === f.id ? "on" : ""}`} onClick={() => form.setFormat(f.id)}>
            <Icon name={f.icon} size={20} />
            <strong>{f.label}</strong>
            <span>{f.hint}</span>
          </button>
        ))}
      </div>

      <div className="pod-row">
        <div className="field">
          <label>{t("Niveau", "Level")}</label>
          <Segmented
            id="pod-level"
            label={t("Niveau", "Level")}
            value={String(form.level)}
            options={LEVELS.map((l, i) => ({ value: String(i + 1), label: l }))}
            onChange={(v) => form.setLevel(Number(v))}
          />
        </div>
        <div className="field">
          <label>{t("Durée", "Length")}</label>
          <Segmented id="pod-minutes" label={t("Durée", "Length")} value={String(form.minutes)} options={MINUTES.map((m) => ({ value: String(m), label: `${m} min` }))} onChange={(v) => form.setMinutes(Number(v))} />
        </div>
      </div>
      <span className="import-hint pod-level-hint">
        {form.ownLevel
          ? t(`${LEVELS[form.level - 1]} : votre niveau ${inLang(form.lang)}, d'après vos mots connus.`, `${LEVELS[form.level - 1]}: your level ${inLang(form.lang)}, from the words you know.`)
          : t(`Votre niveau est ${LEVELS[(form.learnerLevel || 2) - 1]} : ce podcast sera ${form.level > (form.learnerLevel || 2) ? "un pas plus loin" : "plus facile"}.`, `Your level is ${LEVELS[(form.learnerLevel || 2) - 1]}: this podcast will be ${form.level > (form.learnerLevel || 2) ? "a step further" : "easier"}.`)}
      </span>

      <div className="pod-switch">
        <div className="grow">
          <strong>{t("Faire revenir mes mots en apprentissage", "Bring back the words I'm learning")}</strong>
          <span>{t("Jusqu'à 25 mots que vous étudiez, glissés naturellement dans le podcast.", "Up to 25 words you're studying, worked naturally into the podcast.")}</span>
        </div>
        <Switch on={form.useWords} onChange={form.setUseWords} label={t("Faire revenir mes mots en apprentissage", "Bring back the words I'm learning")} />
      </div>

      <div className="field">
        <label htmlFor="pod-details">{t("Précisions (facultatif)", "Wishes (optional)")}</label>
        <input
          id="pod-details"
          className="input"
          value={form.details}
          maxLength={300}
          onChange={(e) => form.setDetails(e.target.value)}
          placeholder={t("Ex. : avec humour, au passé, du vocabulaire de cuisine…", "E.g. with humour, in the past tense, some cooking vocabulary…")}
        />
      </div>

      <p className="link-note">
        <Icon name="sparkle" size={15} />
        <span>
          {t(
            "Gemini écrit et dit le podcast en ligne, en quelques minutes ; Whisper cale ensuite la lanterne sur ce Mac. Seuls le sujet, vos précisions et les mots à faire revenir sont envoyés à Google.",
            "Gemini writes and voices the podcast online, in a few minutes; Whisper then sets the lantern on this Mac. Only the topic, your wishes and the words to bring back are sent to Google.",
          )}
        </span>
      </p>

      <NotebookRoute form={form} />
    </>
  );
}

/**
 * Le chemin par Gemini Notebook : il ne s'ouvre pas aux autres applications,
 * mais Lumen peut préparer les consignes de l'aperçu audio ; le fichier
 * téléchargé, déposé dans Lumen, est transcrit comme tout son.
 */
function NotebookRoute({ form }: { form: PodcastForm }) {
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState(false);
  const r = form.request();
  const shape = { talk: t("une conversation entre deux animateurs", "a conversation between two hosts"), story: t("une histoire racontée par une seule voix", "a story told by a single voice"), debate: t("un débat amical entre deux avis opposés", "a friendly debate between two opposing views") }[r.format];
  const lines = [
    t(`Crée un aperçu audio entièrement ${inLang(form.lang)}, pour un apprenant de niveau ${LEVELS[r.level - 1]} (CECR).`, `Create an Audio Overview entirely ${inLang(form.lang)}, for a learner at CEFR level ${LEVELS[r.level - 1]}.`),
    r.topic ? t(`Sujet : ${r.topic}.`, `Topic: ${r.topic}.`) : "",
    t(`Forme : ${shape}.`, `Format: ${shape}.`),
    t(`Durée : environ ${r.minutes} minutes.`, `Length: about ${r.minutes} minutes.`),
    notebookLevel(r.level),
    r.details ? t(`Précisions : ${r.details}`, `Wishes: ${r.details}`) : "",
  ].filter(Boolean);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(lines.join("\n"));
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2200);
    } catch {
      useApp.getState().toast(t("La copie n'a pas abouti.", "Copying didn't work."), "error");
    }
  };

  return (
    <div className={`pod-notebook ${open ? "open" : ""}`}>
      <button className="pod-notebook-head" onClick={() => setOpen(!open)} aria-expanded={open}>
        <span>{t("Vous préférez Gemini Notebook ?", "Prefer Gemini Notebook?")}</span>
        <Icon name="chevron" size={14} />
      </button>
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            key="body"
            className="pod-notebook-body"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.28, ease: [0.2, 0.8, 0.2, 1] }}
          >
            <div className="pod-notebook-inner">
              <p>
                {t(
                  "Gemini Notebook ne s'ouvre pas aux autres applications : Lumen ne peut pas le piloter. Mais il peut préparer vos consignes.",
                  "Gemini Notebook isn't open to other apps: Lumen can't drive it. But it can prepare your instructions.",
                )}
              </p>
              <ol className="pod-steps">
                <li>{t("Copiez les consignes (sujet, niveau, forme, durée).", "Copy the instructions (topic, level, format, length).")}</li>
                <li>
                  {t(
                    `Dans Gemini Notebook, ajoutez une source sur le sujet, puis Aperçu audio › Personnaliser : choisissez ${theLang(form.lang)} et collez les consignes.`,
                    `In Gemini Notebook, add a source on the topic, then Audio Overview › Customize: choose ${theLang(form.lang)} and paste the instructions.`,
                  )}
                </li>
                <li>{t("Téléchargez l'aperçu et déposez le fichier dans Lumen : Whisper et Qwen3-ASR le transcrivent en leçon.", "Download the overview and drop the file into Lumen: Whisper and Qwen3-ASR turn it into a lesson.")}</li>
              </ol>
              <div className="pod-notebook-actions">
                <button className="btn sm" onClick={() => void copy()}>
                  <Icon name={copied ? "check" : "copy"} size={14} /> {copied ? t("Consignes copiées", "Instructions copied") : t("Copier les consignes", "Copy the instructions")}
                </button>
                <button className="btn sm ghost" onClick={() => void openSource(NOTEBOOK_URL)}>
                  {t("Ouvrir Gemini Notebook", "Open Gemini Notebook")} <Icon name="external" size={12} stroke={2} />
                </button>
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
