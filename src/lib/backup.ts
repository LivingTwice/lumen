// Sauvegarde de la progression dans iCloud Drive, un autre nuage du Mac
// (Dropbox, Google Drive, OneDrive…) ou un dossier choisi.
// L'état vit ici pour que la barre latérale, les Réglages et l'accueil le
// partagent ; les sauvegardes automatiques (événement « backup ») le tiennent à jour.
import { create } from "zustand";
import { api, errorText, isTauri } from "./api";
import { useChat } from "./chat";
import { count, isEn, locale, t } from "./i18n";
import { inFolder, isWindows, pathParts } from "./platform";
import { formatBytes, formatNumber, useApp } from "./store";
import type { BackupInfo, BackupPlace, BackupRestored, BackupStatus } from "./types";

export function restoreStage(stage: string): string | undefined {
  return {
    download: t("Lecture de la sauvegarde…", "Reading the backup…"),
    media: t("Copie de l'audio et des couvertures…", "Copying audio and covers…"),
    apply: t("Mise en place de votre progression…", "Setting up your progress…"),
  }[stage];
}

interface BackupState {
  status: BackupStatus | null;
  /** sauvegarde demandée à la main (bouton, activation) */
  saving: boolean;
  /** sauvegardes trouvées ; null : pas encore cherchées */
  list: BackupInfo[] | null;
  listing: boolean;
  listError: string;
  restoring: { key: string; stage: string; value: number } | null;
  /** dernière restauration échouée (l'accueil n'affiche pas les notifications) */
  restoreError: string;
  /** nuages installés sur ce Mac ; null : pas encore lus */
  places: BackupPlace[] | null;
  refresh(): Promise<void>;
  loadPlaces(): Promise<void>;
  /** active ou coupe la sauvegarde automatique ; l'activation sauvegarde tout de suite */
  enable(on: boolean): Promise<boolean>;
  save(): Promise<boolean>;
  load(): Promise<void>;
  restore(info: BackupInfo, day: string | null): Promise<BackupRestored | null>;
}

export const useBackup = create<BackupState>((set, get) => ({
  status: null,
  saving: false,
  list: null,
  listing: false,
  listError: "",
  restoring: null,
  restoreError: "",
  places: null,

  async loadPlaces() {
    try {
      set({ places: await api().backupPlaces() });
    } catch {
      if (!get().places) set({ places: [] });
    }
  },

  async refresh() {
    try {
      set({ status: await api().backupStatus() });
    } catch {
      /* état indisponible : on garde le précédent */
    }
  },

  async enable(on) {
    await useApp.getState().setSetting("backup_on", on ? "1" : "0");
    if (!on) {
      await get().refresh();
      return true;
    }
    const ok = await get().save();
    if (ok) void get().load();
    return ok;
  },

  async save() {
    if (get().saving) return false;
    set({ saving: true });
    try {
      set({ status: await api().backupRun() });
      return true;
    } catch (e) {
      useApp.getState().toast(errorText(e), "error");
      await get().refresh();
      return false;
    } finally {
      set({ saving: false });
    }
  },

  async load() {
    if (get().listing) return;
    set({ listing: true, listError: "" });
    try {
      set({ list: await api().backupList() });
    } catch (e) {
      set({ list: [], listError: errorText(e) });
    } finally {
      set({ listing: false });
    }
  },

  async restore(info, day) {
    if (get().restoring) return null;
    set({ restoring: { key: info.key, stage: "download", value: 0 }, restoreError: "" });
    try {
      return await api().backupRestore(info.key, day, (e) =>
        set((s) => (s.restoring ? { restoring: e.type === "stage" ? { ...s.restoring, stage: e.stage, value: 0 } : { ...s.restoring, value: e.value } } : {})),
      );
    } catch (e) {
      set({ restoreError: errorText(e) });
      useApp.getState().toast(errorText(e), "error");
      return null;
    } finally {
      set({ restoring: null });
    }
  },
}));

/** Suit les sauvegardes automatiques (appelé une fois au démarrage). */
export function startBackupEvents() {
  void useBackup.getState().refresh();
  void api()
    .backupListen((status) => useBackup.setState({ status }))
    .catch(() => {});
}

