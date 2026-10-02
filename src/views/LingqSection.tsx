import { AnimatePresence, motion } from "motion/react";
import { useState } from "react";
import { Icon } from "../components/Icon";
import { Orb, Switch } from "../components/ui";
import { isTauri } from "../lib/api";
import { langInfo } from "../lib/langs";
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
      <p>Retrouvez dans Lumen tout votre parcours LingQ : mots connus et ignorés, LingQ avec leurs traductions et leurs notes, et les leçons de tous vos cours avec leur audio.</p>

      <div className="set-card">
        <div className="set-row">
          <div className="grow">
            <strong>Clé API LingQ</strong>
            <span>
              Copiez-la depuis{" "}
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
              , connecté à votre compte. Elle reste sur ce Mac.
            </span>
          </div>
        </div>
        <div className="set-row">
          <div className="key-field">
            <input
              className="input"
              type={show ? "text" : "password"}
              value={key}
              placeholder="Collez votre clé ici"
              spellCheck={false}
              autoComplete="off"
              autoCorrect="off"
              onChange={(e) => setSetting("lingq_key", e.target.value.trim())}
              onKeyDown={(e) => e.key === "Enter" && key && !busy && lq.scan(key)}
              aria-label="Clé API LingQ"
            />
            <button className={`icon-btn ${show ? "on" : ""}`} onClick={() => setShow(!show)} aria-label={show ? "Masquer la clé" : "Afficher la clé"}>
              <Icon name="eye" size={15} />
            </button>
          </div>
          <button className="btn sm primary" disabled={!key || busy} onClick={() => lq.scan(key)}>
            {lq.phase === "scanning" ? "Analyse…" : lq.account.length ? "Analyser à nouveau" : "Analyser mon compte"}
          </button>
        </div>
        <AnimatePresence initial={false}>
          {lq.phase === "scanning" && (
            <motion.div key="scan" className="set-row lingq-wait" {...enter}>
              <Orb size={18} />
              <span>Lecture de votre compte LingQ, langue par langue…</span>
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
                    {lq.total ? `${formatNumber(lq.done)} sur ${formatNumber(lq.total)}` : "Préparation…"}
                    {lq.stage?.stage === "lessons" && lq.lastLesson ? ` · ${lq.lastLesson}` : ""}
                  </span>
                </div>
                <button className="btn sm ghost" onClick={lq.cancel}>
                  Arrêter
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
                      <strong>{lq.report.cancelled ? "Import interrompu" : "Import terminé"}</strong>
                      <span>
                        {[
                          summary(lq.report) || "Rien de nouveau",
                          lq.report.skipped ? `${plural(lq.report.skipped, "leçon déjà présente", "leçons déjà présentes")}` : "",
                          lq.report.failed ? `${plural(lq.report.failed, "leçon inaccessible", "leçons inaccessibles")} (réservées ou vides)` : "",
                          lq.report.audio_failed ? `${plural(lq.report.audio_failed, "leçon", "leçons")} sans audio` : "",
                        ]
                          .filter(Boolean)
                          .join(" · ")}
                      </span>
                    </div>
                  </div>
                )}

                <div className="set-row">
                  <div className="grow">
                    <span className="eyebrow">Sur votre compte</span>
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
                          {plural(a.known_words, "mot connu", "mots connus")} · {plural(a.lingqs, "LingQ", "LingQ")} · {plural(a.courses.length, "cours", "cours")},{" "}
                          {plural(a.lessons, "leçon", "leçons")}
                        </span>
                      </div>
                      <Switch on={lq.chosen.includes(a.lang)} onChange={() => lq.toggleLang(a.lang)} label={`Importer ${li.name.toLowerCase()}`} />
                    </div>
                  );
                })}

                <div className="set-row">
                  <div className="grow">
                    <span className="eyebrow">À importer</span>
                  </div>
                </div>
                <div className="set-row">
                  <div className="grow">
                    <strong>Vocabulaire</strong>
                    <span>Mots connus, ignorés et LingQ, avec traductions, notes et contexte. Les niveaux 1 à 3 restent en apprentissage, le niveau 4 et ✓ deviennent connus.</span>
                  </div>
                  <Switch on={lq.vocab} onChange={(v) => lq.setOption("vocab", v)} label="Importer le vocabulaire" />
                </div>
                <div className="set-row">
                  <div className="grow">
                    <strong>Leçons</strong>
                    <span>
                      {lessonCount > 0 ? `Le texte ${lessonCount > 1 ? `des ${formatNumber(lessonCount)} leçons` : "de la leçon"} de vos cours` : "Le texte des leçons de vos cours"}, rangé par cours dans la
                      bibliothèque.
                    </span>
                  </div>
                  <Switch on={lq.lessons} onChange={(v) => lq.setOption("lessons", v)} label="Importer les leçons" />
                </div>
                {lq.lessons && (
                  <div className="set-row">
                    <div className="grow">
                      <strong>Audio des leçons</strong>
                      <span>Écoute synchronisée avec la lanterne. Comptez quelques Mo par leçon.</span>
                    </div>
                    <Switch on={lq.audio} onChange={(v) => lq.setOption("audio", v)} label="Importer l'audio" />
                  </div>
                )}

                <div className="set-row lingq-go">
                  <p className="lingq-note">
                    Rien n'est effacé : un mot déjà présent garde son statut s'il est plus avancé, et une leçon déjà importée n'est pas dupliquée. Sur LingQ, les leçons lues
                    pour l'import remontent dans votre étagère « Continuer ».
                  </p>
                  <button className="btn primary glow" disabled={!canStart} onClick={() => lq.start(key)}>
                    <Icon name="import" size={15} /> {lq.phase === "done" ? "Importer à nouveau" : "Tout importer"}
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
