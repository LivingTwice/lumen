// Lumen sur iPhone et iPad, depuis ce Mac (Xcode installé dans /Applications).
//   node scripts/build-ios.mjs --check        vérifie seulement que tout compile pour iOS
//   node scripts/build-ios.mjs --sim [nom]    lance Lumen dans le simulateur (« iPhone 17 Pro » par défaut),
//                                             interface rechargée à chaud comme `npm run app:dev`
//   node scripts/build-ios.mjs --device       lance Lumen sur l'iPhone branché, en développement
//                                             (l'interface vient de ce Mac, rechargée à chaud)
//   node scripts/build-ios.mjs --install      installe sur l'iPhone branché la version complète,
//                                             autonome (comme « Installer Lumen sur l'iPhone.command »)
//   node scripts/build-ios.mjs --sim-release  la version complète dans le simulateur (essai)
//
// Sans compte développeur payant, un identifiant Apple suffit (Xcode › Réglages › Comptes) :
// l'app installée ainsi s'ouvre pendant 7 jours, puis il suffit de la réinstaller (les
// données restent). APPLE_DEVELOPMENT_TEAM choisit l'équipe s'il y en a plusieurs.
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
import { chmodSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
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

// ---------- version complète sur l'iPhone branché ----------

/** Lit une commande et renvoie sa sortie (vide en cas d'échec). */
const out = (cmd) => {
  const r = spawnSync("sh", ["-c", cmd], { env, encoding: "utf8" });
  return r.status === 0 ? r.stdout : "";
};

/** iPhone et iPad branchés (ou appairés), d'après CoreDevice. */
function devices() {
  const json = join(tmpdir(), `lumen-devices-${process.pid}.json`);
  spawnSync("xcrun", ["devicectl", "list", "devices", "--json-output", json, "--quiet"], { env, stdio: "ignore" });
  try {
    const all = JSON.parse(readFileSync(json, "utf8")).result?.devices ?? [];
    rmSync(json, { force: true });
    return all
      .filter((d) => ["iOS", "iPadOS"].includes(d.hardwareProperties?.platform) && d.hardwareProperties?.reality === "physical")
      .map((d) => ({
        id: d.identifier,
        udid: d.hardwareProperties?.udid,
        name: d.deviceProperties?.name ?? "iPhone",
        paired: d.connectionProperties?.pairingState === "paired",
        wired: d.connectionProperties?.transportType === "wired",
        tunnel: d.connectionProperties?.tunnelState,
        developerMode: d.deviceProperties?.developerModeStatus,
        ios: d.deviceProperties?.osVersionNumber ?? "",
      }));
  } catch {
    return [];
  }
}

/** Équipe de signature : l'identifiant Apple ajouté dans Xcode (gratuit ou payant). */
function team() {
  if (process.env.APPLE_DEVELOPMENT_TEAM) return process.env.APPLE_DEVELOPMENT_TEAM;
  // certificat « Apple Development » déjà créé : son équipe (OU)
  const ids = out("security find-identity -v -p codesigning");
  const name = ids.match(/"(Apple Development: [^"]+)"/)?.[1];
  if (name) {
    const subject = out(`security find-certificate -c "${name}" -p | openssl x509 -noout -subject -nameopt multiline`);
    const ou = subject.match(/organizationalUnitName\s*=\s*([A-Z0-9]{10})/)?.[1];
    if (ou) return ou;
  }
  // sinon, l'équipe que Xcode a retenue pour le compte
  const teams = out("defaults read com.apple.dt.Xcode IDEProvisioningTeamByIdentifier 2>/dev/null; defaults read com.apple.dt.Xcode IDEProvisioningTeams 2>/dev/null");
  return teams.match(/teamID\s*=\s*"?([A-Z0-9]{10})"?/)?.[1] ?? null;
}