/** Après une restauration : toute l'application relit la base. */
export async function reloadProgress() {
  useChat.setState({ lang: null, list: [], thread: null, draftLesson: null, reading: null, error: null });
  const app = useApp.getState();
  await app.init();
  app.go("library");
  app.bumpLibrary();
  useBackup.setState({ list: null });
  await useBackup.getState().refresh();
}

/** Message de fin de restauration. */
export function restoredText(r: BackupRestored): string {
  const lessons = count(r.counts.lessons, "leçon", "leçons", "lesson", "lessons");
  return t(
    `Progression retrouvée : ${formatNumber(r.counts.known)} mots connus, ${lessons}`,
    `Progress restored: ${formatNumber(r.counts.known)} known words, ${lessons}`,
  );
}

/** « à l'instant », « il y a 4 min », « hier à 18:05 », « le 3 octobre à 9:12 » (« just now », « 4 min ago »…). */
export function formatWhen(ts: number): string {
  const d = new Date(ts * 1000);
  const now = new Date();
  const secs = (now.getTime() - d.getTime()) / 1000;
  if (secs < 60) return t("à l'instant", "just now");
  if (secs < 3600) return t(`il y a ${Math.floor(secs / 60)} min`, `${Math.floor(secs / 60)} min ago`);
  const time = d.toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit" });
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (d.toDateString() === now.toDateString()) return t(`aujourd'hui à ${time}`, `today at ${time}`);
  if (d.toDateString() === yesterday.toDateString()) return t(`hier à ${time}`, `yesterday at ${time}`);
  return t(`le ${frenchDate(d, d.getFullYear() !== now.getFullYear())} à ${time}`, `on ${frenchDate(d, d.getFullYear() !== now.getFullYear())} at ${time}`);
}

/** « 1er octobre », « 12 mars 2025 » (« October 1 », « March 12, 2025 » en anglais). */
export function frenchDate(d: Date, year = false): string {
  const s = d.toLocaleDateString(locale(), { day: "numeric", month: "long", ...(year ? { year: "numeric" } : {}) });
  return !isEn() && d.getDate() === 1 ? s.replace(/^1 /, "1er ") : s;
}

/** Version d'un jour précédent (« 2026-10-01 ») : « jeudi 1er octobre » (« Thursday, October 1 »). */
export function dayLabel(day: string): string {
  const d = new Date(`${day}T12:00:00`);
  const weekday = d.toLocaleDateString(locale(), { weekday: "long" });
  return t(`${weekday} ${frenchDate(d, d.getFullYear() !== new Date().getFullYear())}`, `${weekday}, ${frenchDate(d, d.getFullYear() !== new Date().getFullYear())}`);
}

/** Un nuage (iCloud Drive, Dropbox…), et non un disque ou un dossier ordinaire. */
export function isCloud(p: BackupPlace | undefined): boolean {
  return !!p && p.kind !== "drive" && p.kind !== "folder";
}

/** « iCloud Drive », « Dropbox », « Google Drive » ; « le dossier choisi » pour un dossier ordinaire. */
export function placeName(s: BackupStatus): string {
  if (s.icloud) return "iCloud Drive";
  if (isCloud(s.place) || s.place?.kind === "drive") return s.place.name;
  return t("le dossier choisi", "the chosen folder");
}

/** Où en est l'envoi vers le nuage (vide quand il n'y a rien d'utile à dire). */
export function cloudText(s: BackupStatus): string {
  const name = s.icloud ? "iCloud" : (s.place?.name ?? "");
  switch (s.cloud) {
    case "uploaded":
      return t(`dans ${name}`, `in ${name}`);
    case "uploading":
      return t(`envoi vers ${name}…`, `uploading to ${name}…`);
    case "waiting":
      return t(`en attente d'envoi vers ${name}`, `waiting to upload to ${name}`);
    case "error":
      return t(`${name} : ${s.cloud_error ?? "envoi impossible"}`, `${name}: ${s.cloud_error ?? "upload failed"}`);
    case "local":
      // dossier que l'app du service envoie elle-même (anciennes versions de Dropbox…)
      if (s.icloud) return "";
      if (s.place?.kind === "drive") return t(`sur ${name}`, `on ${name}`);
      return isCloud(s.place) ? t(`dans le dossier ${name}`, `in the ${name} folder`) : t("dans le dossier choisi", "in the chosen folder");
    default:
      return "";
  }
}

