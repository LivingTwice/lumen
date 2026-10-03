// Sauvegarde de la progression dans iCloud Drive (ou un dossier choisi).
// L'état vit ici pour que la barre latérale, les Réglages et l'accueil le
// partagent ; les sauvegardes automatiques (événement « backup ») le tiennent à jour.
import { create } from "zustand";
import { api, errorText, isTauri } from "./api";
import { useChat } from "./chat";
import { formatBytes, formatNumber, useApp } from "./store";
import type { BackupInfo, BackupRestored, BackupStatus } from "./types";

export const RESTORE_STAGE: Record<string, string> = {
  download: "Lecture de la sauvegarde…",
  media: "Copie de l'audio et des couvertures…",
  apply: "Mise en place de votre progression…",
};

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
  refresh(): Promise<void>;
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
  const lessons = `${formatNumber(r.counts.lessons)} leçon${r.counts.lessons > 1 ? "s" : ""}`;
  return `Progression retrouvée : ${formatNumber(r.counts.known)} mots connus, ${lessons}`;
}

/** « à l'instant », « il y a 4 min », « hier à 18:05 », « le 3 octobre à 9:12 ». */
export function formatWhen(ts: number): string {
  const d = new Date(ts * 1000);
  const now = new Date();
  const secs = (now.getTime() - d.getTime()) / 1000;
  if (secs < 60) return "à l'instant";
  if (secs < 3600) return `il y a ${Math.floor(secs / 60)} min`;
  const time = d.toLocaleTimeString("fr-FR", { hour: "2-digit", minute: "2-digit" });
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (d.toDateString() === now.toDateString()) return `aujourd'hui à ${time}`;
  if (d.toDateString() === yesterday.toDateString()) return `hier à ${time}`;
  return `le ${frenchDate(d, d.getFullYear() !== now.getFullYear())} à ${time}`;
}

/** « 1er octobre », « 12 mars 2025 ». */
export function frenchDate(d: Date, year = false): string {
  const s = d.toLocaleDateString("fr-FR", { day: "numeric", month: "long", ...(year ? { year: "numeric" } : {}) });
  return d.getDate() === 1 ? s.replace(/^1 /, "1er ") : s;
}

/** Version d'un jour précédent (« 2026-10-01 ») : « jeudi 1er octobre ». */
export function dayLabel(day: string): string {
  const d = new Date(`${day}T12:00:00`);
  const weekday = d.toLocaleDateString("fr-FR", { weekday: "long" });
  return `${weekday} ${frenchDate(d, d.getFullYear() !== new Date().getFullYear())}`;
}

/** Où en est l'envoi vers le nuage (vide quand il n'y a rien d'utile à dire). */
export function cloudText(s: BackupStatus): string {
  switch (s.cloud) {
    case "uploaded":
      return "dans iCloud";
    case "uploading":
      return "envoi vers iCloud…";
    case "waiting":
      return "en attente d'envoi vers iCloud";
    case "error":
      return `iCloud : ${s.cloud_error ?? "envoi impossible"}`;
    case "local":
      return s.icloud ? "" : "dans le dossier choisi";
    default:
      return "";
  }
}

/** Ligne d'état sous « Sauvegarde automatique ». */
export function statusLine(s: BackupStatus, saving: boolean): string {
  if (saving || s.running) return "Sauvegarde en cours…";
  if (s.error) return s.error;
  if (!s.enabled) return s.decided ? "Désactivée : votre progression ne vit que sur ce Mac." : "Activez-la pour mettre votre progression à l'abri.";
  if (!s.dir) return "iCloud Drive n'est pas activé sur ce Mac : choisissez un dossier.";
  if (s.last_at === null) return "Activée : la première copie se fera dès votre première lecture.";
  const parts = [`Sauvegardée ${formatWhen(s.last_at)}`, formatBytes(s.size + s.media_size)];
  const cloud = cloudText(s);
  if (cloud) parts.push(cloud);
  return parts.join(" · ");
}

/** Emplacement lisible : « iCloud Drive › Lumen », « Dropbox › Lumen ». */
export function placeLabel(s: BackupStatus): string {
  if (!s.dir) return "Aucun";
  if (s.icloud) return "iCloud Drive › Lumen";
  const home = s.dir.match(/^\/Users\/[^/]+/)?.[0];
  const rel = home ? s.dir.slice(home.length + 1) : s.dir;
  return rel.split("/").filter(Boolean).slice(-3).join(" › ");
}

/** Choisit un autre dossier (Dropbox, Google Drive, clé USB…). */
export async function pickBackupFolder(): Promise<string | null> {
  if (!isTauri) return "/Users/vous/Dropbox";
  const { open } = await import("@tauri-apps/plugin-dialog");
  const res = await open({ directory: true, multiple: false, title: "Dossier de sauvegarde de Lumen" });
  return typeof res === "string" ? res : null;
}

/** Montre le dossier dans le Finder. */
export async function revealBackup(dir: string) {
  if (!isTauri) return;
  const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
  await revealItemInDir(dir).catch(() => {});
}
