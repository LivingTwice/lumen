// Rend les visuels de design/ et les installe dans l'app :
//   design/icon.mjs → icon.svg, icon-small.svg → design/icon-1024.png → src-tauri/icons/ (toutes les tailles)
//   design/dmg-background.html → src-tauri/dmg/background.png (2x, 144 dpi : net sur un écran Retina)
// Rendu par un Chromium sans fenêtre : celui de Playwright s'il est installé, sinon Google Chrome.
import { execFileSync, spawn } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, statSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const design = join(root, "design");
const icons = join(root, "src-tauri", "icons");
const tmp = mkdtempSync(join(tmpdir(), "lumen-design-"));

function chromium() {
  const pw = join(homedir(), "Library/Caches/ms-playwright");
  if (existsSync(pw)) {
    for (const d of readdirSync(pw).filter((d) => d.startsWith("chromium_headless_shell-")).sort().reverse()) {
      const p = join(pw, d, "chrome-headless-shell-mac-arm64", "chrome-headless-shell");
      if (existsSync(p)) return p;
    }
  }
  const chrome = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
  if (existsSync(chrome)) return chrome;
  throw new Error("Aucun Chromium trouvé : installez Google Chrome, ou `npx playwright install chromium-headless-shell`.");
}

async function render(file, out, w, h, scale = 1) {
  rmSync(out, { force: true });
  const profile = mkdtempSync(join(tmp, "chrome-"));
  const child = spawn(
    chromium(),
    [
      "--headless", "--hide-scrollbars", "--no-first-run", "--allow-file-access-from-files",
      `--user-data-dir=${profile}`, "--default-background-color=00000000",
      `--force-device-scale-factor=${scale}`, `--window-size=${w},${h}`, `--screenshot=${out}`,
      pathToFileURL(file).href,
    ],
    { stdio: "ignore" },
  );
  // Google Chrome ne se ferme pas toujours après la capture : on attend que le fichier soit écrit
  let last = -1;
  for (let i = 0; i < 200 && child.exitCode === null; i++) {
    await new Promise((r) => setTimeout(r, 300));
    const size = existsSync(out) ? statSync(out).size : 0;
    if (size > 0 && size === last) break;
    last = size;
  }
  child.kill();
  if (!existsSync(out)) throw new Error(`rendu impossible : ${file}`);
}

const sips = (...args) => execFileSync("sips", args.map(String), { stdio: "ignore" });

// ---------- icône ----------
console.log("› Icône");
execFileSync(process.execPath, [join(design, "icon.mjs")]);
const big = join(design, "icon-1024.png");
const small = join(tmp, "icon-small-1024.png");
await render(join(design, "icon.svg"), big, 1024, 1024);
await render(join(design, "icon-small.svg"), small, 1024, 1024);

// toutes les tailles par Tauri (PNG, ICO, ICNS) à part, puis seulement les fichiers que l'app utilise déjà
const out = join(tmp, "icons");
execFileSync("npx", ["tauri", "icon", big, "-o", out], { cwd: root, stdio: "ignore" });
for (const f of readdirSync(icons)) {
  const src = join(out, f);
  if (existsSync(src) && statSync(src).isFile()) copyFileSync(src, join(icons, f));
}

// icns refait à la main : la version simplifiée pour 16 et 32 px, plus lisible que la réduction de la grande
const set = join(tmp, "Lumen.iconset");
mkdirSync(set);
for (const s of [16, 32, 128, 256, 512]) {
  for (const k of [1, 2]) {
    const px = s * k;
    sips("-z", px, px, px <= 32 ? small : big, "--out", join(set, `icon_${s}x${s}${k === 2 ? "@2x" : ""}.png`));
  }
}
execFileSync("iconutil", ["-c", "icns", set, "-o", join(icons, "icon.icns")]);
sips("-z", 32, 32, small, "--out", join(icons, "32x32.png"));
console.log("✓ src-tauri/icons/");

// ---------- fond du DMG ----------
console.log("› Fond du DMG");
const bg = join(tmp, "background.png");
await render(join(design, "dmg-background.html"), bg, 660, 420, 2);
sips("-s", "dpiWidth", 144, "-s", "dpiHeight", 144, bg, "--out", join(root, "src-tauri", "dmg", "background.png"));
console.log("✓ src-tauri/dmg/background.png");

rmSync(tmp, { recursive: true, force: true });
