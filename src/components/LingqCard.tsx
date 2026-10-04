import { AnimatePresence, motion } from "motion/react";
import { stageText, useLingq } from "../lib/lingq";
import { useApp } from "../lib/store";
import { t } from "../lib/i18n";

/** Avancement de l'import LingQ dans la barre latérale, hors des Réglages. */
export function LingqCard() {
  const { phase, stage, done, total } = useLingq();
  // cachée seulement sur la page LingQ des Réglages, qui montre déjà l'avancement
  const here = useApp((s) => s.view === "settings" && s.settingsTab === "lingq");
  const openSettings = useApp((s) => s.openSettings);
  const visible = phase === "importing" && !here;
  return (
    <AnimatePresence>
      {visible && (
        <motion.button
          className="update-card"
          style={{ textAlign: "left", cursor: "pointer" }}
          onClick={() => openSettings("lingq")}
          initial={{ opacity: 0, y: 12, scale: 0.97 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: 8 }}
          transition={{ type: "spring", stiffness: 300, damping: 26 }}
        >
          <div className="update-head">
            <span className="update-dot" />
            <strong>{t("Import LingQ", "LingQ import")}</strong>
          </div>
          <span className="muted" style={{ fontSize: 11.5 }}>
            {stageText(stage)}
          </span>
          <div className="bar live">
            <i style={{ width: `${total ? Math.min(100, (done / total) * 100) : 0}%` }} />
          </div>
        </motion.button>
      )}
    </AnimatePresence>
  );
}
