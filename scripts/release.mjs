// Outils de publication : numéro de version, nouveautés et manifeste de mise à jour.
//   node scripts/release.mjs bump 0.2.0
//   node scripts/release.mjs preview 0.2.0   (notes de la version, tirées de src/changelog.json)
//   node scripts/release.mjs stamp 0.2.0     (la version "next" du journal prend ce numéro et la date du jour)
//   node scripts/release.mjs manifest 0.2.0 "Notes" aarch64 Proprio/depot
//     (avec la version Windows si scripts/build-windows.mjs l'a compilée et signée)
import { readFileSync, writeFileSync, mkdirSync, copyFileSync, existsSync } from "node:fs";
import { join } from "node:path";

const CHANGELOG = "src/changelog.json";

/** La version du journal des nouveautés : celle qui porte ce numéro, sinon "next". */
function release(version, { next = true } = {}) {
  if (!existsSync(CHANGELOG)) return null;
  const { releases } = JSON.parse(readFileSync(CHANGELOG, "utf8"));
  const r = releases.find((x) => x.version === version) ?? (next ? releases.find((x) => x.version === "next") : null);
  return r && r.items?.length ? r : null;
}

/** Notes de publication (GitHub) : en français, puis en anglais. */
function markdown(r) {
  const part = (lang, sep) => [`**${r.title[lang]}**`, "", ...r.items.map((it) => `- **${it.title[lang]}**${sep}${it.body[lang]}`)].join("\n");
  return `${part("fr", " : ")}\n\n---\n\n${part("en", ": ")}`;
}

const [cmd, version, ...rest] = process.argv.slice(2);
if (!/^\d+\.\d+\.\d+$/.test(version ?? "")) {
  console.error("Version invalide (attendu : 1.2.3)");
  process.exit(1);
}

if (cmd === "bump") {
  const pkg = JSON.parse(readFileSync("package.json", "utf8"));
  pkg.version = version;
  writeFileSync("package.json", JSON.stringify(pkg, null, 2) + "\n");
  const conf = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
  conf.version = version;
  writeFileSync("src-tauri/tauri.conf.json", JSON.stringify(conf, null, 2) + "\n");
  const cargo = readFileSync("src-tauri/Cargo.toml", "utf8").replace(
    /(\[package\]\s*\nname = "lumen"\s*\nversion = )"[^"]+"/,
    `$1"${version}"`,
  );
  writeFileSync("src-tauri/Cargo.toml", cargo);
  console.log(`  ok : ${version}`);
} else if (cmd === "preview") {
  const r = release(version);
  if (r) console.log(markdown(r));
} else if (cmd === "stamp") {
  // dans le texte même du journal, pour ne pas en changer la mise en forme
  if (existsSync(CHANGELOG)) {
    const text = readFileSync(CHANGELOG, "utf8");
    const d = new Date();
    const day = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
    const next = text.replace(/"version":\s*"next",(\s*)"date":\s*"[^"]*"/, `"version": "${version}",$1"date": "${day}"`);
    if (next !== text) {
      writeFileSync(CHANGELOG, next);
      console.log(`  ok : nouveautés datées (${version}, ${day})`);
    }
  }
} else if (cmd === "manifest") {
  const [plain, arch, repo] = rest;
  // l'app affiche les titres des nouveautés dans sa carte de mise à jour, dans sa langue
  // Le manifeste est le même pour Mac et Windows : seulement les nouveautés des deux
  // systèmes (une nouveauté écrite pour chacun, `only`, y figure une fois).
  const r = release(version, { next: false });
  const shared = r ? r.items.filter((it) => !it.only || r.items.some((o) => o.only && o.only !== it.only && o.title.fr === it.title.fr)) : [];
  const titles = (lang) => [...new Set(shared.map((it) => it.title[lang]))];
  const notes = r ? JSON.stringify({ fr: titles("fr"), en: titles("en") }) : plain;
  const bundle = "src-tauri/target/release/bundle";
  const tar = join(bundle, "macos", "Lumen.app.tar.gz");
  const sig = tar + ".sig";
  if (!existsSync(tar) || !existsSync(sig)) {
    console.error("Paquet de mise à jour introuvable (Lumen.app.tar.gz et .sig).");
    process.exit(1);
  }
  const outDir = join(bundle, "release");
  mkdirSync(outDir, { recursive: true });
  const name = `Lumen_${version}_${arch}.app.tar.gz`;
  copyFileSync(tar, join(outDir, name));
  const manifest = {
    version,
    notes,
    pub_date: new Date().toISOString(),
    platforms: {
      [`darwin-${arch}`]: {
        signature: readFileSync(sig, "utf8").trim(),
        url: `https://github.com/${repo}/releases/download/v${version}/${name}`,
      },
    },
  };
  console.log(`  ok : ${name}`);
  // la version Windows (scripts/build-windows.mjs), si elle est compilée et signée pour la mise à jour
  const winName = `Lumen_${version}_x64-setup.exe`;
  const winExe = join("src-tauri/target/windows/x86_64-pc-windows-msvc/release/bundle/nsis", winName);
  if (existsSync(winExe) && existsSync(winExe + ".sig")) {
    copyFileSync(winExe, join(outDir, winName));
    manifest.platforms["windows-x86_64"] = {
      signature: readFileSync(winExe + ".sig", "utf8").trim(),
      url: `https://github.com/${repo}/releases/download/v${version}/${winName}`,
    };
    console.log(`  ok : ${winName}`);
  }
  writeFileSync(join(outDir, "latest.json"), JSON.stringify(manifest, null, 2) + "\n");
} else {
  console.error("Commande inconnue");
  process.exit(1);
}
