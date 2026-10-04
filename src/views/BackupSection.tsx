import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "react";
import { Icon } from "../components/Icon";
import { Orb, Switch } from "../components/ui";
import {
  dayLabel,
  formatWhen,
  pickBackupFolder,
  placeLabel,
  reloadProgress,
  restoreStage,
  restoredText,
  revealBackup,
  statusLine,
  useBackup,
} from "../lib/backup";
import { confirmAsk } from "../lib/dialogs";
import { count, t } from "../lib/i18n";
import { langLower } from "../lib/langs";
import { formatBytes, formatNumber, useApp } from "../lib/store";
import type { BackupInfo } from "../lib/types";

const enter = { initial: { opacity: 0, y: 10 }, animate: { opacity: 1, y: 0 }, exit: { opacity: 0, y: 6 }, transition: { type: "spring" as const, stiffness: 320, damping: 30 } };

const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/** Une sauvegarde trouvée : ce qu'elle contient, ses versions, « Restaurer… ». */
function BackupRow({ info }: { info: BackupInfo }) {
  const restoring = useBackup((s) => s.restoring);
  const restore = useBackup((s) => s.restore);
  const toast = useApp((s) => s.toast);
  const [day, setDay] = useState("");
  const progress = restoring?.key === info.key ? restoring : null;
  const version = info.versions.find((v) => v.day === day);
  const known = version?.known ?? info.counts.known;
  const lessons = version?.lessons ?? info.counts.lessons;
  const langs = info.counts.langs.map((l) => langLower(l)).join(", ");
  const pct = progress ? (progress.stage === "media" ? progress.value * 100 : progress.stage === "apply" ? 100 : 6) : 0;

  const go = async () => {
    const when = version ? dayLabel(version.day) : formatWhen(info.saved_at);
    const ok = await confirmAsk(
      t(
        `Remplacer la progression de ce Mac par celle de « ${info.device_name} », sauvegardée ${when} ?\n\n${formatNumber(known)} mots connus et ${formatNumber(lessons)} leçon${lessons > 1 ? "s" : ""}. Votre progression actuelle reste en copie sur ce Mac.`,
        `Replace the progress on this Mac with the one from “${info.device_name}”, backed up ${when}?\n\n${formatNumber(known)} known words and ${count(lessons, "", "", "lesson", "lessons")}. Your current progress is kept as a copy on this Mac.`,
      ),
      t("Restaurer une sauvegarde", "Restore a backup"),
      t("Restaurer", "Restore"),
    );
    if (!ok) return;
    const r = await restore(info, day || null);
    if (!r) return;
    await reloadProgress();
    toast(restoredText(r), "light");
    if (r.missing_media) {
      const n = r.missing_media;
      toast(
        t(
          `${n} fichier${n > 1 ? "s" : ""} audio ou vidéo manquai${n > 1 ? "ent" : "t"} dans la sauvegarde : ces leçons restent lisibles, et les vidéos en ligne se retéléchargent depuis la leçon.`,
          `${n} audio or video file${n > 1 ? "s were" : " was"} missing from the backup: these lessons can still be read, and online videos can be downloaded again from the lesson.`,
        ),
      );
    }
  };

  return (
    <motion.div layout="position" className="set-row backup-item" {...enter}>
      <span className={`backup-device ${info.this_device ? "here" : ""}`}>
        <Icon name="laptop" size={18} />
      </span>
      <div className="grow">
        <strong>
          {info.device_name}
          {info.this_device && <span className="chip light">{info.mine ? t("Ce Mac", "This Mac") : t("Ce Mac, autre profil", "This Mac, other profile")}</span>}
        </strong>
        <span>
          {progress
            ? restoreStage(progress.stage) ?? t("Restauration…", "Restoring…")
            : `${cap(version ? dayLabel(version.day) : formatWhen(info.saved_at))} · ${count(known, "mot connu", "mots connus", "known word", "known words")} · ${count(lessons, "leçon", "leçons", "lesson", "lessons")}${langs ? ` · ${langs}` : ""}`}
        </span>
        {progress && (
          <div className="bar live">
            <i style={{ width: `${pct}%` }} />
          </div>
        )}
        {info.newer && <span className="backup-warn">{t("Faite par une version plus récente de Lumen : mettez Lumen à jour pour la restaurer.", "Made by a newer version of Lumen: update Lumen to restore it.")}</span>}
      </div>
      {info.versions.length > 0 && !progress && (
        <select className="select" value={day} onChange={(e) => setDay(e.target.value)} aria-label={t("Version à restaurer", "Version to restore")}>
          <option value="">{t("La plus récente", "The most recent")}</option>
          {info.versions.map((v) => (
            <option key={v.day} value={v.day}>
              {cap(dayLabel(v.day))}
            </option>
          ))}
        </select>
      )}
      <button className="btn sm soft" disabled={!!restoring || info.newer} onClick={go}>
        {progress ? t("Restauration…", "Restoring…") : t("Restaurer…", "Restore…")}
      </button>
    </motion.div>
  );
}

