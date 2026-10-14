// Lumen sur iPhone et iPad, depuis ce Mac (Xcode installé dans /Applications).
//   node scripts/build-ios.mjs --check        vérifie seulement que tout compile pour iOS
//   node scripts/build-ios.mjs --sim [nom]    lance Lumen dans le simulateur (« iPhone 17 Pro » par défaut),
//                                             interface rechargée à chaud comme `npm run app:dev`
//   node scripts/build-ios.mjs --device       lance Lumen sur l'iPhone branché (identifiant Apple
//                                             dans Xcode › Réglages › Comptes, voir APPLE_DEVELOPMENT_TEAM)
//
// Outils, installés au besoin la première fois : les cibles iOS de rustup (le Rust de
// Homebrew ne compile que pour le Mac), xcodegen (le projet Xcode) et CocoaPods, par
// Homebrew. Le projet Xcode est dans src-tauri/gen/apple (versionné) ; `tauri ios init`
// ne se relance pas : il écraserait ses réglages (Accelerate, icônes).
//
// Xcode doit être l'outil de développement actif. Sinon (`xcode-select -p` donne
// /Library/Developer/CommandLineTools), ce script passe par de petits relais qui le
// désignent (Tauri lance xcodebuild dans un environnement nettoyé) ; pour régler cela
// une fois pour toutes : sudo xcode-select -s /Applications/Xcode.app
import { execSync, spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const tauriDir = join(root, "src-tauri");
const cargoBin = join(homedir(), ".cargo", "bin");
const xcode = "/Applications/Xcode.app/Contents/Developer";
const brew = ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"].find((p) => existsSync(p));
const brewBin = brew ? dirname(brew) : "/opt/homebrew/bin";

const step = (s) => console.log(`\n› ${s}`);
const fail = (s) => {
  console.error(`\n✗ ${s}`);
  process.exit(1);
};

if (!existsSync(xcode)) fail("Xcode n'est pas installé (App Store), ou pas dans /Applications.");

// relais vers Xcode si les outils en ligne de commande sont l'outil actif
const active = spawnSync("xcode-select", ["-p"]).stdout?.toString().trim() ?? "";
const relays = join(tmpdir(), "lumen-xcode-relais");
if (!active.startsWith("/Applications/Xcode")) {
  mkdirSync(relays, { recursive: true });
  for (const tool of ["xcodebuild", "xcrun"]) {
    const p = join(relays, tool);
    writeFileSync(p, `#!/bin/sh\nexport DEVELOPER_DIR=${xcode}\nexec /usr/bin/${tool} "$@"\n`);
    chmodSync(p, 0o755);
  }
}

const env = {
  ...process.env,
  DEVELOPER_DIR: xcode,
  PATH: [relays, cargoBin, brewBin, process.env.PATH].join(":"),
};
const run = (cmd, cwd = root) => execSync(cmd, { cwd, stdio: "inherit", env });
const has = (cmd) => spawnSync("sh", ["-c", `command -v ${cmd}`], { env, stdio: "ignore" }).status === 0;

step("Outils pour iOS");
if (!existsSync(join(cargoBin, "rustup"))) {
  console.log("  Installation de rustup (pour compiler aussi pour iOS)…");
  run("curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | RUSTUP_INIT_SKIP_PATH_CHECK=yes sh -s -- -y --profile minimal --no-modify-path");
}
run("rustup target add aarch64-apple-ios aarch64-apple-ios-sim >/dev/null 2>&1 || rustup target add aarch64-apple-ios aarch64-apple-ios-sim");
// CocoaPods par Homebrew (Tauri le chercherait avec gem et sudo)
for (const tool of ["xcodegen", "pod"]) {
  if (!has(tool)) {
    if (!brew) fail(`${tool} manque, et Homebrew aussi.`);
    run(`"${brew}" install ${tool === "pod" ? "cocoapods" : tool}`);
  }
}

const args = process.argv.slice(2);
if (args.includes("--check")) {
  step("Vérification : tout compile pour iPhone et iPad (simulateur)");
  run("cargo check -p lumen --lib --target aarch64-apple-ios-sim --target-dir target/ios", tauriDir);
  console.log("\n✓ Lumen compile pour iOS.");
  process.exit(0);
}

if (args.includes("--device")) {
  if (!process.env.APPLE_DEVELOPMENT_TEAM)
    console.log(
      "  Astuce : APPLE_DEVELOPMENT_TEAM=<identifiant d'équipe> (Xcode › Réglages › Comptes) évite de le choisir dans Xcode à chaque fois.",
    );
  step("Lumen sur l'iPhone branché");
  run("npx tauri ios dev --host", root);
} else {
  const device = args.find((a) => !a.startsWith("--")) ?? "iPhone 17 Pro";
  step(`Lumen dans le simulateur (${device})`);
  spawnSync("xcrun", ["simctl", "boot", device], { env, stdio: "ignore" });
  spawnSync("open", ["-a", "Simulator"], { env, stdio: "ignore" });
  run(`npx tauri ios dev "${device}"`, root);
}
