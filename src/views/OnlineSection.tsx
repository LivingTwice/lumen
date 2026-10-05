import { AnimatePresence, motion } from "motion/react";
import { useState } from "react";
import { Icon, type IconName } from "../components/Icon";
import { Orb, Switch } from "../components/ui";
import { api, errorText } from "../lib/api";
import { openSource } from "../lib/discover";
import { t } from "../lib/i18n";
import { latency, onlineProviders, useOnline, type OnlineProvider } from "../lib/online";
import { useApp } from "../lib/store";
import type { OnlineCheck } from "../lib/types";

/* Réglages › IA : où travaille l'IA (sur ce Mac ou en ligne), et l'IA en ligne :
   fournisseur, clé vérifiée, modèle, ce qu'elle prend en charge. */

const TILE = { type: "spring", stiffness: 520, damping: 40 } as const;
const enter = { initial: { opacity: 0, y: 8 }, animate: { opacity: 1, y: 0 }, exit: { opacity: 0, y: 6 }, transition: { type: "spring" as const, stiffness: 320, damping: 30 } };

/** Sur ce Mac (par défaut) ou en ligne : deux tuiles comme celles de la puissance. */
export function EnginePicker() {
  const on = useApp((s) => s.settings.online_on === "1");
  const setSetting = useApp((s) => s.setSetting);
  const pick = (online: boolean) => {
    if (online === on) return;
    void setSetting("online_on", online ? "1" : "");
    // retour sur ce Mac : le modèle se charge d'avance
    if (!online) void api().aiWarmup().catch(() => {});
  };
  const opts: { online: boolean; icon: IconName; name: string; desc: string; tag: string }[] = [
    {
      online: false,
      icon: "laptop",
      name: t("Sur ce Mac", "On this Mac"),
      desc: t(
        "Hors ligne, gratuit et privé : rien ne quitte votre Mac. Qwen3.5, à la puissance choisie plus bas.",
        "Offline, free and private: nothing leaves your Mac. Qwen3.5, at the power chosen below.",
      ),
      tag: t("Par défaut", "Default"),
    },
    {
      online: true,
      icon: "cloud",
      name: t("En ligne", "Online"),
      desc: t(
        "Des modèles bien plus grands, plus fins, rapides même sur un petit Mac. Avec votre propre clé, chez le fournisseur de votre choix.",
        "Much bigger, finer models, fast even on a small Mac. With your own key, at the provider of your choice.",
      ),
      tag: t("Facultatif", "Optional"),
    },
  ];
  return (
    <div className="profile-grid engine-grid" role="radiogroup" aria-label={t("Où travaille l'IA", "Where the AI works")}>
      {opts.map((o) => (
        <button key={o.icon} role="radio" aria-checked={o.online === on} className={`profile engine ${o.online === on ? "on" : ""}`} onClick={() => pick(o.online)}>
          <span className="engine-icon">
            <Icon name={o.icon} size={18} />
          </span>
          <strong>{o.name}</strong>
          <span>{o.desc}</span>
          <span className="size">{o.tag}</span>
        </button>
      ))}
    </div>
  );
}