/** Réglages › Sauvegarde : iCloud Drive (ou un dossier choisi), médias, restauration. */
export function BackupSection() {
  const s = useBackup((x) => x.status);
  const saving = useBackup((x) => x.saving);
  const list = useBackup((x) => x.list);
  const listing = useBackup((x) => x.listing);
  const listError = useBackup((x) => x.listError);
  const restoring = useBackup((x) => !!x.restoring);
  const { enable, save, load, refresh } = useBackup.getState();
  const settings = useApp((x) => x.settings);
  const setSetting = useApp((x) => x.setSetting);
  const toast = useApp((x) => x.toast);

  useEffect(() => {
    void refresh();
    // la date de la dernière sauvegarde et l'envoi vers iCloud avancent tout seuls
    const t = window.setInterval(() => void useBackup.getState().refresh(), 15000);
    return () => window.clearInterval(t);
  }, [refresh]);

  // la liste n'est lue qu'une fois la sauvegarde acceptée : macOS demande alors l'accès à iCloud Drive
  const enabled = !!s?.enabled;
  useEffect(() => {
    if (enabled && useBackup.getState().list === null) void load();
  }, [enabled, load]);

  if (!s) return null;

  // une restauration occupe aussi le natif : ce n'est pas une sauvegarde
  const busy = saving || (s.running && !restoring);
  const beacon = busy ? "busy" : s.enabled && s.error ? "warn" : s.enabled && s.last_at !== null ? "on" : "off";

  const toggle = async (on: boolean) => {
    const ok = await enable(on);
    if (on && ok) toast(useBackup.getState().status?.last_at ? t("Progression sauvegardée", "Progress backed up") : t("Sauvegarde activée", "Backup turned on"), "light");
  };

  const setOption = async (key: "backup_audio" | "backup_video", on: boolean) => {
    await setSetting(key, on ? "1" : "0");
    if (s.enabled) void save();
  };

  // nouvel emplacement : la sauvegarde y est refaite tout de suite (l'ancien dossier reste intact)
  const moveTo = async (dir: string) => {
    await setSetting("backup_dir", dir);
    useBackup.setState({ list: null });
    if (s.enabled) {
      if (await save()) void load();
    } else await refresh();
  };

  const choose = async () => {
    const dir = await pickBackupFolder();
    if (dir) await moveTo(dir);
  };

  return (
    // le titre et la présentation sont dans l'en-tête de la page des Réglages
    <section className="set-section">

      <div className="set-card">
        <div className="set-row backup-hero">
          <span className={`backup-beacon ${beacon}`} aria-hidden="true">
            <i />
          </span>
          <div className="grow">
            <strong>{t("Sauvegarde automatique", "Automatic backup")}</strong>
            <span className={s.enabled && s.error && !busy ? "backup-warn" : ""}>{statusLine({ ...s, running: busy }, saving)}</span>
          </div>
          <Switch on={s.enabled} onChange={(v) => void toggle(v)} label={t("Sauvegarde automatique", "Automatic backup")} />
        </div>
        <div className="set-row">
          <div className="grow">
            <strong>{t("Audio des leçons", "Lesson audio")}</strong>
            <span>
              {s.local_audio
                ? t(
                    `${formatBytes(s.local_audio)} sur ce Mac. Sans lui, une leçon audio restaurée redevient une leçon de texte.`,
                    `${formatBytes(s.local_audio)} on this Mac. Without it, a restored audio lesson becomes a text lesson again.`,
                  )
                : t("Aucun audio sur ce Mac pour l'instant.", "No audio on this Mac yet.")}
            </span>
          </div>
          <Switch on={settings.backup_audio !== "0"} onChange={(v) => void setOption("backup_audio", v)} label={t("Sauvegarder l'audio des leçons", "Back up lesson audio")} />
        </div>
        <div className="set-row">
          <div className="grow">
            <strong>{t("Vidéos", "Videos")}</strong>
            <span>
              {s.local_video ? t(`${formatBytes(s.local_video)} sur ce Mac. `, `${formatBytes(s.local_video)} on this Mac. `) : ""}
              {t("Les vidéos en ligne se retéléchargent depuis leur leçon : inutile d'en encombrer iCloud.", "Online videos can be downloaded again from their lesson: no need to fill iCloud with them.")}
            </span>
          </div>
          <Switch on={settings.backup_video === "1"} onChange={(v) => void setOption("backup_video", v)} label={t("Sauvegarder les vidéos", "Back up videos")} />
        </div>
        <div className="set-row">
          <div className="grow">
            <strong>{t("Emplacement", "Location")}</strong>
            <span>
              {s.dir
                ? placeLabel(s)
                : t(
                    "iCloud Drive n'est pas activé sur ce Mac (Réglages Système › votre nom › iCloud). Vous pouvez aussi choisir un dossier : Dropbox, Google Drive, clé USB…",
                    "iCloud Drive isn't turned on on this Mac (System Settings › your name › iCloud). You can also choose a folder: Dropbox, Google Drive, a USB drive…",
                  )}
            </span>
          </div>
          {s.dir && s.last_at !== null && (
            <button className="btn sm ghost" onClick={() => void revealBackup(s.dir!)}>
              {t("Afficher", "Show")}
            </button>
          )}
          {!s.icloud && s.icloud_available && (
            <button className="btn sm ghost" onClick={() => void moveTo("")}>
              iCloud Drive
            </button>
          )}
          <button className="btn sm soft" onClick={() => void choose()}>
            {t("Choisir…", "Choose…")}
          </button>
        </div>
        <div className="set-row backup-foot">
          <span className="backup-note">
            {t(
              "Toutes les 10 minutes pendant que vous lisez, et en quittant Lumen. Les versions des 14 derniers jours restent disponibles.",
              "Every 10 minutes while you read, and when you quit Lumen. Versions from the last 14 days stay available.",
            )}
          </span>
          <button className="btn sm primary" disabled={busy || !s.dir} onClick={() => void save()}>
            {busy ? t("Sauvegarde…", "Backing up…") : t("Sauvegarder maintenant", "Back up now")}
          </button>
        </div>
      </div>

      <div className="set-card backup-list">
        <div className="set-row">
          <div className="grow">
            <span className="eyebrow">{t("Retrouver une progression", "Get progress back")}</span>
            <span>{t("Depuis ce Mac ou un autre. La progression de ce Mac est alors remplacée, et gardée en copie.", "From this Mac or another one. The progress on this Mac is then replaced, and kept as a copy.")}</span>
          </div>
          {list !== null && (
            <button className={`icon-btn ${listing ? "spin" : ""}`} onClick={() => void load()} disabled={listing} aria-label={t("Chercher à nouveau", "Search again")}>
              <Icon name="refresh" size={16} />
            </button>
          )}
        </div>
        <AnimatePresence initial={false} mode="popLayout">
          {list === null ? (
            <motion.div key="ask" className="set-row" {...enter}>
              <div className="grow">
                <span>{t("Vous avez déjà utilisé Lumen sur un autre Mac ?", "Have you used Lumen on another Mac before?")}</span>
              </div>
              <button className="btn sm soft" disabled={listing || !s.dir} onClick={() => void load()}>
                {listing ? t("Recherche…", "Searching…") : t("Chercher une sauvegarde", "Look for a backup")}
              </button>
            </motion.div>
          ) : listing && !list.length ? (
            <motion.div key="wait" className="set-row lingq-wait" {...enter}>
              <Orb size={18} />
              <span>{s.icloud ? t("Recherche dans votre iCloud Drive…", "Searching your iCloud Drive…") : t("Recherche dans le dossier choisi…", "Searching the chosen folder…")}</span>
            </motion.div>
          ) : listError ? (
            <motion.div key="err" className="set-row lingq-error" {...enter}>
              {listError}
            </motion.div>
          ) : !list.length ? (
            <motion.div key="none" className="set-row lingq-wait" {...enter}>
              {s.icloud ? t("Aucune sauvegarde dans votre iCloud Drive pour l'instant.", "No backup in your iCloud Drive yet.") : t("Aucune sauvegarde dans ce dossier pour l'instant.", "No backup in this folder yet.")}
            </motion.div>
          ) : (
            list.map((b) => <BackupRow key={b.key} info={b} />)
          )}
        </AnimatePresence>
      </div>
    </section>
  );
}
