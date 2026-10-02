import { isTauri } from "./api";

/** Demande de confirmation native (boîte de dialogue macOS dans l'app). */
export async function confirmAsk(message: string, title = "Lumen", okLabel = "Confirmer"): Promise<boolean> {
  if (isTauri) {
    const { ask } = await import("@tauri-apps/plugin-dialog");
    return ask(message, { title, kind: "warning", okLabel, cancelLabel: "Annuler" });
  }
  return window.confirm(message);
}

export async function pickFiles(filters: { name: string; extensions: string[] }[], multiple = true): Promise<string[]> {
  if (!isTauri) return [];
  const { open } = await import("@tauri-apps/plugin-dialog");
  const res = await open({ multiple, filters });
  if (!res) return [];
  return Array.isArray(res) ? res : [res];
}

export async function pickSavePath(defaultPath: string): Promise<string | null> {
  if (!isTauri) return null;
  const { save } = await import("@tauri-apps/plugin-dialog");
  return save({ defaultPath, filters: [{ name: "CSV", extensions: ["csv"] }] });
}
