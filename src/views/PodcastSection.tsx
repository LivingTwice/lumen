import { AnimatePresence, motion } from "motion/react";
import { useState } from "react";
import { Icon } from "../components/Icon";
import { Orb } from "../components/ui";
import { api, errorText } from "../lib/api";
import { openSource } from "../lib/discover";
import { t } from "../lib/i18n";
import { useApp } from "../lib/store";
import type { GeminiModels } from "../lib/types";

/** Page de Google AI Studio où l'on crée sa clé (gratuite, avec un compte Google). */
export const GEMINI_KEY_URL = "https://aistudio.google.com/apikey";

const enter = { initial: { opacity: 0, y: 8 }, animate: { opacity: 1, y: 0 }, exit: { opacity: 0, y: 6 }, transition: { type: "spring" as const, stiffness: 320, damping: 30 } };

/** « gemini-3.8-flash-tts » → « Gemini 3.8 Flash TTS ». */
export function modelLabel(id: string): string {
  return id
    .split("-")
    .filter((w) => w !== "preview")
    .map((w) => (w === "tts" ? "TTS" : w.charAt(0).toUpperCase() + w.slice(1)))
    .join(" ");
}

/**
 * Clé Gemini : collée, vérifiée auprès de Google, puis gardée sur ce Mac (jamais
 * dans la sauvegarde). `compact` : version de la feuille d'import.
 */
