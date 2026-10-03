import { AnimatePresence, motion } from "motion/react";
import { useBackup } from "../lib/backup";
import { useApp } from "../lib/store";
import { Icon } from "./Icon";

/** Une semaine de répit après « Plus tard ». */
const SNOOZE = 7 * 86400;

/** Barre latérale : propose la sauvegarde tant qu'elle n'est pas choisie,
 * et signale une sauvegarde qui échoue (iCloud refusé, disque débranché…). */
export function BackupCard() {
  const status = useBackup((s) => s.status);
  const saving = useBackup((s) => s.saving);
  const view = useApp((s) => s.view);
  const go = useApp((s) => s.go);
  const toast = useApp((s) => s.toast);
  const snooze = useApp((s) => Number(s.settings.backup_snooze) || 0);
  const setSetting = useApp((s) => s.setSetting);

  const offer = !!status && !status.decided && !!status.dir && Date.now() / 1000 - snooze > SNOOZE;
  const failing = !!status?.enabled && !!status.error && !saving && !status.running && view !== "settings";

  const activate = async () => {
    if (!(await useBackup.getState().enable(true))) return;
    const where = useBackup.getState().status?.icloud ? "dans iCloud Drive" : "dans le dossier choisi";
    toast(useBackup.getState().status?.last_at ? `Progression sauvegardée ${where}` : `Sauvegarde activée ${where}`, "light");
  };

  const openSettings = () => {
    go("settings");
    window.setTimeout(() => document.getElementById("set-backup")?.scrollIntoView({ behavior: "smooth", block: "start" }), 350);
  };

  return (
    <AnimatePresence>
      {offer && (
        <motion.div
          key="offer"
          className="update-card"
          initial={{ opacity: 0, y: 12, scale: 0.97 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: 8 }}
          transition={{ type: "spring", stiffness: 300, damping: 26 }}
        >
          <div className="update-head">
            <span className="update-dot" />
            <strong>Mettez vos progrès à l'abri</strong>
            <button
              className="icon-btn"
              style={{ width: 24, height: 24, marginLeft: "auto" }}
              onClick={() => void setSetting("backup_snooze", String(Math.floor(Date.now() / 1000)))}
              aria-label="Plus tard"
            >
              <Icon name="close" size={12} />
            </button>
          </div>
          <span className="muted" style={{ fontSize: 11.5, lineHeight: 1.45 }}>
            Une copie de vos mots et de vos leçons dans votre {status.icloud ? "iCloud Drive" : "dossier de sauvegarde"}, si ce Mac venait à s'effacer.
          </span>
          <button className="btn sm primary glow" disabled={saving} onClick={() => void activate()}>
            <Icon name="cloud" size={14} /> {saving ? "Sauvegarde…" : "Activer la sauvegarde"}
          </button>
        </motion.div>
      )}
      {failing && (
        <motion.button
          key="failing"
          className="update-card"
          style={{ textAlign: "left", cursor: "pointer" }}
          onClick={openSettings}
          initial={{ opacity: 0, y: 12, scale: 0.97 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: 8 }}
          transition={{ type: "spring", stiffness: 300, damping: 26 }}
        >
          <div className="update-head">
            <span className="update-dot warn" />
            <strong>Sauvegarde interrompue</strong>
          </div>
          <span className="muted" style={{ fontSize: 11.5, lineHeight: 1.45 }}>
            {status.error}
          </span>
        </motion.button>
      )}
    </AnimatePresence>
  );
}
