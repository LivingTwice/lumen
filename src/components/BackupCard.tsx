import { AnimatePresence, motion } from "motion/react";
import { useBackup } from "../lib/backup";
import { useApp } from "../lib/store";
import { Icon } from "./Icon";
import { t } from "../lib/i18n";

/** Une semaine de répit après « Plus tard ». */
const SNOOZE = 7 * 86400;

/** Barre latérale : propose la sauvegarde tant qu'elle n'est pas choisie,
 * et signale une sauvegarde qui échoue (iCloud refusé, disque débranché…). */
export function BackupCard() {
  const status = useBackup((s) => s.status);
  const saving = useBackup((s) => s.saving);
  // l'alerte se tait sur la page Sauvegarde des Réglages, qui dit déjà ce qui se passe
  const here = useApp((s) => s.view === "settings" && s.settingsTab === "backup");
  const showSettings = useApp((s) => s.openSettings);
  const toast = useApp((s) => s.toast);
  const snooze = useApp((s) => Number(s.settings.backup_snooze) || 0);
  const setSetting = useApp((s) => s.setSetting);

  const offer = !!status && !status.decided && !!status.dir && Date.now() / 1000 - snooze > SNOOZE;
  const failing = !!status?.enabled && !!status.error && !saving && !status.running && !here;

  const activate = async () => {
    if (!(await useBackup.getState().enable(true))) return;
    const where = useBackup.getState().status?.icloud ? t("dans iCloud Drive", "to iCloud Drive") : t("dans le dossier choisi", "to the chosen folder");
    toast(useBackup.getState().status?.last_at ? t(`Progression sauvegardée ${where}`, `Progress backed up ${where}`) : t(`Sauvegarde activée ${where}`, `Backup turned on, ${where}`), "light");
  };

  const openSettings = () => {
    showSettings("backup");
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
            <strong>{t("Mettez vos progrès à l'abri", "Keep your progress safe")}</strong>
            <button
              className="icon-btn"
              style={{ width: 24, height: 24, marginLeft: "auto" }}
              onClick={() => void setSetting("backup_snooze", String(Math.floor(Date.now() / 1000)))}
              aria-label={t("Plus tard", "Later")}
            >
              <Icon name="close" size={12} />
            </button>
          </div>
          <span className="muted" style={{ fontSize: 11.5, lineHeight: 1.45 }}>
            {t(
              `Une copie de vos mots et de vos leçons dans votre ${status.icloud ? "iCloud Drive" : "dossier de sauvegarde"}, si ce Mac venait à s'effacer.`,
              `A copy of your words and lessons in your ${status.icloud ? "iCloud Drive" : "backup folder"}, in case this Mac is ever wiped.`,
            )}
          </span>
          <button className="btn sm primary glow" disabled={saving} onClick={() => void activate()}>
            <Icon name="cloud" size={14} /> {saving ? t("Sauvegarde…", "Backing up…") : t("Activer la sauvegarde", "Turn on backup")}
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
            <strong>{t("Sauvegarde interrompue", "Backup interrupted")}</strong>
          </div>
          <span className="muted" style={{ fontSize: 11.5, lineHeight: 1.45 }}>
            {status.error}
          </span>
        </motion.button>
      )}
    </AnimatePresence>
  );
}
