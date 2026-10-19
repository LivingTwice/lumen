// Télécommande de développement (seulement `npm run dev`, `tauri ios dev`) : un
// script envoyé au serveur de Vite s'exécute dans la fenêtre de Lumen, simulateur
// iPhone compris, et son résultat revient. Pour essayer les fonctions natives
// sans toucher l'écran :
//   curl -s localhost:1420/__remote/run --data 'return document.title'
// Le code reçoit `api` et `useApp` (voir src/lib/devremote.ts). Rien de tout cela
// n'existe dans une version compilée.
import type { Plugin } from "vite";

interface Job {
  id: number;
  code: string;
  done(result: string): void;
}

export function remote(): Plugin {
  const queue: Job[] = [];
  const waiting: ((job: Job | null) => void)[] = [];
  const pending = new Map<number, Job>();
  let next = 1;
  const body = (req: NodeJS.ReadableStream) =>
    new Promise<string>((resolve) => {
      let s = "";
      req.on("data", (c) => (s += c));
      req.on("end", () => resolve(s));
    });
  return {
    name: "lumen-remote",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use(async (req, res, next_) => {
        const url = req.url ?? "";
        if (!url.startsWith("/__remote/")) return next_();
        res.setHeader("Access-Control-Allow-Origin", "*");
        res.setHeader("Access-Control-Allow-Headers", "*");
        if (req.method === "OPTIONS") return res.end();
        if (url.startsWith("/__remote/run")) {
          // seulement depuis ce Mac (pour un iPhone branché, Vite écoute aussi le réseau local),
          // et jamais d'une page web : un navigateur annonce toujours son origine, curl jamais
          const from = req.socket.remoteAddress ?? "";
          if (!["127.0.0.1", "::1", "::ffff:127.0.0.1"].includes(from) || req.headers.origin) {
            res.statusCode = 403;
            return res.end();
          }
          const code = await body(req);
          const job: Job = {
            id: next++,
            code,
            done: (r) => {
              res.setHeader("Content-Type", "application/json");
              res.end(r);
            },
          };
          pending.set(job.id, job);
          const w = waiting.shift();
          if (w) w(job);
          else queue.push(job);
          // pas de réponse en 10 minutes : la fenêtre n'écoute pas
          setTimeout(() => {
            if (pending.delete(job.id)) job.done(JSON.stringify({ ok: false, error: "timeout: no window listening" }));
          }, 600_000);
          return;
        }
        if (url.startsWith("/__remote/poll")) {
          const job = queue.shift();
          let sent = false;
          const send = (j: Job | null) => {
            if (sent || res.writableEnded) {
              // la fenêtre est partie entre-temps : le script attend la suivante
              if (j) queue.unshift(j);
              return;
            }
            sent = true;
            res.setHeader("Content-Type", "application/json");
            res.end(JSON.stringify(j ? { id: j.id, code: j.code } : null));
          };
          if (job) return send(job);
          const wake = (j: Job | null) => {
            clearTimeout(timer);
            send(j);
          };
          const timer = setTimeout(() => {
            const i = waiting.indexOf(wake);
            if (i >= 0) waiting.splice(i, 1);
            send(null);
          }, 25_000);
          waiting.push(wake);
          res.on("close", () => {
            const i = waiting.indexOf(wake);
            if (i >= 0) waiting.splice(i, 1);
            clearTimeout(timer);
          });
          return;
        }
        if (url.startsWith("/__remote/result")) {
          const r = JSON.parse((await body(req)) || "{}");
          const job = pending.get(r.id);
          if (job) {
            pending.delete(r.id);
            job.done(JSON.stringify(r));
          }
          return res.end("ok");
        }
        next_();
      });
    },
  };
}
