import { AnimatePresence, motion } from "motion/react";
import { useUpdate } from "../lib/updater";
import { Icon } from "./Icon";
import { t } from "../lib/i18n";

/** Carte discrète en bas de la barre latérale quand une mise à jour existe. */
export function UpdateCard() {
  const { phase, version, progress, dismissed, install, restart, dismiss, error } = useUpdate();
  const visible = !dismissed && ["available", "downloading", "ready"].includes(phase);
  return (
    <AnimatePresence>
      {visible && (
        <motion.div
          className="update-card"
          initial={{ opacity: 0, y: 12, scale: 0.97 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: 8 }}
          transition={{ type: "spring", stiffness: 300, damping: 26 }}
        >
          <div className="update-head">
            <span className="update-dot" />
            <strong>{phase === "ready" ? t("Mise à jour installée", "Update installed") : t(`Lumen ${version} est disponible`, `Lumen ${version} is available`)}</strong>
            {phase === "available" && (
              <button className="icon-btn" style={{ width: 24, height: 24, marginLeft: "auto" }} onClick={dismiss} aria-label={t("Plus tard", "Later")}>
                <Icon name="close" size={12} />
              </button>
            )}
          </div>
          {phase === "available" && (
            <button className="btn sm primary glow" onClick={install}>
              <Icon name="download" size={14} /> {t("Mettre à jour", "Update")}
            </button>
          )}
          {phase === "downloading" && (
            <div className="bar live">
              <i style={{ width: `${Math.round(progress * 100)}%` }} />
            </div>
          )}
          {phase === "ready" && (
            <button className="btn sm primary glow" onClick={restart}>
              {t("Redémarrer Lumen", "Restart Lumen")}
            </button>
          )}
          {error && phase !== "available" && <span className="muted" style={{ fontSize: 11.5 }}>{error}</span>}
        </motion.div>
      )}
    </AnimatePresence>
  );
}
