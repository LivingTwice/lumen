// Mises à jour automatiques : Lumen consulte la dernière version publiée,
// la télécharge (paquet signé) puis redémarre sur la nouvelle version.
import { create } from "zustand";
import { isTauri } from "./api";

type Phase = "idle" | "checking" | "available" | "downloading" | "ready" | "uptodate" | "error";

interface UpdateState {
  phase: Phase;
  version: string;
  notes: string;
  progress: number; // 0 à 1
  error: string;
  dismissed: boolean;
  check(manual?: boolean): Promise<void>;
  install(): Promise<void>;
  restart(): Promise<void>;
  dismiss(): void;
}

// objet Update du greffon (gardé hors de l'état pour ne pas le sérialiser)
let pending: { downloadAndInstall(cb: (e: { event: string; data?: { contentLength?: number; chunkLength?: number } }) => void): Promise<void> } | null =
  null;

export const useUpdate = create<UpdateState>((set, get) => ({
  phase: "idle",
  version: "",
  notes: "",
  progress: 0,
  error: "",
  dismissed: false,

  async check(manual = false) {
    if (!isTauri) {
      if (manual) set({ phase: "uptodate" });
      return;
    }
    if (["checking", "downloading", "ready"].includes(get().phase)) return;
    set({ phase: "checking", error: "" });
    try {
      const { check } = await import("@tauri-apps/plugin-updater");
      const update = await check();
      if (update) {
        pending = update as unknown as typeof pending;
        set({ phase: "available", version: update.version, notes: update.body ?? "", dismissed: false });
      } else {
        set({ phase: manual ? "uptodate" : "idle" });
      }
    } catch (e) {
      set({ phase: manual ? "error" : "idle", error: String(e) });
    }
  },

  async install() {
    if (!pending) return;
    set({ phase: "downloading", progress: 0 });
    let total = 0;
    let got = 0;
    try {
      await pending.downloadAndInstall((e) => {
        if (e.event === "Started") total = e.data?.contentLength ?? 0;
        else if (e.event === "Progress") {
          got += e.data?.chunkLength ?? 0;
          set({ progress: total ? Math.min(1, got / total) : 0 });
        } else if (e.event === "Finished") set({ progress: 1 });
      });
      set({ phase: "ready" });
    } catch (e) {
      set({ phase: "error", error: String(e) });
    }
  },

  async restart() {
    const { relaunch } = await import("@tauri-apps/plugin-process");
    await relaunch();
  },

  dismiss() {
    set({ dismissed: true });
  },
}));

/** Vérifie au démarrage puis toutes les six heures. */
export function startUpdateChecks() {
  if (!isTauri) return;
  window.setTimeout(() => void useUpdate.getState().check(), 8000);
  window.setInterval(() => void useUpdate.getState().check(), 6 * 3600 * 1000);
}
