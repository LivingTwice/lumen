/**
 * Système de l'app : Mac, Windows ou Linux. Il décide de la fenêtre (boutons de
 * la fenêtre, barre de titre), des raccourcis (⌘ ou Ctrl), des chemins de
 * fichiers et de quelques mots (« ce Mac » devient « ce PC » sous Windows,
 * « cet ordinateur » sous Linux, voir `forPc`).
 *
 * Windows et Linux partent de la même allure : boutons de fenêtre dessinés par
 * Lumen, raccourcis Ctrl, pas de barre des menus (`isPc`).
 *
 * Dans le navigateur (`npm run dev`), on peut essayer l'allure d'un autre
 * système : http://localhost:1420/?platform=windows (ou ?platform=linux), ou
 *   localStorage.setItem("lumen-platform", "linux") puis recharger.
 */

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export type Platform = "mac" | "windows" | "linux";

function detect(): Platform {
  if (typeof navigator === "undefined") return "mac";
  if (!inTauri) {
    try {
      const forced = new URLSearchParams(location.search).get("platform") ?? localStorage.getItem("lumen-platform");
      if (forced === "windows" || forced === "linux") return forced;
    } catch {
      // stockage indisponible : on suit le navigateur
    }
  }
  if (/Windows/i.test(navigator.userAgent)) return "windows";
  if (/Linux/i.test(navigator.userAgent) && !/Android/i.test(navigator.userAgent)) return "linux";
  return "mac";
}

/** Système sur lequel l'app tourne. */
export const platform: Platform = detect();

/**
 * Formats d'image que le moteur de cette fenêtre lit, en plus du H.264 (« vp9 »,
 * « av1 ») : les vidéos en ligne arrivent dans le plus léger (`media::video_format`).
 * WebKit (Mac) lit le VP9 sur les macOS récents, l'AV1 seulement avec une puce M3 ou
 * plus récente ; WebView2 (Windows) lit les deux ; WebKitGTK (Linux) selon GStreamer.
 */
export function playableCodecs(): string[] {
  if (typeof document === "undefined") return [];
  const v = document.createElement("video");
  const sure = (type: string) => v.canPlayType(type) === "probably";
  const out: string[] = [];
  if (sure('video/webm; codecs="vp9"')) out.push("vp9");
  if (sure('video/mp4; codecs="av01.0.08M.08"')) out.push("av1");
  return out;
}

/** L'app tourne sous Windows. */
export const isWindows = platform === "windows";

/** L'app tourne sous Linux (Wayland ou X11). */
export const isLinux = platform === "linux";

/** Fenêtre sans barre de titre native et raccourcis Ctrl : Windows et Linux. */
export const isPc = platform !== "mac";

/** Touche des raccourcis, telle qu'on l'écrit : ⌘ sur Mac, Ctrl ailleurs. */
export const modKey = isPc ? "Ctrl" : "⌘";

/** La touche des raccourcis (⌘ sur Mac, Ctrl ailleurs) est-elle enfoncée ? */
export function modDown(e: { metaKey: boolean; ctrlKey: boolean }): boolean {
  return isPc ? e.ctrlKey && !e.metaKey : e.metaKey;
}

/** Séparateur des chemins de fichiers. */
export const pathSep = isWindows ? "\\" : "/";

/** Chemin d'un fichier dans un dossier, avec le séparateur du système. */
export function joinPath(dir: string, name: string): string {
  return dir.endsWith("/") || dir.endsWith("\\") ? dir + name : `${dir}${pathSep}${name}`;
}

/** Morceaux d'un chemin, quel que soit le séparateur. */
export function pathParts(path: string): string[] {
  return path.split(/[\\/]/).filter(Boolean);
}

/** Le chemin `path` est-il dans le dossier `dir` (ou ce dossier lui-même) ? */
export function inFolder(path: string, dir: string): boolean {
  if (!dir) return false;
  const norm = (p: string) => (isWindows ? p.replace(/\//g, "\\").toLowerCase() : p).replace(/[\\/]+$/, "");
  const a = norm(path);
  const b = norm(dir);
  return a === b || a.startsWith(b + pathSep);
}

// ---------- « ce Mac » devient « ce PC » (ou « cet ordinateur ») ----------

// « ce Mac », « votre Mac », « un nouveau Mac »… mais pas « Mac mini » ni « MacBook »
// (noms d'appareils glissés dans une phrase, « sauvegardée sur Mac mini »)
const MAC_FR = /\b(ce|Ce|votre|Votre|du|le|Le|un|nouveau|autre|les|Les|chaque|application|sur|petit) Mac\b(?! (?:mini|Studio|Pro)\b)/g;
const MAC_EN = /\b(this|This|your|Your|the|The|a|new|another|every|each|on|small|other) Mac\b(?! (?:mini|Studio|Pro)\b)/g;

// « ce Mac » → « cet ordinateur » (l'élision ne joue que devant une voyelle)
const ORDINATEUR = (_m: string, mot: string): string => (mot === "ce" ? "cet" : mot === "Ce" ? "Cet" : mot) + " ordinateur";

type Replacement = string | ((substring: string, ...groups: string[]) => string);

const SWAPS_WINDOWS: [RegExp, Replacement][] = [
  [MAC_FR, "$1 PC"],
  [MAC_EN, "$1 PC"],
  [/\bMacs\b/g, "PCs"],
  [/\bdans le Finder\b/g, "dans l'Explorateur de fichiers"],
  [/\bin Finder\b/g, "in File Explorer"],
  [/\bRéglages Système\b/g, "Paramètres de Windows"],
  [/\bSystem Settings\b/g, "Windows Settings"],
  [/\bmacOS\b/g, "Windows"],
  [/⌘ ?/g, "Ctrl+"],
];

const SWAPS_LINUX: [RegExp, Replacement][] = [
  [MAC_FR, ORDINATEUR],
  [MAC_EN, "$1 computer"],
  [/\bMacs\b/g, "computers"],
  [/\bdans le Finder\b/g, "dans le gestionnaire de fichiers"],
  [/\bin Finder\b/g, "in the file manager"],
  [/\bmacOS\b/g, "Linux"],
  [/⌘ ?/g, "Ctrl+"],
];

/**
 * Sous Windows ou Linux, un texte écrit pour le Mac parle de cet ordinateur :
 * « Calculé sur votre Mac » devient « Calculé sur votre PC » (Windows) ou
 * « Calculé sur votre ordinateur » (Linux). Appliqué par `t()` ; sur Mac, rien
 * ne change.
 */
export function forPc(s: string): string {
  if (!isPc || !/Mac|Finder|Système|System Settings|macOS|⌘/.test(s)) return s;
  const swaps = platform === "windows" ? SWAPS_WINDOWS : SWAPS_LINUX;
  return swaps.reduce((out, [re, by]) => (typeof by === "string" ? out.replace(re, by) : out.replace(re, by)), s);
}
