import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { remote } from "./scripts/vite-remote.ts";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  // remote : télécommande de développement (scripts/vite-remote.ts), jamais dans une version compilée
  plugins: [react(), remote()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: "safari16",
    minify: !process.env.TAURI_ENV_DEBUG,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    chunkSizeWarningLimit: 2500,
  },
});
