// Rend les visuels de design/ et les installe dans l'app :
//   design/icon.mjs → icon.svg, icon-small.svg → design/icon-1024.png → src-tauri/icons/ (toutes les tailles)
//   design/dmg-background.html → src-tauri/dmg/background.png (2x, 144 dpi : net sur un écran Retina)
//   design/nsis-sidebar.html, nsis-header.html → src-tauri/nsis/*.bmp (images de l'installateur Windows)
// Rendu par un Chromium sans fenêtre : celui de Playwright s'il est installé, sinon Google Chrome.
//   node scripts/design.mjs        tout
//   node scripts/design.mjs nsis   seulement les images de l'installateur Windows
import { execFileSync, spawn } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
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

/** PNG → BMP 24 bits, sans couche alpha : le seul format que l'installateur NSIS affiche à coup sûr. */
function bmp24(png, out) {
  const raw = join(tmp, "raw.bmp");
  sips("-s", "format", "bmp", png, "--out", raw);
  const b = readFileSync(raw);
  const offset = b.readUInt32LE(10);
  const w = b.readInt32LE(18);
  const h = b.readInt32LE(22);
  const bpp = b.readUInt16LE(28);
  if (bpp !== 24 && bpp !== 32) throw new Error(`BMP de ${bpp} bits inattendu`);
  const rows = Math.abs(h);
  const from = Math.ceil((w * bpp) / 32) * 4;
  const to = Math.ceil((w * 24) / 32) * 4;
  const px = Buffer.alloc(to * rows);
  for (let y = 0; y < rows; y++) {
    // BMP ordinaire : de bas en haut ; hauteur négative : de haut en bas
    const src = offset + (h < 0 ? rows - 1 - y : y) * from;
    for (let x = 0; x < w; x++) {
      const i = src + (x * bpp) / 8;
      const a = bpp === 32 ? b[i + 3] / 255 : 1;
      // composée sur blanc, le fond des pages de l'installateur
      for (let c = 0; c < 3; c++) px[y * to + x * 3 + c] = Math.round(b[i + c] * a + 255 * (1 - a));
    }
  }
  const head = Buffer.alloc(54);
  head.write("BM", 0);
  head.writeUInt32LE(54 + px.length, 2);
  head.writeUInt32LE(54, 10);
  head.writeUInt32LE(40, 14);
  head.writeInt32LE(w, 18);
  head.writeInt32LE(rows, 22);
  head.writeUInt16LE(1, 26);
  head.writeUInt16LE(24, 28);
  head.writeUInt32LE(px.length, 34);
  head.writeInt32LE(2835, 38);
  head.writeInt32LE(2835, 42);
  writeFileSync(out, Buffer.concat([head, px]));
}

async function installerImages() {
  console.log("› Images de l'installateur Windows");
  const dir = join(root, "src-tauri", "nsis");
  mkdirSync(dir, { recursive: true });
  for (const [name, w, h] of [["sidebar", 164, 314], ["header", 150, 57]]) {
    const png = join(tmp, `${name}.png`);
    await render(join(design, `nsis-${name}.html`), png, w, h);
    bmp24(png, join(dir, `${name}.bmp`));
  }
  console.log("✓ src-tauri/nsis/");
}

if (process.argv[2] === "nsis") {
  await installerImages();
  rmSync(tmp, { recursive: true, force: true });
  process.exit(0);
}

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

// ---------- installateur Windows ----------
await installerImages();

rmSync(tmp, { recursive: true, force: true });