export function GeminiKey({ compact = false }: { compact?: boolean }) {
  const saved = useApp((s) => s.settings.gemini_key ?? "");
  const setSetting = useApp((s) => s.setSetting);
  const [key, setKey] = useState(saved);
  const [show, setShow] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [models, setModels] = useState<GeminiModels | null>(null);

  const check = async () => {
    const k = key.trim();
    if (!k || busy) return;
    setBusy(true);
    setError(null);
    setModels(null);
    try {
      const m = await api().geminiCheck(k);
      setModels(m);
      setSetting("gemini_key", k);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className={`gemini-key ${compact ? "compact" : ""}`}>
      <div className="key-row">
        <div className="key-field">
          <input
            className="input"
            type={show ? "text" : "password"}
            value={key}
            placeholder={t("Collez votre clé Gemini ici", "Paste your Gemini key here")}
            spellCheck={false}
            autoComplete="off"
            autoCorrect="off"
            onChange={(e) => {
              setKey(e.target.value);
              setModels(null);
              setError(null);
            }}
            onKeyDown={(e) => e.key === "Enter" && void check()}
            aria-label={t("Clé Gemini", "Gemini key")}
          />
          <button className={`icon-btn ${show ? "on" : ""}`} onClick={() => setShow(!show)} aria-label={show ? t("Masquer la clé", "Hide the key") : t("Afficher la clé", "Show the key")}>
            <Icon name="eye" size={15} />
          </button>
        </div>
        <button className="btn sm primary" disabled={!key.trim() || busy} onClick={() => void check()}>
          {busy ? t("Vérification…", "Checking…") : key.trim() && key.trim() === saved && !error ? t("Vérifier à nouveau", "Check again") : t("Vérifier et garder", "Check and keep")}
        </button>
      </div>
      <AnimatePresence initial={false}>
        {busy && (
          <motion.div key="busy" className="key-note" {...enter}>
            <Orb size={16} />
            <span>{t("Lumen demande à Google ce que cette clé permet…", "Lumen is asking Google what this key allows…")}</span>
          </motion.div>
        )}
        {error && (
          <motion.div key="err" className="key-note error" {...enter}>
            <Icon name="ban" size={15} />
            <span>{error}</span>
          </motion.div>
        )}
        {models && (
          <motion.div key="ok" className="key-note ok" {...enter}>
            <span className="key-check">
              <Icon name="check" size={13} stroke={2.4} />
            </span>
            <span>
              {t(
                `Clé prête. ${modelLabel(models.text[0])} écrira vos podcasts, ${modelLabel(models.tts[0])} leur donnera voix.`,
                `Key ready. ${modelLabel(models.text[0])} will write your podcasts, ${modelLabel(models.tts[0])} will voice them.`,
              )}
            </span>
          </motion.div>
        )}
      </AnimatePresence>
      <span className="import-hint">
        {t("Pas encore de clé ? Elle est gratuite avec un compte Google :", "No key yet? It's free with a Google account:")}{" "}
        <a
          className="link"
          href={GEMINI_KEY_URL}
          onClick={(e) => {
            e.preventDefault();
            void openSource(GEMINI_KEY_URL);
          }}
        >
          aistudio.google.com/apikey <Icon name="external" size={12} stroke={2} />
        </a>
        {t(", bouton « Create API key ». Elle reste sur ce Mac.", ", “Create API key” button. It stays on this Mac.")}
      </span>
    </div>
  );
}

/** Réglages › Podcasts : la clé Gemini, ce qui part en ligne, Gemini Notebook. */
export function PodcastSection() {
  const saved = useApp((s) => s.settings.gemini_key ?? "");
  const setSetting = useApp((s) => s.setSetting);
  const openImport = useApp((s) => s.openImport);
  return (
    // le titre et la présentation sont dans l'en-tête de la page des Réglages
    <section className="set-section">
      <div className="set-card">
        <div className="set-row">
          <div className="grow">
            <strong>{t("Clé Gemini", "Gemini key")}</strong>
            <span>
              {t(
                "Gemini est l'IA en ligne de Google. Avec votre clé, il écrit le podcast à votre niveau puis le dit à une ou deux voix ; Whisper cale ensuite la lanterne sur ce Mac.",
                "Gemini is Google's online AI. With your key, it writes the podcast at your level, then voices it with one or two voices; Whisper then sets the lantern on this Mac.",
              )}
            </span>
          </div>
        </div>
        <div className="set-row">
          <GeminiKey />
        </div>
        {saved && (
          <div className="set-row">
            <div className="grow">
              <strong>{t("Créer un podcast", "Create a podcast")}</strong>
              <span>{t("Importer › Podcast : un sujet, un niveau, une durée.", "Import › Podcast: a topic, a level, a length.")}</span>
            </div>
            <button className="btn sm" onClick={() => openImport(undefined, "podcast")}>
              <Icon name="podcast" size={14} /> {t("Commencer", "Start")}
            </button>
            <button className="btn sm ghost" onClick={() => setSetting("gemini_key", "")}>
              {t("Retirer la clé", "Remove the key")}
            </button>
          </div>
        )}
      </div>

      <div className="set-card">
        <div className="set-row">
          <div className="grow">
            <strong>{t("Ce qui part en ligne", "What goes online")}</strong>
            <span>
              {t(
                "Seulement le sujet, vos précisions et, si vous le demandez, jusqu'à 25 mots que vous étudiez. Vos leçons, votre vocabulaire et votre progression restent sur ce Mac. Avec une clé gratuite, Google peut se servir de ces demandes pour améliorer ses modèles ; le quota gratuit se renouvelle chaque jour.",
                "Only the topic, your wishes and, if you ask for it, up to 25 words you're studying. Your lessons, vocabulary and progress stay on this Mac. With a free key, Google may use these requests to improve its models; the free quota renews every day.",
              )}
            </span>
          </div>
        </div>
        <div className="set-row">
          <div className="grow">
            <strong>{t("Et Gemini Notebook ?", "What about Gemini Notebook?")}</strong>
            <span>
              {t(
                "Gemini Notebook (autrefois NotebookLM) ne s'ouvre pas aux autres applications : Lumen ne peut pas le piloter. Il passe donc par Gemini lui-même, qui a les mêmes voix et tient mieux le niveau. Vous pouvez aussi créer un aperçu audio dans Gemini Notebook et déposer le fichier dans Lumen : il sera transcrit.",
                "Gemini Notebook (formerly NotebookLM) isn't open to other apps: Lumen can't drive it. So Lumen goes through Gemini itself, which has the same voices and keeps to your level better. You can also make an Audio Overview in Gemini Notebook and drop the file into Lumen: it will be transcribed.",
              )}
            </span>
          </div>
        </div>
      </div>
    </section>
  );
}