if (args.includes("--install") || args.includes("--sim-release")) {
  const sim = args.includes("--sim-release");
  let target = null;
  let teamId = null;
  if (!sim) {
    step("iPhone branché");
    const list = devices();
    target = list.find((d) => d.wired) ?? list.find((d) => d.paired) ?? list[0];
    if (!target)
      fail(
        "Aucun iPhone trouvé.\n" +
          "  1. Branchez votre iPhone à ce Mac avec un câble, et déverrouillez-le.\n" +
          "  2. Sur l'iPhone, touchez « Se fier à cet ordinateur » et tapez votre code.\n" +
          "  3. Relancez cette installation.",
      );
    console.log(`  ${target.name} (iOS ${target.ios})`);
    if (!target.paired)
      fail(`${target.name} n'a pas encore autorisé ce Mac : déverrouillez-le, touchez « Se fier à cet ordinateur », puis relancez.`);
    if (target.developerMode === "disabled")
      fail(
        "Le mode développeur de l'iPhone est coupé. Sur l'iPhone : Réglages › Confidentialité et sécurité › Mode développeur, " +
          "activez-le, laissez l'iPhone redémarrer et confirmez « Activer ». Puis relancez.\n" +
          "  (L'option n'apparaît qu'une fois l'iPhone branché à ce Mac avec Xcode ouvert une fois.)",
      );
    step("Signature");
    teamId = team();
    if (!teamId)
      fail(
        "Aucun identifiant Apple dans Xcode.\n" +
          "  1. Ouvrez Xcode › Réglages (⌘,) › Comptes, touchez « + » et ajoutez votre identifiant Apple (gratuit).\n" +
          "  2. Choisissez-le, puis « Gérer les certificats » › « + » › « Apple Development ».\n" +
          "  3. Relancez cette installation.",
      );
    console.log(`  équipe ${teamId}`);
  }

  step(sim ? "Compilation de la version complète (simulateur)" : "Compilation de la version complète (une dizaine de minutes la première fois)");
  const buildEnv = teamId ? { APPLE_DEVELOPMENT_TEAM: teamId } : {};
  const cmd = sim ? "npx tauri ios build --target aarch64-sim" : "npx tauri ios build --export-method debugging";
  const r = spawnSync("sh", ["-c", cmd], { cwd: root, stdio: "inherit", env: { ...env, ...buildEnv } });
  if (r.status !== 0)
    fail(
      "La compilation a échoué (le détail est plus haut).\n" +
        "  Si Xcode parle de « bundle identifier » déjà pris, ou d'appareil non enregistré : ouvrez une fois\n" +
        "  src-tauri/gen/apple/lumen.xcodeproj dans Xcode, onglet Signing & Capabilities, choisissez votre équipe, puis relancez.",
    );

  const buildDir = join(tauriDir, "gen", "apple", "build");
  if (sim) {
    // l'app du simulateur, installée et ouverte dans le simulateur en marche
    const app = out(`find "${buildDir}" -name "Lumen.app" -path "*simulator*" -maxdepth 4 | head -1`).trim() || out(`find "${buildDir}" -name "Lumen.app" -maxdepth 5 | head -1`).trim();
    if (!app) fail("Lumen.app est introuvable après la compilation.");
    spawnSync("xcrun", ["simctl", "install", "booted", app], { env, stdio: "inherit" });
    spawnSync("xcrun", ["simctl", "launch", "booted", "app.lumen.reader"], { env, stdio: "inherit" });
    console.log("\n✓ Lumen (version complète) est ouvert dans le simulateur.");
    process.exit(0);
  }
  const ipa = readdirSync(join(buildDir, "arm64"), { withFileTypes: true }).find((f) => f.name.endsWith(".ipa"));
  if (!ipa) fail("Le paquet de l'app (.ipa) est introuvable après la compilation.");
  step(`Installation sur ${target.name}`);
  const inst = spawnSync("xcrun", ["devicectl", "device", "install", "app", "--device", target.id, join(buildDir, "arm64", ipa.name)], { env, stdio: "inherit" });
  if (inst.status !== 0) fail(`L'installation sur ${target.name} a échoué : gardez l'iPhone déverrouillé et branché, puis relancez.`);
  const launched = spawnSync("xcrun", ["devicectl", "device", "process", "launch", "--device", target.id, "app.lumen.reader"], { env, stdio: "ignore" });
  console.log(`\n✓ Lumen est installé sur ${target.name}.`);
  if (launched.status !== 0)
    console.log(
      "\n  Première fois : l'iPhone refuse d'ouvrir une app d'un « développeur non approuvé ».\n" +
        "  Sur l'iPhone : Réglages › Général › VPN et gestion de l'appareil › votre identifiant Apple › « Faire confiance ».\n" +
        "  Puis ouvrez Lumen depuis l'écran d'accueil.",
    );
  console.log(
    "\n  Avec un identifiant Apple gratuit, l'app s'ouvre pendant 7 jours. Ensuite, rebranchez l'iPhone et relancez\n" +
      "  cette installation : Lumen se met à jour et garde toutes vos données.",
  );
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
