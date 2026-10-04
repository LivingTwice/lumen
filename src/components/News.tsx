import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { useEffect, useMemo } from "react";
import { actionLabel, compareVersions, releaseDate, RELEASES, runAction, tx, unseenReleases, versionOf, type Release } from "../lib/changelog";
import { t } from "../lib/i18n";
import { useApp } from "../lib/store";
import { Icon } from "./Icon";

/* Nouveautés : après une mise à jour, ce qui a changé depuis la dernière version vue ;
   depuis Réglages › À propos, tout l'historique. */

const EASE = [0.16, 1, 0.3, 1] as const;

/** Ouvre les nouveautés d'elles-mêmes après une mise à jour (jamais pendant l'accueil ou la visite). */
export function useNewsOnUpdate() {
  const ready = useApp((s) => s.ready);
  const onboarded = useApp((s) => s.settings.onboarded);
  const replay = useApp((s) => s.replay);
  const tour = useApp((s) => s.tour);
  const version = useApp((s) => s.info?.version ?? "");
  const seen = useApp((s) => s.settings.seen_version ?? "");
  useEffect(() => {
    if (!ready || !onboarded || replay || tour !== null || !version) return;
    const current = version.match(/\d+\.\d+\.\d+/)?.[0];
    if (!current || (seen && compareVersions(seen, current) >= 0)) return;
    if (!unseenReleases(version, seen).length) {
      // rien à raconter (version de correction) : on retient simplement la version
      void useApp.getState().setSetting("seen_version", current);
      return;
    }
    // un instant après l'ouverture, quand la bibliothèque est posée
    const timer = window.setTimeout(() => useApp.getState().openNews("update"), 1400);
    return () => window.clearTimeout(timer);
  }, [ready, onboarded, replay, tour, version, seen]);
}

export function News() {
  const which = useApp((s) => s.news);
  const close = useApp((s) => s.closeNews);
  const version = useApp((s) => s.info?.version ?? "");
  const seen = useApp((s) => s.settings.seen_version ?? "");
  const still = !!useReducedMotion();

  // la liste est figée à l'ouverture : fermer (et retenir la version) ne la vide pas pendant la sortie
  const list = useMemo<Release[]>(() => {
    if (!which) return [];
    const unseen = which === "update" ? unseenReleases(version, seen) : [];
    return unseen.length ? unseen : RELEASES.filter((r) => r.items.length && compareVersions(versionOf(r, version), version) <= 0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [which]);

  useEffect(() => {
    if (!which) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey) return;
      // la fenêtre est modale : le lecteur derrière n'entend rien
      e.stopPropagation();
      if (e.key === "Escape" || (e.key === "Enter" && !(e.target as HTMLElement).closest?.("button"))) {
        e.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [which, close]);

  const act = (action: string) => {
    close();
    runAction(action);
  };

  const [head, ...older] = list;
  const v = head ? versionOf(head, version) : "";

  return (
    <AnimatePresence>
      {which && head && (
        <motion.div className="scrim news-scrim" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} onMouseDown={(e) => e.target === e.currentTarget && close()}>
          <motion.div
            className="news"
            role="dialog"
            aria-modal="true"
            aria-label={t("Nouveautés de Lumen", "What's new in Lumen")}
            initial={{ opacity: 0, y: 28, scale: 0.96 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 14, scale: 0.98 }}
            transition={{ type: "spring", stiffness: 300, damping: 30 }}
          >
            <div className="news-sky" aria-hidden="true">
              <motion.div
                className="news-sun"
                initial={still ? false : { y: 70, opacity: 0 }}
                animate={{ y: 0, opacity: 1 }}
                transition={{ duration: 1.6, delay: 0.15, ease: EASE }}
              >
                <span className="news-rays" />
                <span className="news-halo" />
                <span className="news-core" />
              </motion.div>
              <div className="news-horizon" />
              {Array.from({ length: 14 }).map((_, i) => (
                <i key={i} className="news-mote" style={{ left: `${(i * 37 + 9) % 100}%`, animationDelay: `${(i * 0.7) % 6}s`, animationDuration: `${7 + (i % 5)}s` }} />
              ))}
              <button className="icon-btn news-close" onClick={close} aria-label={t("Fermer", "Close")}>
                <Icon name="close" size={15} />
              </button>
            </div>

            <div className="news-scroll">
              <motion.header
                className="news-head"
                initial={still ? false : { opacity: 0, y: 12, filter: "blur(8px)" }}
                animate={{ opacity: 1, y: 0, filter: "blur(0px)" }}
                transition={{ duration: 0.7, delay: 0.35, ease: EASE }}
              >
                <span className="eyebrow">{which === "update" && older.length === 0 ? t("Nouveau dans Lumen", "New in Lumen") : t("Nouveautés", "What's new")}</span>
                <h2 className="display">
                  <span className="news-version">Lumen {v}</span>
                  <span className="news-sheen" aria-hidden="true">
                    Lumen {v}
                  </span>
                </h2>
                <p className="news-title">{tx(head.title)}</p>
                {releaseDate(head) && <span className="news-date">{releaseDate(head)}</span>}
              </motion.header>

              <ul className="news-items">
                {head.items.map((it, i) => (
                  <motion.li
                    key={i}
                    className="news-item"
                    initial={still ? false : { opacity: 0, y: 14 }}
                    animate={{ opacity: 1, y: 0 }}
                    transition={{ duration: 0.55, delay: 0.55 + i * 0.08, ease: EASE }}
                  >
                    <span className="news-ic">
                      <Icon name={it.icon} size={17} />
                    </span>
                    <span className="news-body">
                      <strong>{tx(it.title)}</strong>
                      <span>{tx(it.body)}</span>
                      {it.action && (
                        <button className="news-act" onClick={() => act(it.action!)}>
                          {actionLabel(it.action)} <Icon name="forward" size={12} stroke={2} />
                        </button>
                      )}
                    </span>
                  </motion.li>
                ))}
              </ul>

              {older.map((r) => (
                <section key={r.version} className="news-older">
                  <div className="news-older-head">
                    <strong>Lumen {versionOf(r, version)}</strong>
                    <span>{tx(r.title)}</span>
                    {releaseDate(r) && <span className="news-date">{releaseDate(r)}</span>}
                  </div>
                  <ul>
                    {r.items.map((it, i) => (
                      <li key={i}>
                        <Icon name={it.icon} size={14} />
                        <span>
                          <strong>{tx(it.title)}</strong>
                          <span>{tx(it.body)}</span>
                        </span>
                      </li>
                    ))}
                  </ul>
                </section>
              ))}
            </div>

            <div className="news-foot">
              <span className="news-hint">{t("Retrouvez les nouveautés dans Réglages › À propos.", "Find what's new any time in Settings › About.")}</span>
              <button className="btn primary glow" onClick={close}>
                {t("Continuer", "Continue")} <Icon name="forward" size={14} stroke={2} />
              </button>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
