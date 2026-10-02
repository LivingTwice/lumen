// Outils de publication : numéro de version et manifeste de mise à jour.
//   node scripts/release.mjs bump 0.2.0
//   node scripts/release.mjs manifest 0.2.0 "Notes" aarch64 Proprio/depot
import { readFileSync, writeFileSync, mkdirSync, copyFileSync, existsSync } from "node:fs";
import { join } from "node:path";

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
} else if (cmd === "manifest") {
  const [notes, arch, repo] = rest;
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
  writeFileSync(join(outDir, "latest.json"), JSON.stringify(manifest, null, 2) + "\n");
  console.log(`  ok : ${name}`);
} else {
  console.error("Commande inconnue");
  process.exit(1);
}
