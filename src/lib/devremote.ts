// Télécommande de développement : voir scripts/vite-remote.ts. Chargée seulement
// par `npm run dev` et `tauri ios dev` (import.meta.env.DEV), jamais dans une version compilée.
import { api } from "./api";
import { useApp } from "./store";

export function startRemote() {
  // sur iPhone, la page vient de tauri://localhost : le serveur de Vite reste sur ce Mac
  const base = window.location.origin.startsWith("http") ? window.location.origin : "http://localhost:1420";
  const tick = async () => {
    try {
      const r = await fetch(`${base}/__remote/poll`, { cache: "no-store" });
      const job = (await r.json()) as { id: number; code: string } | null;
      if (job) {
        let out: { id: number; ok: boolean; value?: unknown; error?: string };
        try {
          const fn = new Function("api", "useApp", `return (async () => { ${job.code} })()`);
          const value = await fn(api(), useApp);
          out = { id: job.id, ok: true, value: value === undefined ? null : JSON.parse(JSON.stringify(value)) };
        } catch (e) {
          out = { id: job.id, ok: false, error: e instanceof Error ? `${e.message}\n${e.stack ?? ""}` : String(e) };
        }
        await fetch(`${base}/__remote/result`, { method: "POST", body: JSON.stringify(out) });
      }
      window.setTimeout(tick, 50);
    } catch {
      // serveur arrêté : on réessaie plus tard
      window.setTimeout(tick, 3000);
    }
  };
  void tick();
}