/** Liste des modèles gardée à la dernière vérification (réglage `online_models_<id>`). */
function savedModels(raw: string | undefined): string[] {
  try {
    const v = JSON.parse(raw || "[]");
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

const hostOf = (url: string) => url.replace(/^https?:\/\//, "").replace(/\/.*$/, "");

/** Clé (et adresse, pour un serveur choisi) : collée, vérifiée, gardée sur ce Mac ; puis le modèle. */
function OnlineKey({ provider: p }: { provider: OnlineProvider }) {
  const settings = useApp((s) => s.settings);
  const setSetting = useApp((s) => s.setSetting);
  const custom = p.id === "custom";
  const savedKey = settings[p.keySetting] ?? "";
  const savedUrl = settings.online_url ?? "";
  const modelKey = `online_model_${p.id}`;
  const listKey = `online_models_${p.id}`;
  const model = settings[modelKey] ?? "";
  const models = savedModels(settings[listKey]);
  const [key, setKey] = useState(savedKey);
  const [url, setUrl] = useState(savedUrl);
  const [typed, setTyped] = useState(model);
  const [show, setShow] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<OnlineCheck | null>(null);

  const saved = custom ? !!savedUrl.trim() : !!savedKey.trim();
  const dirty = key.trim() !== savedKey.trim() || (custom && url.trim() !== savedUrl.trim());
  const can = custom ? !!url.trim() : !!key.trim();

  const check = async (wanted = model) => {
    if (busy || !can) return;
    setBusy(true);
    setError(null);
    setDone(null);
    try {
      const r = await api().onlineCheck(p.id, key.trim(), custom ? url.trim() : undefined, wanted || undefined);
      void setSetting(p.keySetting, key.trim());
      if (custom) void setSetting("online_url", url.trim());
      void setSetting(modelKey, r.model);
      void setSetting(listKey, JSON.stringify(r.models));
      setTyped(r.model);
      setDone(r);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  // un autre modèle : il sert aussitôt, et une petite question dit s'il répond (et en combien de temps)
  const choose = (m: string) => {
    if (!m || m === model) return;
    void setSetting(modelKey, m);
    void check(m);
  };

  const options = model && !models.includes(model) ? [model, ...models] : models;

  return (
    <div className="gemini-key online-key">
      {custom && (
        <div className="online-url">
          <input
            className="input"
            value={url}
            placeholder={t("Adresse du serveur, par exemple http://localhost:11434/v1", "Server address, for example http://localhost:11434/v1")}
            aria-label={t("Adresse du serveur", "Server address")}
            spellCheck={false}
            autoComplete="off"
            autoCorrect="off"
            onChange={(e) => {
              setUrl(e.target.value);
              setDone(null);
              setError(null);
            }}
            onKeyDown={(e) => e.key === "Enter" && void check()}
          />
        </div>
      )}
      <div className="key-row">
        <div className="key-field">
          <input
            className="input"
            type={show ? "text" : "password"}
            value={key}
            placeholder={custom ? t("Clé, si le serveur en demande une", "Key, if the server asks for one") : t(`Collez votre clé ${p.name} ici`, `Paste your ${p.name} key here`)}
            spellCheck={false}
            autoComplete="off"
            autoCorrect="off"
            onChange={(e) => {
              setKey(e.target.value);
              setDone(null);
              setError(null);
            }}
            onKeyDown={(e) => e.key === "Enter" && void check()}
            aria-label={t(`Clé ${p.name}`, `${p.name} key`)}
          />
          <button className={`icon-btn ${show ? "on" : ""}`} onClick={() => setShow(!show)} aria-label={show ? t("Masquer la clé", "Hide the key") : t("Afficher la clé", "Show the key")}>
            <Icon name="eye" size={15} />
          </button>
        </div>
        <button className="btn sm primary" disabled={!can || busy} onClick={() => void check()}>
          {busy ? t("Vérification…", "Checking…") : saved && !dirty && !error ? t("Vérifier à nouveau", "Check again") : t("Vérifier et garder", "Check and keep")}
        </button>
      </div>

      <AnimatePresence initial={false}>
        {busy && (
          <motion.div key="busy" className="key-note" {...enter}>
            <Orb size={16} />
            <span>{custom ? t("Lumen pose une petite question au serveur…", "Lumen is asking the server a little question…") : t(`Lumen pose une petite question à ${p.name}…`, `Lumen is asking ${p.name} a little question…`)}</span>
          </motion.div>
        )}
        {error && (
          <motion.div key="err" className="key-note error" {...enter}>
            <Icon name="ban" size={15} />
            <span>{error}</span>
          </motion.div>
        )}
        {done && (
          <motion.div key="ok" className="key-note ok" {...enter}>
            <span className="key-check">
              <Icon name="check" size={13} stroke={2.4} />
            </span>
            <span>{t(`Prêt. ${done.model} a répondu en ${latency(done.ms)}.`, `Ready. ${done.model} answered in ${latency(done.ms)}.`)}</span>
          </motion.div>
        )}
      </AnimatePresence>

      {saved && (options.length > 0 || custom) && (
        <div className="online-model">
          <span>{t("Modèle", "Model")}</span>
          {options.length > 0 ? (
            <select className="select" value={model} onChange={(e) => choose(e.target.value)} aria-label={t("Modèle", "Model")} disabled={busy}>
              {options.map((m) => (
                <option key={m} value={m}>
                  {m}
                </option>
              ))}
            </select>
          ) : (
            <input
              className="input"
              value={typed}
              placeholder={t("Nom du modèle, par exemple qwen3:8b", "Model name, for example qwen3:8b")}
              spellCheck={false}
              autoComplete="off"
              onChange={(e) => setTyped(e.target.value)}
              onBlur={() => typed.trim() && choose(typed.trim())}
              onKeyDown={(e) => e.key === "Enter" && typed.trim() && choose(typed.trim())}
            />
          )}
        </div>
      )}

      <span className="import-hint">
        {p.keyUrl ? (
          <>
            {saved ? t("Votre compte, son crédit et vos clés :", "Your account, its credit and your keys:") : t("Pas encore de clé ? Créez-la sur", "No key yet? Create one at")}{" "}
            <a
              className="link"
              href={p.keyUrl}
              onClick={(e) => {
                e.preventDefault();
                void openSource(p.keyUrl);
              }}
            >
              {hostOf(p.keyUrl)} <Icon name="external" size={12} stroke={2} />
            </a>
            {p.id === "gemini"
              ? t(". C'est la même clé que pour les podcasts ; elle reste sur ce Mac.", ". It's the same key as for podcasts; it stays on this Mac.")
              : t(". Elle reste sur ce Mac, jamais dans la sauvegarde.", ". It stays on this Mac, never in the backup.")}
          </>
        ) : (
          t("Ollama : http://localhost:11434/v1 · LM Studio : http://localhost:1234/v1", "Ollama: http://localhost:11434/v1 · LM Studio: http://localhost:1234/v1")
        )}
      </span>
    </div>
  );
}

/** L'IA en ligne : fournisseur, clé et modèle, ce qu'elle prend en charge, ce qui part en ligne. */
export function OnlineSection() {
  const settings = useApp((s) => s.settings);
  const setSetting = useApp((s) => s.setSetting);
  const st = useOnline();
  const p = st.provider;
  const saved = p.id === "custom" ? !!settings.online_url?.trim() : !!settings[p.keySetting]?.trim();

  return (
    <>
      <section className="set-section">
        <h3>{t("Fournisseur", "Provider")}</h3>
        <p>{t("Là où partent vos demandes. Chacun a sa clé, gardée sur ce Mac.", "Where your requests go. Each one has its own key, kept on this Mac.")}</p>
        <div className="places online-places" role="radiogroup" aria-label={t("Fournisseur", "Provider")}>
          {onlineProviders().map((x) => {
            const on = x.id === p.id;
            const has = x.id === "custom" ? !!settings.online_url?.trim() : !!settings[x.keySetting]?.trim();
            return (
              <button key={x.id} role="radio" aria-checked={on} className={`place ${on ? "on" : ""}`} onClick={() => void setSetting("online_provider", x.id)} title={x.name}>
                {on && <motion.span layoutId="online-provider" className="look-ring" transition={TILE} />}
                <span className={`place-glyph ${x.mark ? "mono" : ""}`}>{x.mark || <Icon name="link" size={18} />}</span>
                <strong>{x.name}</strong>
                <span className="place-note">{has ? (x.id === "custom" ? t("Adresse enregistrée", "Address saved") : t("Clé enregistrée", "Key saved")) : x.note}</span>
              </button>
            );
          })}
        </div>
      </section>

      <section className="set-section">
        <div className="set-card">
          <div className="set-row">
            <div className="grow">
              <strong>{p.name}</strong>
              <span>{p.about}</span>
            </div>
            {saved && p.id !== "gemini" && (
              <button
                className="btn sm ghost"
                onClick={() => {
                  void setSetting(p.keySetting, "");
                  if (p.id === "custom") void setSetting("online_url", "");
                }}
              >
                {p.id === "custom" ? t("Oublier ce serveur", "Forget this server") : t("Retirer la clé", "Remove the key")}
              </button>
            )}
          </div>
          <div className="set-row">
            {/* une clé par fournisseur : le champ repart de la sienne */}
            <OnlineKey key={p.id} provider={p} />
          </div>
          <div className="set-row">
            <div className="grow">
              <strong>{t("Sens des mots et traduction des phrases", "Word meanings and sentence translation")}</strong>
              <span>
                {t(
                  "Au toucher d'un mot, dans les leçons et les sous-titres. Sans connexion, Lumen traduit avec le modèle de ce Mac s'il est installé.",
                  "When you tap a word, in lessons and subtitles. Without a connection, Lumen translates with this Mac's model if it's installed.",
                )}
              </span>
            </div>
            <Switch on={st.words} onChange={(v) => void setSetting("online_words", v ? "1" : "0")} label={t("Traduction en ligne", "Online translation")} />
          </div>
          <div className="set-row">
            <div className="grow">
              <strong>{t("Chat et Simplifier", "Chat and Simplify")}</strong>
              <span>
                {t(
                  "Un grand modèle lit des leçons plus longues en entier et explique avec plus de finesse.",
                  "A large model reads longer lessons in full and explains with more nuance.",
                )}
              </span>
            </div>
            <Switch on={st.chat} onChange={(v) => void setSetting("online_chat", v ? "1" : "0")} label={t("Chat en ligne", "Online chat")} />
          </div>
        </div>
      </section>

      <section className="set-section">
        <div className="set-card">
          <div className="set-row">
            <div className="grow">
              <strong>{t("Ce qui part en ligne", "What goes online")}</strong>
              <span>
                {t(
                  "Le mot touché et sa phrase, le texte à simplifier, vos questions au chat avec la leçon jointe. Votre profil, votre vocabulaire et votre progression restent sur ce Mac. Chaque traduction est gardée : un mot déjà traduit ne repart pas. La transcription et la voix restent toujours sur ce Mac.",
                  "The tapped word and its sentence, the text to simplify, your chat questions with the attached lesson. Your profile, vocabulary and progress stay on this Mac. Every translation is kept: a word already translated isn't sent again. Transcription and the voice always stay on this Mac.",
                )}
              </span>
            </div>
          </div>
          <div className="set-row">
            <div className="grow">
              <strong>{t("Et mon abonnement ChatGPT, Claude ou Gemini ?", "What about my ChatGPT, Claude or Gemini subscription?")}</strong>
              <span>
                {t(
                  "Un abonnement ne sert que dans les applications de son éditeur : Anthropic et Google interdisent d'utiliser Claude Pro ou Google AI Pro ailleurs, et suspendent les comptes qui le font. Il faut donc une clé, payée à l'usage, ou la clé gratuite de Gemini.",
                  "A subscription only works in its maker's own apps: Anthropic and Google forbid using Claude Pro or Google AI Pro anywhere else, and suspend accounts that do. So you need a key, paid as you go, or Gemini's free key.",
                )}
              </span>
            </div>
          </div>
        </div>
      </section>
    </>
  );
}