/** Ligne d'état sous « Sauvegarde automatique ». */
export function statusLine(s: BackupStatus, saving: boolean): string {
  if (saving || s.running) return t("Sauvegarde en cours…", "Backing up…");
  if (s.error) return s.error;
  if (!s.enabled)
    return s.decided
      ? t("Désactivée : votre progression ne vit que sur ce Mac.", "Off: your progress only lives on this Mac.")
      : t("Activez-la pour mettre votre progression à l'abri.", "Turn it on to keep your progress safe.");
  if (!s.dir)
    return isWindows
      ? t("Choisissez où sauvegarder : OneDrive, un autre nuage ou un dossier.", "Choose where to back up: OneDrive, another cloud or a folder.")
      : t("iCloud Drive n'est pas activé sur ce Mac : choisissez un dossier.", "iCloud Drive isn't turned on on this Mac: choose a folder.");
  if (s.last_at === null) return t("Activée : la première copie se fera dès votre première lecture.", "On: the first copy will be made as soon as you start reading.");
  const parts = [t(`Sauvegardée ${formatWhen(s.last_at)}`, `Backed up ${formatWhen(s.last_at)}`), formatBytes(s.size + s.media_size)];
  const cloud = cloudText(s);
  if (cloud) parts.push(cloud);
  return parts.join(" · ");
}

/** Emplacement lisible : « iCloud Drive › Lumen », « Google Drive › Mon Drive › Lumen ». */
export function placeLabel(s: BackupStatus): string {
  if (!s.dir) return t("Aucun", "None");
  if (s.icloud) return "iCloud Drive › Lumen";
  const p = s.place;
  if (p && (isCloud(p) || p.kind === "drive") && inFolder(s.dir, p.path) && s.dir.length > p.path.length) {
    return [p.name, ...pathParts(s.dir.slice(p.path.length))].join(" › ");
  }
  // dans le dossier personnel (« /Users/léa », « C:\Users\léa ») : à partir de lui
  const home = s.dir.match(/^(?:[A-Za-z]:)?[\\/]Users[\\/][^\\/]+/)?.[0];
  const rel = home ? s.dir.slice(home.length + 1) : s.dir;
  return pathParts(rel).slice(-3).join(" › ");
}

/** Dossier à retenir pour un nuage du Mac ("" : iCloud Drive ; null : impossible, déjà signalé). */
export async function placeDir(p: BackupPlace): Promise<string | null> {
  if (p.kind === "icloud") return "";
  try {
    return await api().backupPlaceDir(p.path);
  } catch (e) {
    useApp.getState().toast(errorText(e), "error");
    return null;
  }
}

/** Ce nuage est-il celui de la sauvegarde ? */
export function isPlaceOf(s: BackupStatus, p: BackupPlace): boolean {
  if (p.kind === "icloud") return s.icloud;
  return !s.icloud && !!s.dir && inFolder(s.dir, p.path);
}

/** Choisit un autre dossier (clé USB, disque, NAS, ou un nuage qui n'est pas proposé). */
export async function pickBackupFolder(): Promise<string | null> {
  if (!isTauri) return isWindows ? t("C:\\Users\\vous\\Dropbox", "C:\\Users\\you\\Dropbox") : t("/Users/vous/Dropbox", "/Users/you/Dropbox");
  const { open } = await import("@tauri-apps/plugin-dialog");
  const res = await open({ directory: true, multiple: false, title: t("Dossier de sauvegarde de Lumen", "Lumen backup folder") });
  return typeof res === "string" ? res : null;
}

/** Montre le dossier dans le Finder. */
export async function revealBackup(dir: string) {
  if (!isTauri) return;
  const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
  await revealItemInDir(dir).catch(() => {});
}
