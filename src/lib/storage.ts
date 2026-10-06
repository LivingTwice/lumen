import { create } from "zustand";
import { api, errorText } from "./api";
import { count, t } from "./i18n";
import { formatBytes, useApp } from "./store";
import type { LightenProgress, MediaUsage } from "./types";

/** Réglage `media_quality` : ce que Lumen fait des vidéos et des sons importés. */
export type MediaQuality = "original" | "balanced" | "compact";

export function mediaQuality(v: string | undefined): MediaQuality {
  return v === "original" || v === "compact" ? v : "balanced";
}

interface StorageStore {
  /** place occupée, ce qui peut encore s'alléger (null : pas encore calculé) */
  usage: MediaUsage | null;
  /** allègement des leçons déjà importées en cours (null : rien) */
  run: LightenProgress | null;
  busy: boolean;
  refresh(): Promise<void>;
  lighten(): Promise<void>;
  stop(): void;
}

/**
 * Stockage : place occupée sur ce Mac et allègement des leçons déjà importées.
 * L'allègement continue si l'on quitte la page ; une notification en donne le bilan.
 */
export const useStorage = create<StorageStore>((set, get) => ({
  usage: null,
  run: null,
  busy: false,

  async refresh() {
    try {
      set({ usage: await api().mediaUsage() });
    } catch {
      // la page reste sur l'ancien calcul
    }
  },

  async lighten() {
    if (get().busy) return;
    const toast = useApp.getState().toast;
    set({ busy: true, run: null });
    try {
      const r = await api().mediaLighten((e) => set({ run: e }));
      if (r.saved > 0) toast(t(`${formatBytes(r.saved)} libérés sur ce Mac`, `${formatBytes(r.saved)} freed on this Mac`), "light");
      else if (!r.cancelled) toast(t("Rien de plus à alléger.", "Nothing more to lighten."));
      if (r.failed)
        toast(
          t(
            `${count(r.failed, "fichier n'a", "fichiers n'ont", "", "")} pas pu être allégé${r.failed > 1 ? "s" : ""} : ${r.failed > 1 ? "ils restent" : "il reste"} tel${r.failed > 1 ? "s" : ""} quel${r.failed > 1 ? "s" : ""}.`,
            `${count(r.failed, "", "", "file", "files")} couldn't be lightened and stay as they were.`,
          ),
        );
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      set({ busy: false, run: null });
      void get().refresh();
    }
  },

  stop() {
    void api().mediaLightenCancel();
  },
}));
