import { AnimatePresence, motion } from "motion/react";
import { useState } from "react";
import { Icon } from "../components/Icon";
import { Orb, Switch } from "../components/ui";
import { isTauri } from "../lib/api";
import { t } from "../lib/i18n";
import { langInfo, langLower } from "../lib/langs";
import { LINGQ_KEY_URL, plural, stageText, summary, useLingq } from "../lib/lingq";
import { formatNumber, useApp } from "../lib/store";

const enter = { initial: { opacity: 0, y: 10 }, animate: { opacity: 1, y: 0 }, exit: { opacity: 0, y: 6 }, transition: { type: "spring" as const, stiffness: 320, damping: 30 } };

function openKeyPage() {
  if (isTauri) void import("@tauri-apps/plugin-opener").then((o) => o.openUrl(LINGQ_KEY_URL));
  else window.open(LINGQ_KEY_URL, "_blank", "noopener");
}

/** Réglages › LingQ : clé API, analyse du compte, import de tout le parcours. */
export function LingqSection() {
  const key = useApp((s) => s.settings.lingq_key ?? "");
  const setSetting = useApp((s) => s.setSetting);
  const lq = useLingq();
  const [show, setShow] = useState(false);
  const busy = lq.phase === "scanning" || lq.phase === "importing";
  const pct = lq.total ? Math.min(100, (lq.done / lq.total) * 100) : 0;
  const chosen = lq.account.filter((a) => lq.chosen.includes(a.lang));
  const lessonCount = chosen.reduce((n, a) => n + a.lessons, 0);
  const canStart = chosen.length > 0 && (lq.vocab || lq.lessons);

  return (
    <section className="set-section">
      <h2>LingQ</h2>
      <p>
        {t(
          "Retrouvez dans Lumen tout votre parcours LingQ : mots connus et ignorés, LingQ avec leurs traductions et leurs notes, et les leçons de tous vos cours avec leur audio.",
          "Bring your whole LingQ journey into Lumen: known and ignored words, LingQs with their translations and notes, and the lessons of all your courses with their audio.",
        )}
      </p>

      <div className="set-card">
        <div className="set-row">
          <div className="grow">
            <strong>{t("Clé API LingQ", "LingQ API key")}</strong>
            <span>
              {t("Copiez-la depuis", "Copy it from")}{" "}
              <a
                className="link"
                href={LINGQ_KEY_URL}
                onClick={(e) => {
                  e.preventDefault();
                  openKeyPage();
                }}
              >
                lingq.com/accounts/apikey <Icon name="external" size={12} stroke={2} />
              </a>
              {t(", connecté à votre compte. Elle reste sur ce Mac.", ", signed in to your account. It stays on this Mac.")}
            </span>
          </div>
        </div>
        <div className="set-row">
          <div className="key-field">
            <input
              className="input"
              type={show ? "text" : "password"}
              value={key}
              placeholder={t("Collez votre clé ici", "Paste your key here")}
              spellCheck={false}
              autoComplete="off"
              autoCorrect="off"
              onChange={(e) => setSetting("lingq_key", e.target.value.trim())}
              onKeyDown={(e) => e.key === "Enter" && key && !busy && lq.scan(key)}
              aria-label={t("Clé API LingQ", "LingQ API key")}
            />
            <button className={`icon-btn ${show ? "on" : ""}`} onClick={() => setShow(!show)} aria-label={show ? t("Masquer la clé", "Hide the key") : t("Afficher la clé", "Show the key")}>
              <Icon name="eye" size={15} />
            </button>
          </div>
          <button className="btn sm primary" disabled={!key || busy} onClick={() => lq.scan(key)}>
            {lq.phase === "scanning" ? t("Analyse…", "Scanning…") : lq.account.length ? t("Analyser à nouveau", "Scan again") : t("Analyser mon compte", "Scan my account")}
          </button>
        </div>
        <AnimatePresence initial={false}>
          {lq.phase === "scanning" && (
            <motion.div key="scan" className="set-row lingq-wait" {...enter}>
              <Orb size={18} />
              <span>{t("Lecture de votre compte LingQ, langue par langue…", "Reading your LingQ account, language by language…")}</span>
            </motion.div>
          )}
          {lq.error && (
            <motion.div key="err" className="set-row lingq-error" {...enter}>
              {lq.error}
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      <AnimatePresence>
        {lq.account.length > 0 && lq.phase !== "scanning" && (
          <motion.div key="account" className="set-card lingq-account" {...enter}>
            {lq.phase === "importing" ? (
              <div className="lingq-progress">
                <Orb size={30} />
                <div className="grow">
                  <strong>{stageText(lq.stage)}</strong>
                  <div className="bar live">
                    <i style={{ width: `${pct}%` }} />
                  </div>
                  <span className="num">
                    {lq.total ? t(`${formatNumber(lq.done)} sur ${formatNumber(lq.total)}`, `${formatNumber(lq.done)} of ${formatNumber(lq.total)}`) : t("Préparation…", "Preparing…")}
                    {lq.stage?.stage === "lessons" && lq.lastLesson ? ` · ${lq.lastLesson}` : ""}
                  </span>
                </div>
                <button className="btn sm ghost" onClick={lq.cancel}>
                  {t("Arrêter", "Stop")}
                </button>
              </div>
            ) : (
              <>
                {lq.phase === "done" && lq.report && (
                  <div className="set-row lingq-report">
                    <span className="lingq-check">
                      <Icon name="check" size={15} stroke={2.2} />
                    </span>
                    <div className="grow">
                      <strong>{lq.report.cancelled ? t("Import interrompu", "Import stopped") : t("Import terminé", "Import finished")}</strong>
                      <span>
                        {[
                          summary(lq.report) || t("Rien de nouveau", "Nothing new"),
                          lq.report.skipped ? plural(lq.report.skipped, "leçon déjà présente", "leçons déjà présentes", "lesson already there", "lessons already there") : "",
                          lq.report.failed
                            ? `${plural(lq.report.failed, "leçon inaccessible", "leçons inaccessibles", "unavailable lesson", "unavailable lessons")} ${t("(réservées ou vides)", "(restricted or empty)")}`
                            : "",
                          lq.report.audio_failed ? `${plural(lq.report.audio_failed, "leçon", "leçons", "lesson", "lessons")} ${t("sans audio", "without audio")}` : "",
                        ]
                          .filter(Boolean)
                          .join(" · ")}
                      </span>
                    </div>
                  </div>
                )}

                <div className="set-row">
                  <div className="grow">
                    <span className="eyebrow">{t("Sur votre compte", "On your account")}</span>
                  </div>
                </div>
                {lq.account.map((a) => {
                  const li = langInfo(a.lang);
                  return (
                    <div key={a.lang} className="set-row">
                      <span className="lang-badge" style={{ background: li.color }}>
                        {li.badge}
                      </span>
                      <div className="grow">
                        <strong>{li.name}</strong>
                        <span className="num">
                          {plural(a.known_words, "mot connu", "mots connus", "known word", "known words")} · {plural(a.lingqs, "LingQ", "LingQ", "LingQ", "LingQs")} ·{" "}
                          {plural(a.courses.length, "cours", "cours", "course", "courses")}, {plural(a.lessons, "leçon", "leçons", "lesson", "lessons")}
                        </span>
                      </div>
                      <Switch on={lq.chosen.includes(a.lang)} onChange={() => lq.toggleLang(a.lang)} label={t(`Importer ${langLower(a.lang)}`, `Import ${li.name}`)} />
                    </div>
                  );
                })}

                <div className="set-row">
                  <div className="grow">
                    <span className="eyebrow">{t("À importer", "To import")}</span>
                  </div>
                </div>
                <div className="set-row">
                  <div className="grow">
                    <strong>{t("Vocabulaire", "Vocabulary")}</strong>
                    <span>
                      {t(
                        "Mots connus, ignorés et LingQ, avec traductions, notes et contexte. Les niveaux 1 à 3 restent en apprentissage, le niveau 4 et ✓ deviennent connus.",
                        "Known and ignored words and LingQs, with translations, notes and context. Levels 1 to 3 stay in learning, level 4 and ✓ become known.",
                      )}
                    </span>
                  </div>
                  <Switch on={lq.vocab} onChange={(v) => lq.setOption("vocab", v)} label={t("Importer le vocabulaire", "Import the vocabulary")} />
                </div>
                <div className="set-row">
                  <div className="grow">
                    <strong>{t("Leçons", "Lessons")}</strong>
                    <span>
                      {t(
                        `${lessonCount > 0 ? `Le texte ${lessonCount > 1 ? `des ${formatNumber(lessonCount)} leçons` : "de la leçon"} de vos cours` : "Le texte des leçons de vos cours"}, rangé par cours dans la bibliothèque.`,
                        `The text of ${lessonCount > 1 ? `the ${formatNumber(lessonCount)} lessons` : "the lessons"} in your courses, sorted by course in the library.`,
                      )}
                    </span>
                  </div>
                  <Switch on={lq.lessons} onChange={(v) => lq.setOption("lessons", v)} label={t("Importer les leçons", "Import the lessons")} />
                </div>
                {lq.lessons && (
                  <div className="set-row">
                    <div className="grow">
                      <strong>{t("Audio des leçons", "Lesson audio")}</strong>
                      <span>{t("Écoute synchronisée avec la lanterne. Comptez quelques Mo par leçon.", "Listening in sync with the lantern. Allow a few MB per lesson.")}</span>
                    </div>
                    <Switch on={lq.audio} onChange={(v) => lq.setOption("audio", v)} label={t("Importer l'audio", "Import the audio")} />
                  </div>
                )}

                <div className="set-row lingq-go">
                  <p className="lingq-note">
                    {t(
                      "Rien n'est effacé : un mot déjà présent garde son statut s'il est plus avancé, et une leçon déjà importée n'est pas dupliquée. Sur LingQ, les leçons lues pour l'import remontent dans votre étagère « Continuer ».",
                      "Nothing is erased: a word already there keeps its status if it is further along, and a lesson already imported isn't duplicated. On LingQ, the lessons read for the import move up in your “Continue” shelf.",
                    )}
                  </p>
                  <button className="btn primary glow" disabled={!canStart} onClick={() => lq.start(key)}>
                    <Icon name="import" size={15} /> {lq.phase === "done" ? t("Importer à nouveau", "Import again") : t("Tout importer", "Import everything")}
                  </button>
                </div>
              </>
            )}
          </motion.div>
        )}
      </AnimatePresence>
    </section>
  );
}
