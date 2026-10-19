import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "react";
import { Avatar } from "../components/Avatar";
import { Icon, type IconName } from "../components/Icon";
import { Orb, Switch } from "../components/ui";
import { api, errorText, isTauri } from "../lib/api";
import { isIOS, isWindows } from "../lib/platform";
import {
  dayLabel,
  formatWhen,
  joinOtherDevice,
  isPlaceOf,
  pickBackupFolder,
  placeDir,
  placeLabel,
  placeName,
  reloadProgress,
  restoreStage,
  restoredText,
  revealBackup,
  statusLine,
  syncedText,
  useBackup,
} from "../lib/backup";
import { confirmAsk } from "../lib/dialogs";
import { count, t } from "../lib/i18n";
import { langLower } from "../lib/langs";
import { formatBytes, formatNumber, useApp } from "../lib/store";
import type { BackupInfo, BackupPlace, BackupStatus } from "../lib/types";
import { userFrom } from "../lib/user";

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
  // le profil de cette sauvegarde : on reconnaît la sienne à son nom et à son avatar
  const who = userFrom(info.name, info.avatar, info.photo);

  // fusionner : sa progression rejoint celle-ci, rien n'est remplacé
  const [merging, setMerging] = useState(false);
  const merge = async () => {
    setMerging(true);
    try {
      const r = await api().backupMerge(info.key);
      await reloadProgress();
      toast(syncedText({ device_name: info.device_name, report: r }), "light");
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setMerging(false);
    }
  };

  const go = async () => {
    const when = version ? dayLabel(version.day) : formatWhen(info.saved_at);
    const ok = await confirmAsk(
      t(
        `Remplacer la progression de ce Mac par celle de « ${who.name ? `${who.name}, sur ${info.device_name}` : info.device_name} », sauvegardée ${when} ?\n\n${formatNumber(known)} mots connus et ${formatNumber(lessons)} leçon${lessons > 1 ? "s" : ""}. Votre progression actuelle reste en copie sur ce Mac.`,
        `Replace the progress on this Mac with the one from “${who.name ? `${who.name}, on ${info.device_name}` : info.device_name}”, backed up ${when}?\n\n${formatNumber(known)} known words and ${count(lessons, "", "", "lesson", "lessons")}. Your current progress is kept as a copy on this Mac.`,
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
      {who.empty ? (
        <span className={`backup-device ${info.this_device ? "here" : ""}`}>
          <Icon name="laptop" size={18} />
        </span>
      ) : (
        <Avatar size={34} spec={who.avatar} name={who.name} photo={who.photo} empty={false} />
      )}
      <div className="grow">
        <strong>
          {who.name ? (
            <>
              {who.name}
              <span className="backup-on">{t(`sur ${info.device_name}`, `on ${info.device_name}`)}</span>
            </>
          ) : (
            info.device_name
          )}
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
      {!info.this_device && !progress && !day && (
        <button className="btn sm soft" disabled={!!restoring || info.newer || merging} onClick={() => void merge()} title={t("Ajouter sa progression à la vôtre, sans rien remplacer", "Add its progress to yours, without replacing anything")}>
          {merging ? t("Fusion…", "Merging…") : t("Fusionner", "Merge")}
        </button>
      )}
      <button className={`btn sm ${info.this_device ? "soft" : "ghost"}`} disabled={!!restoring || info.newer || merging} onClick={go}>
        {progress ? t("Restauration…", "Restoring…") : info.this_device ? t("Restaurer…", "Restore…") : t("Remplacer…", "Replace…")}
      </button>
    </motion.div>
  );
}

/** Pictogramme d'un emplacement : nuage, disque ou dossier. */
export function placeIcon(kind: BackupPlace["kind"]): IconName {
  switch (kind) {
    case "dropbox":
    case "gdrive":
    case "onedrive":
    case "box":
    case "folder":
      return kind;
    case "drive":
      return "disk";
    default:
      return "cloud";
  }
}

/** Les grands nuages, proposés même absents : on apprend qu'il suffit de les installer. */
const SUGGESTED: { kind: BackupPlace["kind"]; name: string; url: string }[] = [
  { kind: "dropbox", name: "Dropbox", url: "https://www.dropbox.com/install" },
  { kind: "gdrive", name: "Google Drive", url: "https://www.google.com/drive/download/" },
  { kind: "onedrive", name: "OneDrive", url: "https://www.microsoft.com/microsoft-365/onedrive/download" },
];

const TILE = { type: "spring", stiffness: 520, damping: 40 } as const;

/** Où va la sauvegarde : iCloud Drive, les autres nuages du Mac, un autre dossier. */
function PlacePicker({ s, onPick }: { s: BackupStatus; onPick(dir: string): Promise<void> }) {
  const places = useBackup((x) => x.places);
  const loadPlaces = useBackup((x) => x.loadPlaces);
  const [busy, setBusy] = useState<string | null>(null);
  const [missing, setMissing] = useState<(typeof SUGGESTED)[number] | null>(null);

  // un nuage installé pendant que la page est ouverte apparaît au retour dans Lumen
  useEffect(() => {
    void loadPlaces();
    const again = () => void loadPlaces();
    window.addEventListener("focus", again);
    return () => window.removeEventListener("focus", again);
  }, [loadPlaces]);

  if (!places) return null;
  const known = places.some((p) => isPlaceOf(s, p));
  const other = !known && !!s.dir;
  const absent = SUGGESTED.filter((x) => !places.some((p) => p.kind === x.kind));
  // un même service avec plusieurs comptes : le compte suffit à les distinguer
  const twice = (p: BackupPlace) => places.filter((q) => q.kind === p.kind).length > 1;

  const pick = async (p: BackupPlace) => {
    setMissing(null);
    if (isPlaceOf(s, p) || busy) return;
    const key = p.path || p.kind;
    setBusy(key);
    const dir = await placeDir(p);
    if (dir !== null) await onPick(dir);
    setBusy(null);
  };
  const choose = async () => {
    setMissing(null);
    const dir = await pickBackupFolder();
    if (dir) await onPick(dir);
  };

  const tile = (key: string, icon: IconName, name: string, note: string, on: boolean, run: () => void, extra = "") => (
    <button key={key} className={`place ${on ? "on" : ""} ${extra}`} onClick={run} aria-pressed={on} title={name}>
      {on && <motion.span layoutId="backup-place" className="look-ring" transition={TILE} />}
      <span className="place-glyph">
        {busy === key ? <Orb size={16} /> : <Icon name={icon} size={20} />}
      </span>
      <strong>{name}</strong>
      <span className="place-note">{note}</span>
    </button>
  );

  return (
    <div className="place-pick">
      <div className="places" role="group" aria-label={t("Emplacement de la sauvegarde", "Backup location")}>
        {places.map((p) =>
          tile(
            p.path || p.kind,
            placeIcon(p.kind),
            p.name,
            p.account && (twice(p) || p.kind === "gdrive") ? p.account : isPlaceOf(s, p) ? t("Sauvegarde ici", "Backed up here") : t("Disponible", "Available"),
            isPlaceOf(s, p),
            () => void pick(p),
          ),
        )}
        {absent.map((x) =>
          tile(x.kind, placeIcon(x.kind), x.name, t("À installer", "Not installed"), false, () => setMissing((m) => (m?.kind === x.kind ? null : x)), `absent ${missing?.kind === x.kind ? "asked" : ""}`),
        )}
        {tile(
          "other",
          s.place?.kind === "drive" && other ? "disk" : "folder",
          other ? s.place?.name || t("Autre dossier", "Other folder") : t("Autre dossier…", "Other folder…"),
          other ? t("Changer…", "Change…") : t("Clé USB, disque, NAS", "USB drive, disk, NAS"),
          other,
          () => void choose(),
        )}
      </div>
      <AnimatePresence initial={false}>
        {missing && (
          <motion.div
            key={missing.kind}
            className="place-install"
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: "auto" }}
            exit={{ opacity: 0, height: 0 }}
            transition={{ duration: 0.28, ease: [0.2, 0.8, 0.2, 1] }}
          >
            <p>
              {t(
                `${missing.name} n'est pas installé sur ce Mac. Installez l'app ${missing.name} et connectez-vous : il apparaîtra ici, prêt à recevoir la sauvegarde. Lumen n'a besoin d'aucun mot de passe.`,
                `${missing.name} isn't installed on this Mac. Install the ${missing.name} app and sign in: it will show up here, ready to receive the backup. Lumen needs no password.`,
              )}
            </p>
            {isTauri && (
              <button className="btn sm soft" onClick={() => void import("@tauri-apps/plugin-opener").then((o) => o.openUrl(missing.url))}>
                {t(`Télécharger ${missing.name}`, `Download ${missing.name}`)} <Icon name="external" size={14} />
              </button>
            )}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/**
 * iPhone et iPad : un seul emplacement, le dossier « Lumen » d'iCloud Drive choisi dans
 * l'app Fichiers (celui où le Mac sauvegarde déjà ; chaque appareil y a son dossier).
 */
function IosPlace({ s, onPick }: { s: BackupStatus; onPick(dir: string): Promise<void> }) {
  const [busy, setBusy] = useState(false);
  const choose = async () => {
    setBusy(true);
    try {
      const first = !s.dir;
      const dir = await pickBackupFolder();
      if (dir) {
        await onPick(dir);
        // premier dossier choisi : la progression du Mac qui s'y trouve arrive ici
        if (first) await joinOtherDevice();
      }
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="ios-place">
      <span className="place-glyph">{busy ? <Orb size={16} /> : <Icon name="cloud" size={20} />}</span>
      <div className="grow">
        <strong>{s.dir ? placeLabel(s) : t("Le dossier « Lumen » de votre sauvegarde", "The “Lumen” folder of your backup")}</strong>
        <span>
          {s.dir
            ? t("Choisi dans l'app Fichiers. Vos autres appareils y sauvegardent aussi.", "Chosen in the Files app. Your other devices back up there too.")
            : t(
                "Celui où votre\u00a0Mac sauvegarde déjà (iCloud Drive, Google Drive, Dropbox…), choisi dans l'app Fichiers : vous y retrouverez sa progression, et celle de cet appareil y sera à l'abri.",
                "The one your\u00a0Mac already backs up to (iCloud Drive, Google Drive, Dropbox…), chosen in the Files app: you'll find its progress there, and this device's progress will be safe there too.",
              )}
        </span>
      </div>
      <button className="btn sm soft" onClick={() => void choose()} disabled={busy}>
        {s.dir ? t("Changer…", "Change…") : t("Choisir…", "Choose…")}
      </button>
    </div>
  );
}

/**
 * Réunir la progression de ses appareils : chacun ajoute à la sienne ce qu'a fait
 * l'autre (mots, leçons, avancement), par la sauvegarde.
 */
function SyncRow({ on, ready, onChange }: { on: boolean; ready: boolean; onChange(v: boolean): void }) {
  const toast = useApp((s) => s.toast);
  const [busy, setBusy] = useState(false);
  const now = async () => {
    setBusy(true);
    try {
      const r = await api().backupSync();
      if (!r || (!r.report.terms && !r.report.lessons && !r.report.progress && !r.report.removed))
        toast(t("Tout est à jour : rien de nouveau sur vos autres appareils.", "All up to date: nothing new on your other devices."));
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="set-row">
      <div className="grow">
        <strong>{t("Réunir mes appareils", "Bring my devices together")}</strong>
        <span>
          {t(
            "Ce que vous faites sur votre\u00a0Mac, votre iPhone ou votre iPad (mots appris, leçons, page atteinte) rejoint les autres par la sauvegarde, sans rien écraser. Chaque appareil doit sauvegarder au même endroit.",
            "What you do on your\u00a0Mac, your iPhone or your iPad (words learned, lessons, page reached) reaches the others through the backup, without overwriting anything. Each device must back up to the same place.",
          )}
        </span>
      </div>
      {on && ready && (
        <button className="btn sm ghost" onClick={() => void now()} disabled={busy}>
          {busy ? t("Synchronisation…", "Syncing…") : t("Synchroniser", "Sync now")}
        </button>
      )}
      <Switch on={on} onChange={onChange} label={t("Réunir mes appareils", "Bring my devices together")} />
    </div>
  );
}

/** Réglages › Sauvegarde : iCloud Drive (ou un autre nuage, un dossier choisi), médias, restauration. */
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
  // iPhone et iPad : le dossier choisi dans l'app Fichiers se lit sans autre demande
  const enabled = !!s?.enabled || (isIOS && !!s?.dir);
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
    // iPhone et iPad : le natif a déjà retenu le dossier (et son signet)
    if (!isIOS) await setSetting("backup_dir", dir);
    useBackup.setState({ list: null });
    if (s.enabled) {
      if (await save()) void load();
    } else await refresh();
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
              {t(
                `Les vidéos en ligne se retéléchargent depuis leur leçon : inutile d'en encombrer ${s.icloud ? "iCloud" : placeName(s)}.`,
                `Online videos can be downloaded again from their lesson: no need to fill ${s.icloud ? "iCloud" : placeName(s)} with them.`,
              )}
            </span>
          </div>
          <Switch on={settings.backup_video === "1"} onChange={(v) => void setOption("backup_video", v)} label={t("Sauvegarder les vidéos", "Back up videos")} />
        </div>
        <div className="set-row backup-where">
          <div className="grow">
            <strong>{t("Emplacement", "Location")}</strong>
            <span>
              {s.dir
                ? placeLabel(s)
                : isIOS
                  ? t("Choisissez ci-dessous le dossier « Lumen » où votre\u00a0Mac sauvegarde.", "Choose below the “Lumen” folder your\u00a0Mac backs up to.")
                  : isWindows
                  ? t("Choisissez où sauvegarder : OneDrive, un autre nuage ou un dossier.", "Choose where to back up: OneDrive, another cloud or a folder.")
                  : t(
                      "iCloud Drive n'est pas activé sur ce Mac (Réglages Système › votre nom › iCloud). Choisissez un autre nuage ou un dossier.",
                      "iCloud Drive isn't turned on on this Mac (System Settings › your name › iCloud). Choose another cloud or a folder.",
                    )}
            </span>
          </div>
          {s.dir && s.last_at !== null && !isIOS && (
            <button className="btn sm ghost" onClick={() => void revealBackup(s.dir!)}>
              {t("Afficher", "Show")}
            </button>
          )}
        </div>
        <div className="set-row backup-places">{isIOS ? <IosPlace s={s} onPick={moveTo} /> : <PlacePicker s={s} onPick={moveTo} />}</div>
        <SyncRow on={settings.backup_sync !== "0"} ready={!!s.dir} onChange={(v) => void setSetting("backup_sync", v ? "1" : "0")} />
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
            <span>
              {t(
                "« Fusionner » ajoute la progression d'un autre appareil à celle-ci ; « Remplacer » la prend à la place (celle de ce Mac reste en copie).",
                "“Merge” adds another device's progress to this one; “Replace” takes it instead (this Mac's progress is kept as a copy).",
              )}
            </span>
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
                <span>
                  {isIOS
                    ? t("Votre progression est sur votre\u00a0Mac ? Choisissez d'abord le dossier « Lumen » de sa sauvegarde.", "Your progress is on your\u00a0Mac? First choose the “Lumen” folder of its backup.")
                    : t("Vous avez déjà utilisé Lumen sur un autre Mac ?", "Have you used Lumen on another Mac before?")}
                </span>
              </div>
              <button className="btn sm soft" disabled={listing || !s.dir} onClick={() => void load()}>
                {listing ? t("Recherche…", "Searching…") : t("Chercher une sauvegarde", "Look for a backup")}
              </button>
            </motion.div>
          ) : listing && !list.length ? (
            <motion.div key="wait" className="set-row lingq-wait" {...enter}>
              <Orb size={18} />
              <span>{t(`Recherche dans ${s.icloud ? "votre iCloud Drive" : placeName(s)}…`, `Searching ${s.icloud ? "your iCloud Drive" : placeName(s)}…`)}</span>
            </motion.div>
          ) : listError ? (
            <motion.div key="err" className="set-row lingq-error" {...enter}>
              {listError}
            </motion.div>
          ) : !list.length ? (
            <motion.div key="none" className="set-row lingq-wait" {...enter}>
              {s.icloud
                ? t("Aucune sauvegarde dans votre iCloud Drive pour l'instant.", "No backup in your iCloud Drive yet.")
                : s.place && s.place.kind !== "folder"
                  ? t(`Aucune sauvegarde dans ${s.place.name} pour l'instant.`, `No backup in ${s.place.name} yet.`)
                  : t("Aucune sauvegarde dans ce dossier pour l'instant.", "No backup in this folder yet.")}
            </motion.div>
          ) : (
            list.map((b) => <BackupRow key={b.key} info={b} />)
          )}
        </AnimatePresence>
      </div>
    </section>
  );
}
